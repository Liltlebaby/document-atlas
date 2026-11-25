use super::*;
use axum::{body::Body, extract::{DefaultBodyLimit, Path as RoutePath, State}, http::{header, HeaderMap, HeaderValue, StatusCode}, middleware::{self, Next}, response::{IntoResponse, Response}, routing::{get, post}, Extension, Json, Router};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use tokio::{io::AsyncWriteExt, sync::Semaphore};
use tower_http::services::ServeDir;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Account {
    username: String,
    password_sha256: String,
    role: Role,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Role { Viewer, Reviewer }

#[derive(Clone)]
struct User { username: String, role: Role }

struct Site {
    atlas: AppState,
    accounts: Vec<Account>,
    workspace: String,
    runtime: PathBuf,
    chat_enabled: bool,
    render_slots: Semaphore,
    search_slots: Semaphore,
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn same_digest(left: &str, right: &str) -> bool {
    left.len() == right.len() && left.bytes().zip(right.bytes()).fold(0, |difference, (a, b)| difference | (a ^ b)) == 0
}

fn identify(headers: &HeaderMap, accounts: &[Account]) -> Option<User> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") || encoded.len() > 1024 { return None; }
    let decoded = String::from_utf8(STANDARD.decode(encoded).ok()?).ok()?;
    let (username, password) = decoded.split_once(':')?;
    let digest = hex::encode(Sha256::digest(password.as_bytes()));
    accounts.iter().find(|account| account.username == username && same_digest(&digest, &account.password_sha256))
        .map(|account| User { username: account.username.clone(), role: account.role })
}

async fn access(State(site): State<Arc<Site>>, mut request: axum::extract::Request, next: Next) -> Response {
    let mut response = if request.uri().path() == "/healthz" {
        next.run(request).await
    } else if let Some(user) = identify(request.headers(), &site.accounts) {
        request.extensions_mut().insert(user);
        next.run(request).await
    } else {
        let mut response = (StatusCode::UNAUTHORIZED, "Document Atlas requires a company account").into_response();
        response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"Document Atlas\", charset=\"UTF-8\""));
        response
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self' data:; img-src 'self' blob: data:; connect-src 'self'; frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'self'"));
    response
}

fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"error": message.into()}))).into_response()
}

fn arg<'a>(body: &'a Value, key: &str, maximum: usize) -> Result<&'a str> {
    body.get(key).and_then(Value::as_str).filter(|value| value.chars().count() <= maximum)
        .ok_or_else(|| format!("ข้อมูล {key} ไม่ถูกต้อง"))
}

fn number(body: &Value, key: &str) -> Result<i64> {
    body.get(key).and_then(Value::as_i64).ok_or_else(|| format!("ข้อมูล {key} ไม่ถูกต้อง"))
}

fn request_id(user: &User, body: &Value) -> Result<String> {
    let id = Uuid::parse_str(arg(body, "requestId", 36)?).map_err(|_| "รหัสคำถามไม่ถูกต้อง".to_string())?;
    Ok(hex::encode(Sha256::digest(format!("{}:{id}", user.username))))
}

fn json_response<T: Serialize>(value: T) -> Result<Response> {
    Ok(Json(serde_json::to_value(value).map_err(err)?).into_response())
}

fn query<T>(atlas: &AppState, operation: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db = atlas.db.lock().map_err(err)?;
    operation(&db)
}

fn check_document(db: &Connection, id: &str) -> Result<Node> {
    let item = node(db, id)?;
    if !item.active || item.kind != "file" { return Err("ไม่พบเอกสารในดัชนีปัจจุบัน".into()); }
    Ok(item)
}

async fn render_page(site: &Site, node_id: &str, page: i64) -> Result<Response> {
    let _permit = site.render_slots.try_acquire().map_err(|_| "ตัวอ่านกำลังทำงาน กรุณาลองอีกครั้ง".to_string())?;
    let (expected, pages) = query(&site.atlas, |db| {
        check_document(db, node_id)?;
        db.query_row("SELECT sha256,page_count FROM rag_documents WHERE node_id=?1", [node_id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?)))
            .map_err(|_| "เอกสารนี้ยังไม่มีข้อมูลหน้า PDF".into())
    })?;
    if page < 1 || page > pages || !valid_digest(&expected) { return Err("เลขหน้าหรือข้อมูล PDF ไม่ถูกต้อง".into()); }
    let path = rag::resolve_rag_document(&site.atlas, node_id).await.map_err(|_| "PDF ฉบับที่ทำดัชนีไม่อยู่ในชุดนี้ กรุณาติดต่อ IT".to_string())?;
    let metadata = tokio::fs::metadata(&path).await.map_err(err)?;
    if metadata.len() > 100 * 1024 * 1024 { return Err("PDF ใหญ่เกิน 100 MB".into()); }
    let bytes = tokio::fs::read(&path).await.map_err(err)?;
    if !bytes.starts_with(b"%PDF-") || hex::encode(Sha256::digest(&bytes)) != expected { return Err("PDF ไม่ตรงกับฉบับที่ทำดัชนี".into()); }
    let mut command = tokio::process::Command::new(chat::node_binary()?);
    command.arg(site.runtime.join("render-page.mjs")).arg(page.to_string()).kill_on_drop(true)
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    command.env_remove("NODE_OPTIONS").env_remove("ATLAS_PORTAL_PASSWORD");
    chat::hidden(command.as_std_mut());
    let mut child = command.spawn().map_err(err)?;
    let mut input = child.stdin.take().ok_or("เริ่มตัวอ่าน PDF ไม่สำเร็จ")?;
    let output = tokio::time::timeout(Duration::from_secs(45), async move {
        input.write_all(&bytes).await?;
        drop(input);
        child.wait_with_output().await
    }).await.map_err(|_| "ตัวอ่าน PDF ใช้เวลานานเกินกำหนด".to_string())?.map_err(err)?;
    if !output.status.success() || !output.stdout.starts_with(b"\x89PNG\r\n\x1a\n") || output.stdout.len() > 16 * 1024 * 1024 {
        eprintln!("PDF render failed ({}; {} stdout bytes): {}", output.status, output.stdout.len(), String::from_utf8_lossy(&output.stderr).chars().take(600).collect::<String>());
        return Err("แสดงหน้า PDF ไม่สำเร็จ".into());
    }
    Ok(([(header::CONTENT_TYPE, "image/png")], Body::from(output.stdout)).into_response())
}

async fn dispatch(site: &Site, user: &User, command: &str, body: Value) -> Result<Response> {
    let atlas = &site.atlas;
    match command {
        "session" => json_response(json!({"username":user.username,"role":user.role,"workspaceId":site.workspace})),
        "search_nodes" => {
            let text = arg(&body,"query",2000)?.to_string();
            json_response(query(atlas, |db| search_nodes(db,text,Some("file".into())))?)
        }
        "get_rag_document_detail" => json_response(query(atlas, |db| {
            let id = arg(&body,"nodeId",80)?;
            check_document(db,id)?;
            rag::document_detail(db,id)
        })?),
        "get_rag_document_page" => json_response(query(atlas, |db| {
            let id = arg(&body,"nodeId",80)?;
            check_document(db,id)?;
            rag::document_page(db,id,number(&body,"page")?)
        })?),
        "render_pdf_page" => render_page(site,arg(&body,"nodeId",80)?,number(&body,"page")?).await,
        "rag_search" => {
            let _permit = site.search_slots.try_acquire().map_err(|_| "กำลังค้นข้อมูล กรุณาลองอีกครั้ง".to_string())?;
            json_response(rag::search_hits(atlas,arg(&body,"query",2000)?).await?)
        }
        "get_iso_chat_status" => {
            let mut status = serde_json::to_value(chat::get_iso_chat_status(atlas)?).map_err(err)?;
            if !site.chat_enabled {
                status["ready"] = json!(false);
                status["message"] = json!("IT ยังไม่ได้เปิดถามตอบด้วย GPT-6 Luna");
            }
            json_response(status)
        }
        "ask_iso_chat" if site.chat_enabled => {
            let id = request_id(user,&body)?;
            let answer = chat::ask_iso_chat(atlas,arg(&body,"question",2000)?.into(),site.workspace.clone(),id).await?;
            json_response(answer)
        }
        "cancel_iso_chat" => json_response(chat::cancel_iso_chat(atlas,request_id(user,&body)?)?),
        "iso_overview" => json_response(query(atlas,iso::overview)?),
        "iso_documents" => json_response(query(atlas, |db| iso::documents(db,arg(&body,"department",100)?))?),
        "iso_similar" => json_response(query(atlas, |db| {
            let id = arg(&body,"nodeId",80)?;
            check_document(db,id)?;
            let thresholds = iso::config(db)?.map(|config| config.similarity).unwrap_or_default();
            similar::similar_for(db,id,&thresholds)
        })?),
        "iso_decide" if user.role == Role::Reviewer => json_response(query(atlas, |db| {
            let id = arg(&body,"nodeId",80)?;
            check_document(db,id)?;
            iso::decide(db,id,arg(&body,"decision",12)?,arg(&body,"note",2000)?,&user.username)
        })?),
        "iso_export" if user.role == Role::Reviewer => {
            let csv = query(atlas,iso::export_csv)?;
            Ok(([(header::CONTENT_TYPE,"text/csv; charset=utf-8"),(header::CONTENT_DISPOSITION,"attachment; filename=iso-review-decisions.csv")],csv).into_response())
        }
        "iso_decide" | "iso_export" | "ask_iso_chat" => Ok(api_error(StatusCode::FORBIDDEN,"บัญชีนี้ไม่มีสิทธิ์ หรือฟังก์ชันยังไม่เปิดใช้งาน")),
        _ => Ok(api_error(StatusCode::NOT_FOUND,"ไม่พบคำสั่ง")),
    }
}

async fn api(State(site): State<Arc<Site>>, Extension(user): Extension<User>, RoutePath(command): RoutePath<String>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if headers.get("x-atlas-request").and_then(|value|value.to_str().ok()) != Some("1") {
        return api_error(StatusCode::FORBIDDEN,"คำขอไม่ถูกต้อง");
    }
    match dispatch(&site,&user,&command,body).await {
        Ok(response) => response,
        Err(message) => api_error(StatusCode::BAD_REQUEST,message),
    }
}

pub async fn run() -> Result<()> {
    let root = match std::env::var_os("ATLAS_ROOT") {
        Some(root) => PathBuf::from(root),
        None => std::env::current_exe().map_err(err)?.parent().ok_or("Missing program directory")?.to_path_buf(),
    };
    if !root.is_absolute() { return Err("ATLAS_ROOT must be an absolute path".into()); }
    let data = root.join("data");
    let runtime = root.join("qa-runtime");
    let public = root.join("public");
    if !public.join("index.html").is_file() || !runtime.join("render-page.mjs").is_file() { return Err("Missing public/ or qa-runtime/".into()); }
    let accounts: Vec<Account> = serde_json::from_slice(&fs::read(root.join("accounts.json")).map_err(|_| "Run New-AtlasAccount.ps1 before starting".to_string())?).map_err(err)?;
    let mut names = HashSet::new();
    if accounts.is_empty() || accounts.iter().any(|account| !valid_digest(&account.password_sha256) || account.username.is_empty() || account.username.len() > 80 || account.username.contains(':') || !names.insert(&account.username)) {
        return Err("Invalid account configuration".into());
    }
    unsafe { rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()))); }
    let registry = Connection::open_with_flags(data.join("workspaces.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    let workspace = active_workspace_id(&registry)?;
    let file = workspace_db_file(&registry,&workspace)?;
    let db = Connection::open_with_flags(data.join(file), OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(err)?;
    if setting(&db,"index_schema")? != "2" { return Err("Unsupported snapshot schema".into()); }
    db.busy_timeout(Duration::from_secs(5)).map_err(err)?;
    init_db(&db)?;
    let portal = (std::env::var("ATLAS_PORTAL_URL"),std::env::var("ATLAS_PORTAL_USER"),std::env::var("ATLAS_PORTAL_PASSWORD"));
    if let (Ok(url),Ok(user),Ok(password)) = portal {
        if !url.is_empty() && !user.is_empty() && !password.is_empty() {
            safe_base(&url)?;
            let original = setting(&db,"base_url")?;
            if original != url { return Err("ATLAS_PORTAL_URL must match the snapshot Portal URL".into()); }
            db.execute("INSERT INTO settings(key,value) VALUES('username',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[user]).map_err(err)?;
        }
    }
    std::env::set_current_dir(&data).map_err(err)?;
    std::env::set_var("ATLAS_RUNTIME_DIR", &runtime);
    let state = Arc::new(Site {
        atlas: AppState { web_engine: tokio::sync::OnceCell::new(), db: Mutex::new(db), registry: Mutex::new(registry), data_dir: data,
            chat_cancel: Mutex::new(None), chat_request: Mutex::new(String::new()), scan_pause: Mutex::new(None), download_cancel: Mutex::new(None), rag_pause: Mutex::new(None) },
        accounts, workspace, runtime, chat_enabled: std::env::var("ATLAS_ENABLE_CHAT").as_deref() == Ok("1"),
        render_slots: Semaphore::new(2), search_slots: Semaphore::new(4),
    });
    let router = Router::new().route("/healthz",get(||async { Json(json!({"ok":true})) }))
        .route("/api/{command}",post(api)).fallback_service(ServeDir::new(public))
        .layer(DefaultBodyLimit::max(32 * 1024)).layer(middleware::from_fn_with_state(state.clone(),access)).with_state(state);
    let port = std::env::var("ATLAS_PORT").unwrap_or_else(|_| "8765".into()).parse::<u16>().map_err(err)?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await.map_err(err)?;
    println!("Document Atlas Web is listening at http://{}",listener.local_addr().map_err(err)?);
    axum::serve(listener, router).with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; }).await.map_err(err)
}
