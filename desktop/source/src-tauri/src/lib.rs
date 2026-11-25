use std::{collections::{HashSet, VecDeque}, fs, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use reqwest::{Client, Url};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;
use sha2::{Digest, Sha256};
mod rag;
mod chat;
mod iso;
mod similar;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile { base_url: String, username: String, download_dir: String, has_credential: bool }
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Node { id: String, portal_id: String, kind: String, name: String, path: String, active: bool }
#[derive(Clone, Serialize, Deserialize)]
struct Edge { source: String, target: String }
#[derive(Serialize)]
struct Graph { nodes: Vec<Node>, edges: Vec<Edge> }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphPage { nodes: Vec<Node>, edges: Vec<Edge>, has_more: bool }
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress { kind: String, status: String, current: usize, total: usize, message: String }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanSession { id: String, status: String, current_folder: String, folders_done: i64, folders_pending: i64, files_seen: i64, last_error: String, updated_at: String, elapsed_seconds: i64, retry_attempt: i64, started_at: String }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadPlan { node_id: String, scope: String, items: Vec<Node>, duplicate_count: usize }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Job { id: String, status: String, total: i64, completed: i64, failed: i64, created_at: String }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Workspace { id: String, name: String, active: bool, base_url: String, username: String }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct ScanLog { id:i64, session_id:String, level:String, message:String, created_at:String }

struct AppState { chat_cancel: Mutex<Option<Arc<AtomicBool>>>, chat_request: Mutex<String>, db: Mutex<Connection>, registry: Mutex<Connection>, data_dir: PathBuf, scan_pause: Mutex<Option<Arc<AtomicBool>>>, download_cancel: Mutex<Option<Arc<AtomicBool>>>, rag_pause: Mutex<Option<Arc<AtomicBool>>> }

fn err<E: std::fmt::Display>(e: E) -> String { e.to_string() }
fn credential(base:&str,username: &str) -> Result<keyring::Entry> { keyring::Entry::new("document-atlas", &format!("{}|{}",base,username)).map_err(err) }
fn password_for(base:&str,username:&str)->Result<String>{
    if let Ok(secret)=credential(base,username)?.get_password(){return Ok(secret)}
    keyring::Entry::new("document-atlas",username).map_err(err)?.get_password().map_err(err)
}
fn safe_base(input: &str) -> Result<Url> {
    let url = Url::parse(input.trim()).map_err(err)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || url.username() != "" || url.password().is_some() { return Err("Portal URL ต้องเป็น http/https และไม่ใส่ข้อมูลล็อกอินใน URL".into()); }
    let mut base = url;
    base.set_path("/"); base.set_query(None); base.set_fragment(None);
    Ok(base)
}
fn validate_id(id: &str) -> Result<()> { if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) { Err("ID ต้องเป็นตัวเลข".into()) } else { Ok(()) } }
fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
    CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS nodes (id TEXT PRIMARY KEY, portal_id TEXT NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL, path TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1, last_seen TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
    CREATE TABLE IF NOT EXISTS edges (source TEXT NOT NULL, target TEXT NOT NULL, PRIMARY KEY(source,target));
    CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, status TEXT NOT NULL, total INTEGER NOT NULL, completed INTEGER NOT NULL DEFAULT 0, failed INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
    CREATE TABLE IF NOT EXISTS job_items (job_id TEXT NOT NULL, node_id TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending', file_path TEXT, error TEXT, sha256 TEXT, PRIMARY KEY(job_id,node_id));
    CREATE TABLE IF NOT EXISTS scan_sessions (id TEXT PRIMARY KEY, source_key TEXT NOT NULL, status TEXT NOT NULL, current_folder TEXT NOT NULL DEFAULT '', folders_done INTEGER NOT NULL DEFAULT 0, files_seen INTEGER NOT NULL DEFAULT 0, last_error TEXT NOT NULL DEFAULT '', elapsed_seconds INTEGER NOT NULL DEFAULT 0, retry_attempt INTEGER NOT NULL DEFAULT 0, started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
    CREATE TABLE IF NOT EXISTS scan_queue (ordinal INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, folder_id TEXT NOT NULL, name TEXT NOT NULL, parent_path TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending', UNIQUE(session_id,folder_id));
    CREATE TABLE IF NOT EXISTS scan_files (session_id TEXT NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(session_id,node_id));
    CREATE TABLE IF NOT EXISTS scan_logs (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, level TEXT NOT NULL, message TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
    CREATE INDEX IF NOT EXISTS idx_scan_logs_session ON scan_logs(session_id,id);
    CREATE INDEX IF NOT EXISTS idx_nodes_search ON nodes(portal_id,kind,active);
    CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target);
    CREATE INDEX IF NOT EXISTS idx_scan_queue ON scan_queue(session_id,status,ordinal);").map_err(err)?;
    for (name,definition) in [("elapsed_seconds","INTEGER NOT NULL DEFAULT 0"),("retry_attempt","INTEGER NOT NULL DEFAULT 0")] {
        let mut stmt=conn.prepare("PRAGMA table_info(scan_sessions)").map_err(err)?;
        let columns=stmt.query_map([],|r|r.get::<_,String>(1)).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
        if !columns.iter().any(|c|c==name){conn.execute(&format!("ALTER TABLE scan_sessions ADD COLUMN {name} {definition}"),[]).map_err(err)?;}
    }
    if setting(conn,"index_schema")?!="2" {
        conn.execute_batch("DELETE FROM edges; DELETE FROM nodes; INSERT INTO settings(key,value) VALUES('index_schema','2') ON CONFLICT(key) DO UPDATE SET value='2';").map_err(err)?;
    }
    rag::init_rag_db(conn)?;
    iso::init_iso_db(conn)?;
    similar::init_similar_db(conn)?;
    Ok(())
}
fn init_registry(conn:&Connection)->Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS workspaces (id TEXT PRIMARY KEY,name TEXT NOT NULL,db_file TEXT NOT NULL UNIQUE,created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS registry_settings (key TEXT PRIMARY KEY,value TEXT NOT NULL);").map_err(err)?;
    conn.execute("INSERT OR IGNORE INTO workspaces(id,name,db_file) VALUES('default','พื้นที่หลัก','atlas.sqlite')",[]).map_err(err)?;
    conn.execute("INSERT OR IGNORE INTO registry_settings(key,value) VALUES('active_workspace','default')",[]).map_err(err)?;
    Ok(())
}
fn recover_interrupted(db:&Connection)->Result<()> {
    db.execute("UPDATE jobs SET status='interrupted' WHERE status='running'",[]).map_err(err)?;
    db.execute("UPDATE scan_sessions SET status='paused',retry_attempt=0 WHERE status IN ('running','pausing')",[]).map_err(err)?;
    db.execute("UPDATE rag_jobs SET status='paused' WHERE status='running'",[]).map_err(err)?;
    Ok(())
}
fn active_workspace_id(registry:&Connection)->Result<String>{registry.query_row("SELECT value FROM registry_settings WHERE key='active_workspace'",[],|r|r.get(0)).map_err(err)}
fn workspace_list(registry:&Connection,data_dir:&Path)->Result<Vec<Workspace>>{
    let active=active_workspace_id(registry)?;
    let mut stmt=registry.prepare("SELECT id,name,db_file FROM workspaces ORDER BY created_at,id").map_err(err)?;
    let entries=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    entries.into_iter().map(|(id,name,file)|{let db=Connection::open_with_flags(data_dir.join(file),OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;Ok(Workspace{id:id.clone(),name,active:id==active,base_url:setting(&db,"base_url")?,username:setting(&db,"username")?})}).collect()
}
#[tauri::command]
fn list_workspaces(state:State<'_,AppState>)->Result<Vec<Workspace>>{let registry=state.registry.lock().map_err(err)?;workspace_list(&registry,&state.data_dir)}
#[tauri::command]
fn create_workspace(state:State<'_,AppState>,name:String)->Result<Vec<Workspace>>{
    let name=name.trim();if name.is_empty() || name.chars().count()>80{return Err("กรุณาตั้งชื่อพื้นที่สแกนไม่เกิน 80 ตัวอักษร".into())}
    let id=Uuid::new_v4().to_string();let file=format!("atlas-{id}.sqlite");
    let db=Connection::open(state.data_dir.join(&file)).map_err(err)?;init_db(&db)?;
    let registry=state.registry.lock().map_err(err)?;
    registry.execute("INSERT INTO workspaces(id,name,db_file) VALUES(?1,?2,?3)",params![id,name,file]).map_err(err)?;
    workspace_list(&registry,&state.data_dir)
}
#[tauri::command]
fn switch_workspace(state:State<'_,AppState>,workspace_id:String)->Result<Profile>{
    let scan=state.scan_pause.lock().map_err(err)?;if scan.is_some(){return Err("กรุณาหยุดการสแกนก่อนสลับพื้นที่".into())}
    let download=state.download_cancel.lock().map_err(err)?;if download.is_some(){return Err("กรุณารอให้งานดาวน์โหลดจบก่อนสลับพื้นที่".into())}
    let rag=state.rag_pause.lock().map_err(err)?;if rag.is_some(){return Err("กรุณาหยุดงานทำดัชนี RAG ก่อนสลับพื้นที่".into())}
    let registry=state.registry.lock().map_err(err)?;
    let file=workspace_db_file(&registry,&workspace_id)?;
    let next=Connection::open(state.data_dir.join(file)).map_err(err)?;init_db(&next)?;
    let p=profile(&next)?;
    let mut current=state.db.lock().map_err(err)?;
    registry.execute("UPDATE registry_settings SET value=?1 WHERE key='active_workspace'",[&workspace_id]).map_err(err)?;
    *current=next;
    Ok(p)
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct WorkspaceDeletePreview { workspace_id:String, name:String, is_default:bool, nodes:i64, scan_sessions:i64, rag_documents:i64, download_files:Vec<String>, cache_files:i64 }

fn workspace_db_file(registry:&Connection,id:&str)->Result<String>{
    let file:String=registry.query_row("SELECT db_file FROM workspaces WHERE id=?1",[id],|r|r.get(0)).map_err(|_|"ไม่พบพื้นที่สแกน".to_string())?;
    let expected=if id=="default"{"atlas.sqlite".to_string()}else{Uuid::parse_str(id).map_err(|_|"รหัสพื้นที่สแกนไม่ถูกต้อง")?;format!("atlas-{id}.sqlite")};
    if file!=expected{return Err("ชื่อฐานข้อมูลของพื้นที่สแกนไม่ถูกต้อง".into())}
    Ok(file)
}
fn verified_downloads(db:&Connection,registry:&Connection,data_dir:&Path,workspace_id:&str)->Result<Vec<String>>{
    let mut stmt=db.prepare("SELECT j.file_path,j.sha256,n.name,n.portal_id FROM job_items j JOIN nodes n ON n.id=j.node_id WHERE j.status='completed' AND j.file_path IS NOT NULL AND j.sha256 IS NOT NULL ORDER BY j.rowid DESC").map_err(err)?;
    let entries=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).map_err(err)?;
    let mut candidates=HashSet::new();
    for entry in entries {
        let (path,sha,_name,portal_id)=entry.map_err(err)?;
        let source=Path::new(&path);
        let suffix=format!(" [{portal_id}].pdf");
        if !source.file_name().and_then(|v|v.to_str()).is_some_and(|file|file.len()>suffix.len() && file.ends_with(&suffix)) || sha.len()!=64{continue;}
        let Ok(meta)=fs::symlink_metadata(source) else{continue};
        if !meta.file_type().is_file() || meta.len()>100*1024*1024{continue;}
        let Ok(bytes)=fs::read(source) else{continue};
        if !bytes.starts_with(b"%PDF-") || hex::encode(Sha256::digest(&bytes))!=sha{continue;}
        candidates.insert(path);
    }
    let mut other_files=HashSet::new();
    let mut workspaces=registry.prepare("SELECT id,db_file FROM workspaces WHERE id<>?1").map_err(err)?;
    let others=workspaces.query_map([workspace_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(err)?;
    for row in others {
        let (id,file)=row.map_err(err)?;
        let expected=if id=="default"{"atlas.sqlite".to_string()}else{format!("atlas-{id}.sqlite")};
        if file!=expected{return Err("ชื่อฐานข้อมูลของพื้นที่สแกนอื่นไม่ถูกต้อง".into())}
        let other=Connection::open_with_flags(data_dir.join(file),OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
        let mut paths=other.prepare("SELECT DISTINCT file_path FROM job_items WHERE file_path IS NOT NULL").map_err(err)?;
        for path in paths.query_map([],|r|r.get::<_,String>(0)).map_err(err)?{other_files.insert(path.map_err(err)?);}
    }
    let mut result:Vec<_>=candidates.difference(&other_files).cloned().collect();result.sort();Ok(result)
}
fn cache_files(data_dir:&Path,workspace_id:&str)->Result<Vec<PathBuf>>{
    let dir=data_dir.join("rag-pdf-cache").join(workspace_id);
    if !dir.exists(){return Ok(Vec::new())}
    if !fs::symlink_metadata(&dir).map_err(err)?.file_type().is_dir(){return Err("โฟลเดอร์แคช PDF ไม่ถูกต้อง".into())}
    let mut files=Vec::new();
    for entry in fs::read_dir(dir).map_err(err)? {
        let path=entry.map_err(err)?.path();
        let name=path.file_name().and_then(|value|value.to_str()).unwrap_or("");
        if name.starts_with("document-") && (name.ends_with(".pdf") || (name.contains(".pdf.") && name.ends_with(".part"))) && fs::symlink_metadata(&path).map_err(err)?.file_type().is_file(){files.push(path)}
    }
    Ok(files)
}
fn workspace_delete_preview(registry:&Connection,data_dir:&Path,id:&str)->Result<WorkspaceDeletePreview>{
    let file=workspace_db_file(registry,id)?;
    let name:String=registry.query_row("SELECT name FROM workspaces WHERE id=?1",[id],|r|r.get(0)).map_err(err)?;
    let db=Connection::open_with_flags(data_dir.join(file),OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    let count=|table:&str|->Result<i64>{db.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).map_err(err)};
    Ok(WorkspaceDeletePreview{workspace_id:id.into(),name,is_default:id=="default",nodes:count("nodes")?,scan_sessions:count("scan_sessions")?,rag_documents:count("rag_documents")?,download_files:verified_downloads(&db,registry,data_dir,id)?,cache_files:cache_files(data_dir,id)?.len() as i64})
}
#[tauri::command]
fn preview_workspace_delete(state:State<'_,AppState>,workspace_id:String)->Result<WorkspaceDeletePreview>{
    let registry=state.registry.lock().map_err(err)?;
    workspace_delete_preview(&registry,&state.data_dir,&workspace_id)
}
fn clear_workspace_data(db:&mut Connection)->Result<()>{
    let tx=db.transaction().map_err(err)?;
    if tx.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='rag_vec'",[],|r|r.get::<_,i64>(0)).map_err(err)?>0{tx.execute("DELETE FROM rag_vec",[]).map_err(err)?;}
    tx.execute_batch("DELETE FROM iso_similar; DELETE FROM iso_reviews; DELETE FROM rag_chunks; DELETE FROM rag_documents; DELETE FROM rag_queue; DELETE FROM rag_jobs; DELETE FROM job_items; DELETE FROM jobs; DELETE FROM scan_logs; DELETE FROM scan_files; DELETE FROM scan_queue; DELETE FROM scan_sessions; DELETE FROM edges; DELETE FROM nodes;").map_err(err)?;
    tx.commit().map_err(err)
}
#[tauri::command]
fn delete_workspace(state:State<'_,AppState>,workspace_id:String,mode:String)->Result<Vec<Workspace>>{
    if !matches!(mode.as_str(),"scan_only"|"all_files"){return Err("รูปแบบการลบไม่ถูกต้อง".into())}
    if state.scan_pause.lock().map_err(err)?.is_some() || state.download_cancel.lock().map_err(err)?.is_some() || state.rag_pause.lock().map_err(err)?.is_some(){return Err("กรุณาหยุดงานสแกน ดาวน์โหลด และทำดัชนีก่อนลบพื้นที่".into())}
    let registry=state.registry.lock().map_err(err)?;
    let file=workspace_db_file(&registry,&workspace_id)?;
    if mode=="all_files"{
        let db=Connection::open_with_flags(state.data_dir.join(&file),OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
        for path in verified_downloads(&db,&registry,&state.data_dir,&workspace_id)?{fs::remove_file(path).map_err(err)?;}
        for path in cache_files(&state.data_dir,&workspace_id)?{fs::remove_file(path).map_err(err)?;}
        let dir=state.data_dir.join("rag-pdf-cache").join(&workspace_id);
        if dir.is_dir(){let _=fs::remove_dir(dir);}
    }
    if workspace_id=="default"{
        let mut db=state.db.lock().map_err(err)?;
        if active_workspace_id(&registry)?!="default"{drop(db);let mut other=Connection::open(state.data_dir.join(&file)).map_err(err)?;clear_workspace_data(&mut other)?;}else{clear_workspace_data(&mut db)?;}
    }else{
        let active=active_workspace_id(&registry)?==workspace_id;
        if active{
            let mut current=state.db.lock().map_err(err)?;
            let next=Connection::open(state.data_dir.join("atlas.sqlite")).map_err(err)?;
            init_db(&next)?;
            registry.execute("UPDATE registry_settings SET value='default' WHERE key='active_workspace'",[]).map_err(err)?;
            *current=next;
        }
        let db_path=state.data_dir.join(&file);
        fs::remove_file(&db_path).map_err(err)?;
        for suffix in ["-wal","-shm"]{let sidecar=state.data_dir.join(format!("{file}{suffix}"));if sidecar.exists(){let _=fs::remove_file(sidecar);}}
        registry.execute("DELETE FROM workspaces WHERE id=?1",[&workspace_id]).map_err(err)?;
    }
    workspace_list(&registry,&state.data_dir)
}
fn setting(conn: &Connection, key: &str) -> Result<String> { conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional().map_err(err).map(|v| v.unwrap_or_default()) }
fn profile(conn: &Connection) -> Result<Profile> {
    let base_url=setting(conn,"base_url")?;
    let username = setting(conn, "username")?;
    let has_credential = !username.is_empty() && password_for(&base_url,&username).is_ok();
    Ok(Profile { base_url, username, download_dir: setting(conn,"download_dir")?, has_credential })
}
fn row_node(r: &rusqlite::Row<'_>) -> rusqlite::Result<Node> { Ok(Node { id: r.get(0)?, portal_id: r.get(1)?, kind: r.get(2)?, name: r.get(3)?, path: r.get(4)?, active: r.get::<_, i64>(5)? != 0 }) }
fn node(conn: &Connection, id: &str) -> Result<Node> { conn.query_row("SELECT id,portal_id,kind,name,path,active FROM nodes WHERE id=?1", [id], row_node).map_err(err) }

#[tauri::command]
fn get_profile(state: State<'_, AppState>) -> Result<Profile> { let db=state.db.lock().map_err(err)?; profile(&db) }
#[tauri::command]
fn save_profile(state: State<'_, AppState>, input: Profile, password: Option<String>) -> Result<Profile> {
    safe_base(&input.base_url)?;
    if input.username.trim().is_empty() { return Err("กรุณาระบุชื่อผู้ใช้".into()); }
    if !input.download_dir.is_empty() && !Path::new(&input.download_dir).is_dir() { return Err("โฟลเดอร์ดาวน์โหลดไม่มีอยู่".into()); }
    let scanning=state.scan_pause.lock().map_err(err)?;
    let mut db = state.db.lock().map_err(err)?;
    let old_user = setting(&db,"username")?;
    let old_base = setting(&db,"base_url")?;
    if (old_base != input.base_url.trim() || old_user != input.username.trim()) && scanning.is_some() { return Err("กรุณาหยุดการสแกนก่อนเปลี่ยน Portal หรือชื่อผู้ใช้".into()); }
    drop(scanning);
    let has_history:i64=db.query_row("SELECT (SELECT COUNT(*) FROM nodes)+(SELECT COUNT(*) FROM scan_sessions)",[],|r|r.get(0)).map_err(err)?;
    if has_history>0 && ((!old_base.is_empty() && old_base!=input.base_url.trim()) || (!old_user.is_empty() && old_user!=input.username.trim())){return Err("พื้นที่นี้มีดัชนีแล้ว กรุณาสร้างพื้นที่สแกนใหม่เพื่อเก็บข้อมูลเดิม".into());}
    if let Some(secret) = password.filter(|p| !p.is_empty()) { credential(input.base_url.trim(),&input.username)?.set_password(&secret).map_err(err)?; }
    else if old_user != input.username || old_base != input.base_url.trim() { return Err("เมื่อเปลี่ยน Portal หรือชื่อผู้ใช้ ต้องระบุรหัสผ่านใหม่".into()); }
    let tx = db.transaction().map_err(err)?;
    for (key,value) in [("base_url",input.base_url.trim()),("username",input.username.trim()),("download_dir",input.download_dir.trim())] {
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(err)?;
    }
    tx.commit().map_err(err)?;
    profile(&db)
}

const SCAN_REQUEST_GAP: Duration=Duration::from_millis(750);
const SCAN_ATTEMPTS:usize=4;
fn scan_http_error(status:reqwest::StatusCode)->String{
    if status==reqwest::StatusCode::REQUEST_TIMEOUT || status==reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error(){format!("retryable: Portal ส่ง HTTP {status}")}else{format!("Portal ส่ง HTTP {status}")}
}
fn scan_response_error(response:&reqwest::Response)->String{
    let mut reason=scan_http_error(response.status());
    if let Some(seconds)=response.headers().get(reqwest::header::RETRY_AFTER).and_then(|v|v.to_str().ok()).and_then(|v|v.parse::<u64>().ok()){reason.push_str(&format!(" [retry-after:{}]",seconds.min(300)));}
    reason
}
fn scan_request_error(e:reqwest::Error)->String{if e.is_timeout()||e.is_connect(){format!("retryable: {e}")}else{err(e)}}
fn retry_delay(attempt:usize,reason:&str)->Duration{
    let base=if reason.contains("HTTP 429"){10*attempt as u64}else{1_u64 << (attempt-1).min(3)};
    let advised=reason.split("[retry-after:").nth(1).and_then(|v|v.split(']').next()).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0).min(300);
    Duration::from_secs(base.max(advised))
}
async fn wait_retry_delay(delay:Duration,flag:&AtomicBool)->Result<()> {
    let until=Instant::now()+delay;
    loop {if flag.load(Ordering::Relaxed){return Err("paused".into())}let left=until.saturating_duration_since(Instant::now());if left.is_zero(){return Ok(())}tokio::time::sleep(left.min(Duration::from_millis(250))).await;}
}
struct Portal { client: Client, base: Url, last_request: tokio::sync::Mutex<Option<Instant>> }
impl Portal {
    async fn wait_turn(&self){let mut last=self.last_request.lock().await;if let Some(previous)=*last{let remaining=SCAN_REQUEST_GAP.saturating_sub(previous.elapsed());if !remaining.is_zero(){tokio::time::sleep(remaining).await;}}*last=Some(Instant::now());}
    async fn connect(p: &Profile) -> Result<Self> {
        let base = safe_base(&p.base_url)?;
        let password = password_for(&p.base_url,&p.username).map_err(|_| "ไม่พบรหัสผ่านใน OS credential store".to_string())?;
        let allowed_origin = base.origin();
        let client = Client::builder().cookie_store(true).timeout(Duration::from_secs(30)).redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 || attempt.url().origin() != allowed_origin { attempt.stop() } else { attempt.follow() }
        })).build().map_err(err)?;
        let portal=Self{client,base,last_request:tokio::sync::Mutex::new(None)};
        portal.wait_turn().await;
        let login_url = portal.base.join("login").map_err(err)?;
        let login_response = portal.client.get(login_url.clone()).send().await.map_err(scan_request_error)?;
        if !login_response.status().is_success() && login_response.status() != reqwest::StatusCode::UNAUTHORIZED { return Err(format!("หน้า login: {}",scan_response_error(&login_response))); }
        let page = login_response.text().await.map_err(scan_request_error)?;
        let (post_url,mut form) = parse_login_form(&portal.base,&page)?;
        form.push(("signin[username]".into(),p.username.clone()));
        form.push(("signin[password]".into(),password));
        portal.wait_turn().await;
        let response = portal.client.post(post_url).form(&form).send().await.map_err(scan_request_error)?;
        let status = response.status();
        let final_url = response.url().clone();
        if final_url.origin() != portal.base.origin() { return Err("Portal ส่งต่อไปยังปลายทางที่ไม่ใช่ Portal เดิม".into()); }
        if status==reqwest::StatusCode::REQUEST_TIMEOUT || status==reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error(){return Err(format!("หน้า login: {}",scan_response_error(&response)))}
        let body = response.text().await.map_err(scan_request_error)?;
        if !status.is_success() || body.contains("signin[password]") || body.contains("type=\"password\"") { return Err("เข้าสู่ระบบไม่สำเร็จ กรุณาตรวจสอบบัญชีและรูปแบบฟอร์มของ Portal".into()); }
        Ok(portal)
    }
    async fn discover_root(&self) -> Result<String> {
        let url=self.base.join("browse/folder/index/").map_err(err)?;
        self.wait_turn().await;
        let response=self.client.get(url).send().await.map_err(scan_request_error)?;
        if !response.status().is_success(){return Err(scan_response_error(&response));}
        if response.url().origin()!=self.base.origin(){return Err("อ่านหน้า Browse ของ Portal ไม่สำเร็จ".into());}
        let html=response.text().await.map_err(scan_request_error)?;
        discover_root_id(&html)
    }
    async fn folder(&self, id: &str) -> Result<(Vec<Node>,Vec<Edge>)> {
        validate_id(id)?;
        let mut url = self.base.join(&format!("folder/{id}?itemsPerPage=100")).map_err(err)?;
        let mut visited_pages=HashSet::new();let mut seen_nodes=HashSet::new();
        let mut nodes=Vec::new();let mut edges=Vec::new();
        for _ in 0..1000 {
            if !visited_pages.insert(url.to_string()){return Err(format!("Folder {id} มีลิงก์หน้าซ้ำ"));}
            let html=self.fetch_folder_page(id,url.clone()).await?;
            let (page_nodes,page_edges)=parse_folder_html(&self.base,id,&html)?;
            for (node,edge) in page_nodes.into_iter().zip(page_edges){if seen_nodes.insert(node.id.clone()){nodes.push(node);edges.push(edge);}}
            if let Some(next)=next_folder_page(&self.base,id,&url,&html)?{url=next;}else{return Ok((nodes,edges));}
        }
        Err(format!("Folder {id} มีจำนวนหน้าเกินขอบเขตที่รองรับ"))
    }
    async fn fetch_folder_page(&self,id:&str,url:Url)->Result<String>{
        self.wait_turn().await;
        let response=self.client.get(url).send().await.map_err(scan_request_error)?;
        if response.url().origin()!=self.base.origin(){return Err("Portal เปลี่ยนปลายทางไปนอก origin".into())}
        if response.status()==reqwest::StatusCode::UNAUTHORIZED || response.url().path().contains("login"){return Err("auth: Session หมดอายุหรือ Portal ขอเข้าสู่ระบบใหม่".into())}
        if !response.status().is_success(){return Err(format!("Folder {id}: {}",scan_response_error(&response)))}
        response.text().await.map_err(scan_request_error)
    }
    async fn document(&self, id: &str) -> Result<Vec<u8>> {
        validate_id(id)?;
        self.wait_turn().await;
        let url = self.base.join(&format!("browse/queue/downloadThumbnail/id_document/{id}?preview=1&log=false&print=0")).map_err(err)?;
        let mut response = self.client.get(url).timeout(Duration::from_secs(120)).send().await.map_err(err)?.error_for_status().map_err(err)?;
        if response.url().origin() != self.base.origin() { return Err("Portal เปลี่ยนปลายทางไปนอก origin".into()); }
        const MAX_PDF_BYTES: usize = 100 * 1024 * 1024;
        if response.content_length().is_some_and(|size| size > MAX_PDF_BYTES as u64) { return Err("PDF ใหญ่เกิน 100 MB".into()); }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(err)? {
            if bytes.len() + chunk.len() > MAX_PDF_BYTES { return Err("PDF ใหญ่เกิน 100 MB".into()); }
            bytes.extend_from_slice(&chunk);
        }
        if !bytes.starts_with(b"%PDF-") { return Err("ไฟล์ที่ได้รับไม่ใช่ PDF".into()); }
        Ok(bytes)
    }
}

fn discover_root_id(html:&str)->Result<String>{
    let doc=Html::parse_document(html);
    let selector=Selector::parse("form#form_browse input[name='folder']").map_err(err)?;
    let id=doc.select(&selector).next().and_then(|el|el.value().attr("value")).ok_or("ไม่พบ Root Folder ในหน้า Browse")?.to_string();
    validate_id(&id)?;
    Ok(id)
}

fn parse_login_form(base:&Url,page:&str)->Result<(Url,Vec<(String,String)>)>{
    let doc=Html::parse_document(page);
    let form_selector=Selector::parse("form").map_err(err)?;
    let input_selector=Selector::parse("input[name]").map_err(err)?;
    let form=doc.select(&form_selector).find(|f| f.select(&input_selector).any(|i| i.value().attr("name")==Some("signin[password]"))).ok_or("ไม่พบฟอร์ม login ที่รองรับ")?;
    let post_url=base.join(form.value().attr("action").unwrap_or("/login")).map_err(err)?;
    if post_url.origin()!=base.origin(){return Err("ฟอร์ม login ชี้ไปนอก Portal".into());}
    let fields=form.select(&input_selector).filter_map(|el|{
        let name=el.value().attr("name")?;
        let kind=el.value().attr("type").unwrap_or("");
        if kind.eq_ignore_ascii_case("hidden"){Some((name.to_string(),el.value().attr("value").unwrap_or("").to_string()))}else{None}
    }).collect();
    Ok((post_url,fields))
}

fn parse_folder_html(base:&Url,id:&str,html:&str)->Result<(Vec<Node>,Vec<Edge>)>{
    let doc = Html::parse_document(html);
    let selector = Selector::parse("a[id^='folderItem'][href], .docNameList a[href]").map_err(err)?;
    let mut nodes = Vec::new(); let mut edges = Vec::new(); let mut seen = HashSet::new();
    for el in doc.select(&selector) {
        let href = el.value().attr("href").unwrap_or("");
        let Some(link) = base.join(href).ok() else { continue };
        if link.origin() != base.origin() { continue; }
        let parts: Vec<_> = link.path_segments().map(|s| s.collect()).unwrap_or_default();
        let detected = if parts.len() >= 2 && parts[parts.len()-2] == "folder" { Some(("folder",parts[parts.len()-1])) }
            else if parts.len() >= 2 && parts[parts.len()-2] == "idDocument" { Some(("file",parts[parts.len()-1])) }
            else if parts.len() >= 2 && parts[parts.len()-2] == "id_document" { Some(("file",parts[parts.len()-1])) } else { None };
        let Some((kind,portal_id)) = detected else { continue };
        if validate_id(portal_id).is_err() || (kind == "folder" && portal_id == id) { continue; }
        let node_id = format!("{kind}:{portal_id}");
        if !seen.insert(node_id.clone()) { continue; }
        let label = el.text().collect::<Vec<_>>().join(" ").trim().to_string();
        let name = if label.is_empty() { el.value().attr("title").unwrap_or(portal_id).to_string() } else { label };
        nodes.push(Node { id: node_id.clone(), portal_id: portal_id.into(), kind: kind.into(), name, path: String::new(), active: true });
        edges.push(Edge { source: format!("folder:{id}"), target: node_id });
    }
    Ok((nodes,edges))
}

fn next_folder_page(base:&Url,id:&str,current:&Url,html:&str)->Result<Option<Url>>{
    let doc=Html::parse_document(html);
    let selector=Selector::parse("a[href]").map_err(err)?;
    let current_parts:Vec<_>=current.path_segments().map(|s|s.collect()).unwrap_or_default();
    let current_page=current_parts.get(2).and_then(|s|s.parse::<u32>().ok())
        .or_else(||current.query_pairs().find(|(k,_)|k=="page").and_then(|(_,v)|v.parse::<u32>().ok())).unwrap_or(1);
    let mut candidate:Option<(u32,Url)>=None;
    for anchor in doc.select(&selector){
        let Some(link)=anchor.value().attr("href").and_then(|href|base.join(href).ok()) else {continue};
        if link.origin()!=base.origin(){continue;}
        let parts:Vec<_>=link.path_segments().map(|s|s.collect()).unwrap_or_default();
        let page=if parts.len()==3 && parts[0]=="folder" && parts[1]==id {parts[2].parse::<u32>().ok()}
            else if parts.len()==2 && parts[0]=="folder" && parts[1]==id {link.query_pairs().find(|(k,_)|k=="page").and_then(|(_,v)|v.parse::<u32>().ok())}
            else {None};
        if let Some(page)=page {if page>current_page && candidate.as_ref().is_none_or(|(best,_)|page<*best){candidate=Some((page,link));}}
    }
    Ok(candidate.map(|(_,url)|url))
}

#[tauri::command]
async fn test_connection(state: State<'_, AppState>) -> Result<String> {
    let p = { let db=state.db.lock().map_err(err)?; profile(&db)? };
    let portal = Portal::connect(&p).await?;
    let root=portal.discover_root().await?;
    portal.folder(&root).await?;
    Ok(format!("เชื่อมต่อและค้นพบ Root Folder {root} อัตโนมัติแล้ว"))
}

fn upsert_page_tx(tx: &Transaction<'_>, folder: &Node, children: &[Node], edges: &[Edge]) -> Result<()> {
    for n in std::iter::once(folder).chain(children.iter()) {
        tx.execute("INSERT INTO nodes(id,portal_id,kind,name,path,active) VALUES(?1,?2,?3,?4,?5,1) ON CONFLICT(id) DO UPDATE SET name=excluded.name,path=excluded.path,active=1,last_seen=CURRENT_TIMESTAMP", params![n.id,n.portal_id,n.kind,n.name,n.path]).map_err(err)?;
    }
    tx.execute("DELETE FROM edges WHERE source=?1", [&folder.id]).map_err(err)?;
    for e in edges { tx.execute("INSERT OR IGNORE INTO edges(source,target) VALUES(?1,?2)", params![e.source,e.target]).map_err(err)?; }
    Ok(())
}
#[cfg(test)]
fn upsert_page(db: &mut Connection, folder: &Node, children: &[Node], edges: &[Edge]) -> Result<()> {
    let tx = db.transaction().map_err(err)?;
    upsert_page_tx(&tx,folder,children,edges)?;
    tx.commit().map_err(err)
}
fn source_key(p:&Profile)->Result<String>{ Ok(format!("{}|{}",safe_base(&p.base_url)?,p.username.trim())) }
fn row_scan_session(r:&rusqlite::Row<'_>)->rusqlite::Result<ScanSession>{Ok(ScanSession{id:r.get(0)?,status:r.get(1)?,current_folder:r.get(2)?,folders_done:r.get(3)?,folders_pending:r.get(4)?,files_seen:r.get(5)?,last_error:r.get(6)?,updated_at:r.get(7)?,elapsed_seconds:r.get(8)?,retry_attempt:r.get(9)?,started_at:r.get(10)?})}
fn scan_session(db:&Connection,source:&str)->Result<Option<ScanSession>>{
    db.query_row("SELECT s.id,s.status,s.current_folder,s.folders_done,(SELECT COUNT(*) FROM scan_queue q WHERE q.session_id=s.id AND q.status='pending'),s.files_seen,s.last_error,s.updated_at,s.elapsed_seconds,s.retry_attempt,s.started_at FROM scan_sessions s WHERE s.source_key=?1 ORDER BY s.started_at DESC,s.rowid DESC LIMIT 1",[source],row_scan_session).optional().map_err(err)
}
#[tauri::command]
fn list_scan_sessions(state:State<'_,AppState>)->Result<Vec<ScanSession>>{
    let db=state.db.lock().map_err(err)?;let p=profile(&db)?;if p.base_url.is_empty(){return Ok(Vec::new())}
    let source=source_key(&p)?;
    let mut stmt=db.prepare("SELECT s.id,s.status,s.current_folder,s.folders_done,(SELECT COUNT(*) FROM scan_queue q WHERE q.session_id=s.id AND q.status='pending'),s.files_seen,s.last_error,s.updated_at,s.elapsed_seconds,s.retry_attempt,s.started_at FROM scan_sessions s WHERE s.source_key=?1 ORDER BY s.started_at DESC,s.rowid DESC LIMIT 30").map_err(err)?;
    let rows=stmt.query_map([source],row_scan_session).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    Ok(rows)
}
fn scan_next(db:&Connection,id:&str)->Result<Option<(String,String,String)>>{
    db.query_row("SELECT folder_id,name,parent_path FROM scan_queue WHERE session_id=?1 AND status='pending' ORDER BY ordinal LIMIT 1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)
}
fn commit_scan_page(db:&mut Connection,session_id:&str,parent:&Node,children:&[Node],edges:&[Edge])->Result<ScanSession>{
    let tx=db.transaction().map_err(err)?;
    upsert_page_tx(&tx,parent,children,edges)?;
    for child in children {
        if child.kind=="folder" {tx.execute("INSERT OR IGNORE INTO scan_queue(session_id,folder_id,name,parent_path) VALUES(?1,?2,?3,?4)",params![session_id,child.portal_id,child.name,parent.path]).map_err(err)?;}
        else {tx.execute("INSERT OR IGNORE INTO scan_files(session_id,node_id) VALUES(?1,?2)",params![session_id,child.id]).map_err(err)?;}
    }
    tx.execute("UPDATE scan_queue SET status='done' WHERE session_id=?1 AND folder_id=?2",params![session_id,parent.portal_id]).map_err(err)?;
    tx.execute("UPDATE scan_sessions SET folders_done=folders_done+1,files_seen=(SELECT COUNT(*) FROM scan_files WHERE session_id=?1),updated_at=CURRENT_TIMESTAMP WHERE id=?1",[session_id]).map_err(err)?;
    tx.commit().map_err(err)?;
    scan_session(db,&db.query_row("SELECT source_key FROM scan_sessions WHERE id=?1",[session_id],|r|r.get::<_,String>(0)).map_err(err)?)?.ok_or("ไม่พบ scan session".into())
}
fn emit_scan(app:&AppHandle,db:&Connection,session:&ScanSession,level:&str,message:String){
    let _=db.execute("INSERT INTO scan_logs(session_id,level,message) VALUES(?1,?2,?3)",params![session.id,level,message]);
    let _=app.emit("atlas-progress",Progress{kind:"scan".into(),status:session.status.clone(),current:session.folders_done as usize,total:(session.folders_done+session.folders_pending) as usize,message});
}
#[tauri::command]
fn get_scan_logs(state:State<'_,AppState>,session_id:String)->Result<Vec<ScanLog>>{
    let db=state.db.lock().map_err(err)?;
    let mut stmt=db.prepare("SELECT id,session_id,level,message,created_at FROM (SELECT id,session_id,level,message,created_at FROM scan_logs WHERE session_id=?1 ORDER BY id DESC LIMIT 300) ORDER BY id").map_err(err)?;
    let logs=stmt.query_map([session_id],|r|Ok(ScanLog{id:r.get(0)?,session_id:r.get(1)?,level:r.get(2)?,message:r.get(3)?,created_at:r.get(4)?})).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    Ok(logs)
}
async fn retry_scan<T,F,Fut>(app:&AppHandle,state:&AppState,source:&str,session_id:&str,label:&str,flag:&AtomicBool,mut operation:F)->Result<T>
where F:FnMut()->Fut,Fut:std::future::Future<Output=Result<T>> {
    for attempt in 1..=SCAN_ATTEMPTS {
        if flag.load(Ordering::Relaxed){return Err("paused".into())}
        {let db=state.db.lock().map_err(err)?;db.execute("UPDATE scan_sessions SET retry_attempt=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![session_id,attempt]).map_err(err)?;}
        match operation().await {
            Ok(value)=>{let db=state.db.lock().map_err(err)?;db.execute("UPDATE scan_sessions SET retry_attempt=0,last_error='',updated_at=CURRENT_TIMESTAMP WHERE id=?1",[session_id]).map_err(err)?;return Ok(value)},
            Err(reason)=>{
                let retryable=reason.contains("retryable:");
                let clean=reason.replace("retryable: ","");
                if !retryable || attempt==SCAN_ATTEMPTS{return Err(if retryable{format!("{label} ล้มเหลวหลังลอง {SCAN_ATTEMPTS} ครั้ง: {clean}")}else{clean})}
                let delay=retry_delay(attempt,&reason);
                let message=format!("{label} สะดุด: {clean} · จะลองครั้งที่ {}/{} ใน {} วินาที",attempt+1,SCAN_ATTEMPTS,delay.as_secs());
                {let db=state.db.lock().map_err(err)?;db.execute("UPDATE scan_sessions SET last_error=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![session_id,message]).map_err(err)?;if let Some(session)=scan_session(&db,source)?{emit_scan(app,&db,&session,"warn",message);}}
                wait_retry_delay(delay,flag).await?;
            }
        }
    }
    Err("retry limit".into())
}
fn spawn_scan(app:AppHandle,p:Profile,session_id:String,flag:Arc<AtomicBool>){
    tauri::async_runtime::spawn(async move {
        let state=app.state::<AppState>();
        let elapsed_base=state.db.lock().ok().and_then(|db|db.query_row("SELECT elapsed_seconds FROM scan_sessions WHERE id=?1",[&session_id],|r|r.get::<_,i64>(0)).ok()).unwrap_or(0);
        let run_started=Instant::now();
        let heartbeat_stop=Arc::new(AtomicBool::new(false));
        let heartbeat_flag=heartbeat_stop.clone();let heartbeat_app=app.clone();let heartbeat_id=session_id.clone();
        let heartbeat=tauri::async_runtime::spawn(async move {loop {tokio::time::sleep(Duration::from_secs(5)).await;if heartbeat_flag.load(Ordering::Relaxed){break}if let Ok(db)=heartbeat_app.state::<AppState>().db.lock(){let _=db.execute("UPDATE scan_sessions SET elapsed_seconds=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![heartbeat_id,elapsed_base+run_started.elapsed().as_secs() as i64]);}}});
        let outcome=async {
            let source=source_key(&p)?;
            let mut portal=retry_scan(&app,&state,&source,&session_id,"เชื่อมต่อ Portal",&flag,||Portal::connect(&p)).await?;
            let needs_seed={
                let db=state.db.lock().map_err(err)?;
                let count:i64=db.query_row("SELECT COUNT(*) FROM scan_queue WHERE session_id=?1",[&session_id],|r|r.get(0)).map_err(err)?;
                count==0
            };
            if needs_seed {
                let root=retry_scan(&app,&state,&source,&session_id,"ค้นหา Root Folder",&flag,||portal.discover_root()).await?;
                let db=state.db.lock().map_err(err)?;
                db.execute("INSERT OR IGNORE INTO scan_queue(session_id,folder_id,name,parent_path) VALUES(?1,?2,'Portal','')",params![session_id,root]).map_err(err)?;
            }
            loop {
                if flag.load(Ordering::Relaxed){return Ok::<_,String>("paused".to_string());}
                let item={let db=state.db.lock().map_err(err)?;scan_next(&db,&session_id)?};
                let Some((id,name,parent_path))=item else {return Ok("completed".to_string())};
                let own_path=if parent_path.is_empty(){name.clone()}else{format!("{parent_path}/{name}")};
                {
                    let db=state.db.lock().map_err(err)?;
                    db.execute("UPDATE scan_sessions SET current_folder=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![session_id,own_path]).map_err(err)?;
                    if let Some(s)=scan_session(&db,&source_key(&p)?)?{emit_scan(&app,&db,&s,"info",format!("กำลังอ่าน {own_path} (Folder {id})"));}
                }
                let fetched=retry_scan(&app,&state,&source,&session_id,&format!("Folder {id}"),&flag,||portal.folder(&id)).await;
                let (mut children,edges)=match fetched {
                    Err(e) if e.starts_with("auth:")=>{
                        {let db=state.db.lock().map_err(err)?;db.execute("UPDATE scan_sessions SET last_error='Session หมดอายุ กำลังเชื่อมต่อใหม่',updated_at=CURRENT_TIMESTAMP WHERE id=?1",[&session_id]).map_err(err)?;if let Some(s)=scan_session(&db,&source)?{emit_scan(&app,&db,&s,"warn","Session หมดอายุ กำลังเชื่อมต่อใหม่".into());}}
                        portal=retry_scan(&app,&state,&source,&session_id,"เชื่อมต่อ Portal ใหม่",&flag,||Portal::connect(&p)).await?;
                        retry_scan(&app,&state,&source,&session_id,&format!("Folder {id}"),&flag,||portal.folder(&id)).await?
                    },
                    other=>other?
                };
                for child in &mut children{child.path=format!("{own_path}/{}",child.name);}
                let parent=Node{id:format!("folder:{id}"),portal_id:id,kind:"folder".into(),name,path:own_path,active:true};
                let mut db=state.db.lock().map_err(err)?;
                let session=commit_scan_page(&mut db,&session_id,&parent,&children,&edges)?;
                emit_scan(&app,&db,&session,"success",format!("อ่านแล้ว {} โฟลเดอร์ · พบ {} เอกสาร · รออีก {} โฟลเดอร์",session.folders_done,session.files_seen,session.folders_pending));
            }
        }.await;
        heartbeat_stop.store(true,Ordering::Relaxed);heartbeat.abort();
        let (status,message)=match outcome {Ok(s) if s=="paused"=>("paused","หยุดการสแกนชั่วคราว กดสแกนต่อได้".to_string()),Ok(_)=>("completed","สแกนทั้ง Portal สำเร็จ".to_string()),Err(e) if e=="paused"=>("paused","หยุดการสแกนชั่วคราว กดสแกนต่อได้".to_string()),Err(e)=>("error",e)};
        let mut running=state.scan_pause.lock().ok();
        if let Ok(db)=state.db.lock(){
            let error=if status=="error"{message.as_str()}else{""};
            let _=db.execute("UPDATE scan_sessions SET status=?2,last_error=?3,elapsed_seconds=?4,retry_attempt=0,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![session_id,status,error,elapsed_base+run_started.elapsed().as_secs() as i64]);
            if status=="completed" {
                let _=db.execute("UPDATE nodes SET active=CASE WHEN kind='folder' THEN EXISTS(SELECT 1 FROM scan_queue q WHERE q.session_id=?1 AND q.folder_id=nodes.portal_id AND q.status='done') ELSE EXISTS(SELECT 1 FROM scan_files f WHERE f.session_id=?1 AND f.node_id=nodes.id) END",[&session_id]);
            }
            if let Ok(Some(s))=scan_session(&db,&source_key(&p).unwrap_or_default()){emit_scan(&app,&db,&s,if status=="error"{"error"}else{"success"},message);}
        };
        if let Some(ref mut lock)=running{**lock=None;}
    });
}
#[tauri::command]
fn get_scan_session(state:State<'_,AppState>)->Result<Option<ScanSession>>{
    let db=state.db.lock().map_err(err)?;let p=profile(&db)?;
    if p.base_url.is_empty(){return Ok(None)}
    scan_session(&db,&source_key(&p)?)
}
#[tauri::command]
fn start_scan(app:AppHandle,state:State<'_,AppState>)->Result<ScanSession>{
    let mut running=state.scan_pause.lock().map_err(err)?;
    if running.is_some(){return Err("กำลังสแกนอยู่".into())}
    let db=state.db.lock().map_err(err)?;let p=profile(&db)?;
    if !p.has_credential{return Err("กรุณาตั้งค่า Portal และรหัสผ่านก่อน".into())}
    let source=source_key(&p)?;
    if let Some(previous)=scan_session(&db,&source)?{if matches!(previous.status.as_str(),"paused"|"error"|"running"|"pausing") && (previous.folders_pending>0 || previous.folders_done==0){return Err("มี scan session ค้างอยู่ กรุณากดสแกนต่อ".into())}}
    let id=Uuid::new_v4().to_string();
    db.execute("INSERT INTO scan_sessions(id,source_key,status) VALUES(?1,?2,'running')",params![id,source]).map_err(err)?;
    let session=scan_session(&db,&source)?.ok_or("สร้าง scan session ไม่สำเร็จ")?;
    emit_scan(&app,&db,&session,"info","เริ่มสำรวจ Portal และค้นหา Root Folder".into());
    let flag=Arc::new(AtomicBool::new(false));*running=Some(flag.clone());drop(db);drop(running);
    spawn_scan(app,p,id,flag);Ok(session)
}
#[tauri::command]
fn pause_scan(state:State<'_,AppState>)->Result<()> {
    let running=state.scan_pause.lock().map_err(err)?;
    if let Some(flag)=running.as_ref(){flag.store(true,Ordering::Relaxed);let db=state.db.lock().map_err(err)?;db.execute("UPDATE scan_sessions SET status='pausing',updated_at=CURRENT_TIMESTAMP WHERE status='running'",[]).map_err(err)?;}
    Ok(())
}
#[tauri::command]
fn resume_scan(app:AppHandle,state:State<'_,AppState>,session_id:String)->Result<ScanSession>{
    let mut running=state.scan_pause.lock().map_err(err)?;
    if running.is_some(){return Err("กำลังสแกนอยู่".into())}
    let db=state.db.lock().map_err(err)?;let p=profile(&db)?;let source=source_key(&p)?;
    let session=scan_session(&db,&source)?.ok_or("ไม่พบ scan session")?;
    if session.id!=session_id || !matches!(session.status.as_str(),"paused"|"error") {return Err("session นี้สแกนต่อไม่ได้".into())}
    db.execute("UPDATE scan_sessions SET status='running',last_error='',updated_at=CURRENT_TIMESTAMP WHERE id=?1",[&session_id]).map_err(err)?;
    let updated=scan_session(&db,&source)?.ok_or("ไม่พบ scan session")?;
    emit_scan(&app,&db,&updated,"info","สแกนต่อจากคิวที่บันทึกไว้".into());
    let flag=Arc::new(AtomicBool::new(false));*running=Some(flag.clone());drop(db);drop(running);
    spawn_scan(app,p,session_id,flag);Ok(updated)
}

#[tauri::command]
fn search_nodes(state: State<'_, AppState>, query: String, kind: Option<String>) -> Result<Vec<Node>> {
    let db = state.db.lock().map_err(err)?;
    let pattern = format!("%{}%",query.trim().replace('%',"\\%").replace('_',"\\_"));
    let mut stmt = db.prepare("SELECT id,portal_id,kind,name,path,active FROM nodes WHERE active=1 AND (?1='' OR name LIKE ?3 ESCAPE '\\' OR path LIKE ?3 ESCAPE '\\' OR portal_id LIKE ?3 ESCAPE '\\') AND (?2='' OR kind=?2) ORDER BY CASE kind WHEN 'folder' THEN 0 ELSE 1 END,name LIMIT 150").map_err(err)?;
    let rows=stmt.query_map(params![query.trim(),kind.unwrap_or_default(),pattern],row_node).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    Ok(rows)
}
#[tauri::command]
fn graph_roots(db: &Connection) -> Result<Graph> {
    let mut stmt=db.prepare("SELECT n.id,n.portal_id,n.kind,n.name,n.path,n.active FROM nodes n WHERE n.active=1 AND n.kind='folder' AND NOT EXISTS (SELECT 1 FROM edges e JOIN nodes p ON p.id=e.source AND p.active=1 AND p.kind='folder' WHERE e.target=n.id) ORDER BY CASE WHEN n.path='Portal' THEN 0 ELSE 1 END,n.name LIMIT 40").map_err(err)?;
    let nodes=stmt.query_map([],row_node).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    Ok(Graph{nodes,edges:Vec::new()})
}
#[tauri::command]
fn get_graph_roots(state: State<'_, AppState>) -> Result<Graph> { let db=state.db.lock().map_err(err)?;graph_roots(&db) }
#[tauri::command]
fn get_graph_context(state: State<'_, AppState>, node_id: String) -> Result<Graph> {
    let db=state.db.lock().map_err(err)?;
    let selected=node(&db,&node_id)?;
    if !selected.active { return Err("โหนดนี้ไม่อยู่ในดัชนีปัจจุบัน".into()); }
    let mut stmt=db.prepare("SELECT p.id,p.portal_id,p.kind,p.name,p.path,p.active FROM edges e JOIN nodes p ON p.id=e.source WHERE e.target=?1 AND p.active=1 AND p.kind='folder' LIMIT 8").map_err(err)?;
    let parents=stmt.query_map([&node_id],row_node).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    let edges=parents.iter().map(|p|Edge{source:p.id.clone(),target:node_id.clone()}).collect();
    let mut nodes=vec![selected];nodes.extend(parents);
    Ok(Graph{nodes,edges})
}
fn graph_children(db:&Connection,folder_id:&str,offset:i64)->Result<GraphPage>{
    let folder=node(db,folder_id)?;
    if folder.kind!="folder" || !folder.active {return Err("กรุณาเลือก Folder ที่อยู่ในดัชนี".into());}
    let mut stmt=db.prepare("SELECT n.id,n.portal_id,n.kind,n.name,n.path,n.active FROM edges e JOIN nodes n ON n.id=e.target WHERE e.source=?1 AND n.active=1 ORDER BY CASE n.kind WHEN 'folder' THEN 0 ELSE 1 END,n.name,n.id LIMIT 61 OFFSET ?2").map_err(err)?;
    let mut nodes=stmt.query_map(params![folder_id,offset.max(0)],row_node).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    let has_more=nodes.len()>60;nodes.truncate(60);
    let edges=nodes.iter().map(|n|Edge{source:folder_id.into(),target:n.id.clone()}).collect();
    Ok(GraphPage{nodes,edges,has_more})
}
#[tauri::command]
fn get_graph_children(state: State<'_, AppState>, folder_id: String, offset: i64) -> Result<GraphPage> {let db=state.db.lock().map_err(err)?;graph_children(&db,&folder_id,offset)}
fn plan(db: &Connection, node_id: &str, scope: &str) -> Result<DownloadPlan> {
    let selected=node(db,node_id)?;
    if !matches!(scope,"document"|"folder_direct"|"folder_subtree") { return Err("ขอบเขตดาวน์โหลดไม่ถูกต้อง".into()); }
    if (scope=="document") != (selected.kind=="file") { return Err("ขอบเขตไม่ตรงกับชนิดโหนด".into()); }
    let mut queue=VecDeque::from([node_id.to_string()]); let mut seen=HashSet::new(); let mut file_ids=HashSet::new(); let mut items=Vec::new(); let mut duplicates=0;
    while let Some(id)=queue.pop_front() {
        if !seen.insert(id.clone()) { continue; }
        let current=node(db,&id)?;
        if current.kind=="file" { if file_ids.insert(id) { items.push(current); } else { duplicates+=1; } continue; }
        let mut stmt=db.prepare("SELECT target FROM edges WHERE source=?1").map_err(err)?;
        let children=stmt.query_map([&id],|r|r.get::<_,String>(0)).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
        for child in children { let n=node(db,&child)?; if n.kind=="file" { if file_ids.insert(child) { items.push(n); } else { duplicates+=1; } } else if scope=="folder_subtree" { queue.push_back(child); } }
    }
    Ok(DownloadPlan { node_id:node_id.into(),scope:scope.into(),items,duplicate_count:duplicates })
}
#[tauri::command]
fn plan_download(state: State<'_, AppState>, node_id: String, scope: String) -> Result<DownloadPlan> { let db=state.db.lock().map_err(err)?; plan(&db,&node_id,&scope) }
fn safe_name(input: &str, id: &str) -> String {
    let trimmed=input.trim().trim_end_matches(['.',' ']);
    let clean:String=trimmed.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) {'_'} else {c}).take(120).collect();
    let stem=if clean.is_empty() { format!("document-{id}") } else { clean };
    let stem=if stem.to_lowercase().ends_with(".pdf") { stem[..stem.len()-4].to_string() } else { stem };
    format!("{stem} [{id}].pdf")
}
fn finish_job(db:&Connection,id:&str,status:&str) { let _=db.execute("UPDATE jobs SET status=?2 WHERE id=?1",params![id,status]); }
async fn write_downloaded_pdf(destination:&Path,item:&Node,bytes:&[u8])->Result<(PathBuf,String,String)>{
    if !bytes.starts_with(b"%PDF-"){return Err("ไฟล์ที่ได้รับไม่ใช่ PDF".into())}
    let target=destination.join(safe_name(&item.name,&item.portal_id));
    let digest=hex::encode(Sha256::digest(bytes));
    match tokio::fs::symlink_metadata(&target).await {
        Ok(metadata)=>{
            if !metadata.file_type().is_file() || metadata.len()>100*1024*1024{return Err(format!("ไฟล์ปลายทาง {} ไม่ใช่ PDF ปกติ",target.display()))}
            let existing=tokio::fs::read(&target).await.map_err(err)?;
            if existing.starts_with(b"%PDF-") && hex::encode(Sha256::digest(&existing))==digest{return Ok((target,"skipped".into(),digest));}
            return Err(format!("ไฟล์ปลายทาง {} มีเนื้อหาต่างจาก Portal กรุณาย้ายหรือเปลี่ยนชื่อไฟล์ก่อนดาวน์โหลดใหม่",target.display()));
        }
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
        Err(e)=>return Err(err(e)),
    }
    let partial=target.with_extension(format!("pdf.{}.part",Uuid::new_v4()));
    tokio::fs::write(&partial,bytes).await.map_err(err)?;
    if let Err(e)=tokio::fs::rename(&partial,&target).await{let _=tokio::fs::remove_file(&partial).await;return Err(err(e))}
    Ok((target,"completed".into(),digest))
}
#[tauri::command]
async fn start_download(app:AppHandle,state:State<'_,AppState>,node_id:String,scope:String)->Result<String>{
    let (p,planned)={let db=state.db.lock().map_err(err)?;(profile(&db)?,plan(&db,&node_id,&scope)?)};
    launch_download(app,&state,p,planned.items)
}
fn launch_download(app:AppHandle,state:&AppState,p:Profile,items:Vec<Node>)->Result<String>{
    let mut running=state.download_cancel.lock().map_err(err)?;
    if running.is_some(){return Err("มีงานดาวน์โหลดกำลังทำงาน".into());}
    if items.is_empty(){return Err("ไม่มีเอกสารในขอบเขตที่เลือก".into());}
    let destination=PathBuf::from(&p.download_dir);
    if !destination.is_dir(){return Err("กรุณาเลือกโฟลเดอร์ดาวน์โหลดใน Settings".into());}
    let flag=Arc::new(AtomicBool::new(false));
    let job_id=Uuid::new_v4().to_string();
    {let mut db=state.db.lock().map_err(err)?;let tx=db.transaction().map_err(err)?;
        tx.execute("INSERT INTO jobs(id,status,total) VALUES(?1,'running',?2)",params![job_id,items.len()]).map_err(err)?;
        for item in &items {tx.execute("INSERT INTO job_items(job_id,node_id) VALUES(?1,?2)",params![job_id,item.id]).map_err(err)?;} tx.commit().map_err(err)?;}
    *running=Some(flag.clone());drop(running);
    let returned=job_id.clone();
    tauri::async_runtime::spawn(async move {
        let state_handle=app.state::<AppState>();
        let portal=Portal::connect(&p).await;
        let mut completed=0;let mut failed=0;
        for item in &items {
            if flag.load(Ordering::Relaxed){break;}
            let result=async {
                let portal=portal.as_ref().map_err(|e|e.clone())?;
                let bytes=portal.document(&item.portal_id).await?;
                write_downloaded_pdf(&destination,item,&bytes).await
            }.await;
            let (status,path,error,sha)=match result {Ok((path,status,sha))=>{completed+=1;(status,path.to_string_lossy().to_string(),String::new(),sha)},Err(e)=>{failed+=1;("failed".into(),String::new(),e,String::new())}};
            if let Ok(db)=state_handle.db.lock(){let _=db.execute("UPDATE job_items SET status=?3,file_path=?4,error=?5,sha256=?6 WHERE job_id=?1 AND node_id=?2",params![job_id,item.id,status,path,error,sha]);let _=db.execute("UPDATE jobs SET completed=?2,failed=?3 WHERE id=?1",params![job_id,completed,failed]);}
            let _=app.emit("atlas-progress",Progress{kind:"download".into(),status:"running".into(),current:completed+failed,total:items.len(),message:format!("ดาวน์โหลด {completed} สำเร็จ · {failed} ล้มเหลว")});
        }
        let status=if flag.load(Ordering::Relaxed){"cancelled"}else if failed>0{"completed_with_errors"}else{"completed"};
        if let Ok(db)=state_handle.db.lock(){finish_job(&db,&job_id,status);}
        let _=app.emit("atlas-progress",Progress{kind:"download".into(),status:status.into(),current:completed+failed,total:items.len(),message:format!("เสร็จแล้ว: {completed} สำเร็จ · {failed} ล้มเหลว")});
        if let Ok(mut lock)=state_handle.download_cancel.lock(){*lock=None;};
    });
    Ok(returned)
}
#[tauri::command]
fn cancel_download(state:State<'_,AppState>)->Result<()> {if let Some(flag)=state.download_cancel.lock().map_err(err)?.as_ref(){flag.store(true,Ordering::Relaxed);}Ok(())}
#[tauri::command]
fn list_jobs(state:State<'_,AppState>)->Result<Vec<Job>>{
    let db=state.db.lock().map_err(err)?;let mut stmt=db.prepare("SELECT id,status,total,completed,failed,created_at FROM jobs ORDER BY created_at DESC LIMIT 30").map_err(err)?;
    let rows=stmt.query_map([],|r|Ok(Job{id:r.get(0)?,status:r.get(1)?,total:r.get(2)?,completed:r.get(3)?,failed:r.get(4)?,created_at:r.get(5)?})).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    Ok(rows)
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct DownloadedDocument { node_id:String, name:String, path:String, file_path:String }
fn is_pdf_file(path:&Path)->bool{
    use std::io::Read;
    let Ok(metadata)=fs::symlink_metadata(path) else{return false};
    if !metadata.file_type().is_file(){return false}
    let Ok(mut file)=fs::File::open(path) else{return false};
    let mut header=[0u8;5];file.read_exact(&mut header).is_ok() && &header==b"%PDF-"
}

#[tauri::command]
fn search_downloaded_documents(state:State<'_,AppState>,query:String)->Result<Vec<DownloadedDocument>>{
    let db=state.db.lock().map_err(err)?;
    let mut stmt=db.prepare("SELECT j.node_id,n.name,n.path,j.file_path FROM job_items j JOIN nodes n ON n.id=j.node_id WHERE j.status IN ('completed','skipped') AND j.file_path IS NOT NULL ORDER BY j.rowid DESC").map_err(err)?;
    let needle=query.trim().to_lowercase();
    let rows=stmt.query_map([],|r|Ok(DownloadedDocument{node_id:r.get(0)?,name:r.get(1)?,path:r.get(2)?,file_path:r.get(3)?})).map_err(err)?;
    let mut results=Vec::new();
    let mut seen=HashSet::new();
    for row in rows {
        let doc=row.map_err(err)?;
        if seen.contains(&doc.node_id) || !is_pdf_file(Path::new(&doc.file_path)){continue;}
        if needle.is_empty() || format!("{} {} {}",doc.name,doc.path,doc.file_path).to_lowercase().contains(&needle){seen.insert(doc.node_id.clone());results.push(doc);}
        if results.len()==200{break;}
    }
    Ok(results)
}

#[tauri::command]
fn open_downloaded_document(state:State<'_,AppState>,node_id:String)->Result<String>{
    let db=state.db.lock().map_err(err)?;
    let mut stmt=db.prepare("SELECT file_path FROM job_items WHERE node_id=?1 AND status IN ('completed','skipped') AND file_path IS NOT NULL ORDER BY rowid DESC").map_err(err)?;
    let paths=stmt.query_map([node_id],|r|r.get::<_,String>(0)).map_err(err)?;
    for path in paths {
        let path=path.map_err(err)?;
        if is_pdf_file(Path::new(&path)){
            tauri_plugin_opener::open_path(&path,None::<&str>).map_err(err)?;
            return Ok(path);
        }
    }
    Err("ไม่พบไฟล์ PDF ที่ดาวน์โหลดไว้ กรุณาดาวน์โหลดอีกครั้ง".into())
}

pub fn run(){
    unsafe { rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()))); }
    tauri::Builder::default().plugin(tauri_plugin_dialog::init()).setup(|app|{
        let dir=app.path().app_data_dir()?;fs::create_dir_all(&dir)?;
        let registry=Connection::open(dir.join("workspaces.sqlite"))?;init_registry(&registry).map_err(std::io::Error::other)?;
        let active=active_workspace_id(&registry).map_err(std::io::Error::other)?;
        let file:String=registry.query_row("SELECT db_file FROM workspaces WHERE id=?1",[active],|r|r.get(0))?;
        let db=Connection::open(dir.join(file))?;init_db(&db).map_err(std::io::Error::other)?;
        for workspace in workspace_list(&registry,&dir).map_err(std::io::Error::other)? {
            let filename:String=registry.query_row("SELECT db_file FROM workspaces WHERE id=?1",[workspace.id],|r|r.get(0))?;
            let workspace_db=Connection::open(dir.join(filename))?;init_db(&workspace_db).map_err(std::io::Error::other)?;
            recover_interrupted(&workspace_db).map_err(std::io::Error::other)?;
        }
        app.manage(AppState{chat_cancel:Mutex::new(None),chat_request:Mutex::new(String::new()),db:Mutex::new(db),registry:Mutex::new(registry),data_dir:dir,scan_pause:Mutex::new(None),download_cancel:Mutex::new(None),rag_pause:Mutex::new(None)});
        Ok(())
    }).invoke_handler(tauri::generate_handler![chat::get_iso_chat_status,chat::ask_iso_chat,chat::cancel_iso_chat,get_profile,save_profile,list_workspaces,create_workspace,switch_workspace,preview_workspace_delete,delete_workspace,test_connection,get_scan_session,list_scan_sessions,get_scan_logs,start_scan,pause_scan,resume_scan,search_nodes,search_downloaded_documents,open_downloaded_document,get_graph_roots,get_graph_context,get_graph_children,plan_download,start_download,cancel_download,list_jobs,rag::get_rag_config,rag::install_rag_model,rag::save_rag_api,rag::start_rag_index,rag::pause_rag_index,rag::resume_rag_index,rag::get_rag_job,rag::list_rag_documents, rag::get_rag_document_detail, rag::get_rag_document_page,rag::rag_search,rag::rag_related,rag::get_rag_storage,rag::read_rag_document,rag::open_rag_document,iso::iso_import_config,iso::iso_overview,iso::iso_documents,iso::iso_import_results,iso::iso_decide,iso::iso_export,iso::iso_start_download,iso::iso_agent_status,similar::iso_start_index,similar::iso_compute_similarity,similar::iso_similar,similar::iso_check_new_document,similar::iso_write_case]).run(tauri::generate_context!()).expect("error while running Document Atlas");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_reuses_identical_pdf_and_rejects_conflicting_file() {
        let dir=std::env::temp_dir().join(format!("atlas-download-test-{}",Uuid::new_v4()));fs::create_dir_all(&dir).unwrap();
        let item=Node{id:"file:8".into(),portal_id:"8".into(),kind:"file".into(),name:"Report".into(),path:"Portal/Report".into(),active:true};
        let first=b"%PDF-1.4\nfirst";let changed=b"%PDF-1.4\nchanged";
        let (path,status,sha)=tauri::async_runtime::block_on(write_downloaded_pdf(&dir,&item,first)).unwrap();
        assert_eq!(status,"completed");assert_eq!(sha,hex::encode(Sha256::digest(first)));
        let (_,status,_)=tauri::async_runtime::block_on(write_downloaded_pdf(&dir,&item,first)).unwrap();
        assert_eq!(status,"skipped");
        assert!(tauri::async_runtime::block_on(write_downloaded_pdf(&dir,&item,changed)).is_err());
        assert_eq!(fs::read(&path).unwrap(),first);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn workspace_clear_preserves_settings_and_downloaded_file() {
        let dir=std::env::temp_dir().join(format!("atlas-delete-test-{}",Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let registry=Connection::open(dir.join("workspaces.sqlite")).unwrap();init_registry(&registry).unwrap();
        let mut db=Connection::open(dir.join("atlas.sqlite")).unwrap();init_db(&db).unwrap();
        db.execute("INSERT INTO settings(key,value) VALUES('base_url','https://portal.example/')",[]).unwrap();
        db.execute("INSERT INTO nodes(id,portal_id,kind,name,path) VALUES('file:7','7','file','Report','Portal/Report')",[]).unwrap();
        db.execute("INSERT INTO scan_sessions(id,source_key,status) VALUES('scan-1','source','completed')",[]).unwrap();
        db.execute("INSERT INTO scan_logs(session_id,level,message) VALUES('scan-1','info','done')",[]).unwrap();
        db.execute("INSERT INTO jobs(id,status,total) VALUES('job-1','completed',1)",[]).unwrap();
        let path=dir.join(safe_name("Report","7"));let bytes=b"%PDF-1.4\nexample";fs::write(&path,bytes).unwrap();
        db.execute("INSERT INTO job_items(job_id,node_id,status,file_path,sha256) VALUES('job-1','file:7','completed',?1,?2)",params![path.to_string_lossy(),hex::encode(Sha256::digest(bytes))]).unwrap();
        let preview=workspace_delete_preview(&registry,&dir,"default").unwrap();
        assert_eq!(preview.nodes,1);assert_eq!(preview.scan_sessions,1);assert_eq!(preview.download_files.len(),1);
        db.execute("UPDATE nodes SET name='Renamed' WHERE id='file:7'",[]).unwrap();
        assert_eq!(verified_downloads(&db,&registry,&dir,"default").unwrap().len(),1);
        let other_id=Uuid::new_v4().to_string();let other_file=format!("atlas-{other_id}.sqlite");
        let other=Connection::open(dir.join(&other_file)).unwrap();init_db(&other).unwrap();
        other.execute("INSERT INTO job_items(job_id,node_id,status,file_path) VALUES('shared','file:7','skipped',?1)",[path.to_string_lossy().as_ref()]).unwrap();
        registry.execute("INSERT INTO workspaces(id,name,db_file) VALUES(?1,'Other',?2)",params![other_id,other_file]).unwrap();
        assert!(verified_downloads(&db,&registry,&dir,"default").unwrap().is_empty());
        drop(other);
        clear_workspace_data(&mut db).unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM nodes",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM scan_logs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(setting(&db,"base_url").unwrap(),"https://portal.example/");
        assert!(path.exists());
        drop(db);drop(registry);fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retry_policy_slows_down_rate_limits_and_stops_after_four_attempts() {
        assert_eq!(SCAN_ATTEMPTS,4);
        assert_eq!(SCAN_REQUEST_GAP,Duration::from_millis(750));
        assert_eq!(retry_delay(1,"timeout"),Duration::from_secs(1));
        assert_eq!(retry_delay(3,"timeout"),Duration::from_secs(4));
        assert_eq!(retry_delay(2,"HTTP 429"),Duration::from_secs(20));
        assert_eq!(retry_delay(1,"HTTP 429 [retry-after:45]"),Duration::from_secs(45));
        assert!(scan_http_error(reqwest::StatusCode::TOO_MANY_REQUESTS).starts_with("retryable:"));
        assert!(!scan_http_error(reqwest::StatusCode::FORBIDDEN).starts_with("retryable:"));
    }

    #[test]
    fn workspaces_keep_independent_indexes_and_sessions() {
        let dir=std::env::temp_dir().join(format!("atlas-workspaces-test-{}",Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let registry=Connection::open(dir.join("workspaces.sqlite")).unwrap();init_registry(&registry).unwrap();
        let default=Connection::open(dir.join("atlas.sqlite")).unwrap();init_db(&default).unwrap();
        default.execute("INSERT INTO nodes(id,portal_id,kind,name,path) VALUES('folder:1','1','folder','Old Portal','Portal')",[]).unwrap();
        default.execute("INSERT INTO scan_sessions(id,source_key,status) VALUES('run-old','https://old.example/|u','paused')",[]).unwrap();
        let other=Connection::open(dir.join("atlas-new.sqlite")).unwrap();init_db(&other).unwrap();
        registry.execute("INSERT INTO workspaces(id,name,db_file) VALUES('new','New Portal','atlas-new.sqlite')",[]).unwrap();
        other.execute("INSERT INTO nodes(id,portal_id,kind,name,path) VALUES('folder:1','1','folder','New Portal','Portal')",[]).unwrap();
        let spaces=workspace_list(&registry,&dir).unwrap();assert_eq!(spaces.len(),2);
        assert_eq!(node(&default,"folder:1").unwrap().name,"Old Portal");
        assert_eq!(node(&other,"folder:1").unwrap().name,"New Portal");
        assert!(scan_session(&other,"https://old.example/|u").unwrap().is_none());
        drop(other);drop(default);drop(registry);let _=fs::remove_dir_all(dir);
    }

    #[test]
    fn existing_scan_session_migrates_without_losing_queue() {
        let db=Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE scan_sessions(id TEXT PRIMARY KEY,source_key TEXT NOT NULL,status TEXT NOT NULL,current_folder TEXT NOT NULL DEFAULT '',folders_done INTEGER NOT NULL DEFAULT 0,files_seen INTEGER NOT NULL DEFAULT 0,last_error TEXT NOT NULL DEFAULT '',started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); INSERT INTO scan_sessions(id,source_key,status) VALUES('old','https://portal.example/|u','paused'); CREATE TABLE scan_queue(ordinal INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,folder_id TEXT NOT NULL,name TEXT NOT NULL,parent_path TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending',UNIQUE(session_id,folder_id)); INSERT INTO scan_queue(session_id,folder_id,name,parent_path) VALUES('old','1','Portal','');").unwrap();
        init_db(&db).unwrap();
        let session=scan_session(&db,"https://portal.example/|u").unwrap().unwrap();
        assert_eq!(session.folders_pending,1);assert_eq!(session.elapsed_seconds,0);
    }

    #[test]
    fn graph_starts_with_parent_folders_and_pages_direct_children() {
        let mut db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();
        let root=Node{id:"folder:1".into(),portal_id:"1".into(),kind:"folder".into(),name:"Portal".into(),path:"Portal".into(),active:true};
        let child=Node{id:"folder:2".into(),portal_id:"2".into(),kind:"folder".into(),name:"Child".into(),path:"Portal/Child".into(),active:true};
        let mut nodes=vec![child.clone()];
        for i in 0..65 { nodes.push(Node{id:format!("file:{i}"),portal_id:i.to_string(),kind:"file".into(),name:format!("Document {i}"),path:format!("Portal/Document {i}"),active:true}); }
        let edges=nodes.iter().map(|n|Edge{source:root.id.clone(),target:n.id.clone()}).collect::<Vec<_>>();
        upsert_page(&mut db,&root,&nodes,&edges).unwrap();
        let roots=graph_roots(&db).unwrap();
        assert_eq!(roots.nodes.len(),1);assert_eq!(roots.nodes[0].id,"folder:1");assert!(roots.edges.is_empty());
        let first=graph_children(&db,"folder:1",0).unwrap();
        assert_eq!(first.nodes.len(),60);assert!(first.has_more);assert_eq!(first.nodes[0].id,"folder:2");
        let second=graph_children(&db,"folder:1",60).unwrap();
        assert_eq!(second.nodes.len(),6);assert!(!second.has_more);
        assert!(graph_children(&db,"file:0",0).is_err());
    }

    #[test]
    fn parses_folder_and_document_links_without_external_links_or_duplicates() {
        let base=Url::parse("https://portal.example/").unwrap();
        let html=r#"<a href="/folder/722">Self</a><a id="folderItem2" href="/folder/800">งานปีนี้</a><span class="docNameList"><a href="/browse/previewDocument/idDocument/143245">รายงาน.pdf</a><a href="/browse/previewDocument/idDocument/143245">duplicate</a></span><a id="folderItem3" href="https://other.example/folder/900">external</a>"#;
        let (nodes,edges)=parse_folder_html(&base,"722",html).unwrap();
        assert_eq!(nodes.len(),2);
        assert_eq!(edges.len(),2);
        assert!(nodes.iter().any(|n| n.id=="folder:800" && n.name=="งานปีนี้"));
        assert!(nodes.iter().any(|n| n.id=="file:143245" && n.name=="รายงาน.pdf"));
    }

    #[test]
    fn login_form_uses_portal_action_and_hidden_csrf_fields() {
        let base=Url::parse("http://portal.example/").unwrap();
        let html=r#"<form action="/" method="post"><input type="hidden" name="CSRFTOKEN" value="one"><input type="text" name="signin[username]"><input type="password" name="signin[password]"><input type="hidden" name="signin[_csrf_token]" value="two"></form>"#;
        let (url,fields)=parse_login_form(&base,html).unwrap();
        assert_eq!(url.as_str(),"http://portal.example/");
        assert_eq!(fields.len(),2);
        assert!(fields.iter().any(|(name,_)|name=="CSRFTOKEN"));
        assert!(fields.iter().any(|(name,_)|name=="signin[_csrf_token]"));
        assert!(parse_login_form(&base,"<form action='https://other.example/'><input name='signin[password]'></form>").is_err());
    }

    #[test]
    fn discovers_root_from_browse_form() {
        assert_eq!(discover_root_id("<form id='form_browse'><input name='folder' value='1'></form>").unwrap(),"1");
        assert!(discover_root_id("<a href='/folder/2'>Favorite</a>").is_err());
    }

    #[test]
    fn follows_only_forward_folder_pagination() {
        let base=Url::parse("https://portal.example/").unwrap();
        let current=base.join("folder/7?itemsPerPage=100").unwrap();
        let html="<a href='/folder/7/1/name/down?count=100'>sort</a><a href='/folder/7/2?count=100'>next</a><a href='/folder/8/3'>other</a>";
        let next=next_folder_page(&base,"7",&current,html).unwrap().unwrap();
        assert_eq!(next.path(),"/folder/7/2");
        assert!(next_folder_page(&base,"7",&next,"<a href='/folder/7/1'>previous</a>").unwrap().is_none());
    }

    #[test]
    fn download_plan_distinguishes_direct_and_subtree_and_deduplicates() {
        let mut db=Connection::open_in_memory().unwrap(); init_db(&db).unwrap();
        let root=Node{id:"folder:1".into(),portal_id:"1".into(),kind:"folder".into(),name:"Root".into(),path:"Root".into(),active:true};
        let child=Node{id:"folder:2".into(),portal_id:"2".into(),kind:"folder".into(),name:"Child".into(),path:"Root/Child".into(),active:true};
        let file=Node{id:"file:3".into(),portal_id:"3".into(),kind:"file".into(),name:"Doc".into(),path:"Root/Doc".into(),active:true};
        upsert_page(&mut db,&root,&[child.clone(),file.clone()],&[Edge{source:root.id.clone(),target:child.id.clone()},Edge{source:root.id.clone(),target:file.id.clone()}]).unwrap();
        upsert_page(&mut db,&child,&[file.clone()],&[Edge{source:"folder:2".into(),target:file.id.clone()}]).unwrap();
        let direct=plan(&db,"folder:1","folder_direct").unwrap();
        let subtree=plan(&db,"folder:1","folder_subtree").unwrap();
        assert_eq!(direct.items.len(),1);
        assert_eq!(subtree.items.len(),1);
        assert_eq!(subtree.duplicate_count,1);
    }

    #[test]
    fn scan_session_keeps_pending_work_and_counts_across_reopen() {
        let path=std::env::temp_dir().join(format!("atlas-session-test-{}.sqlite",Uuid::new_v4()));
        let mut db=Connection::open(&path).unwrap();init_db(&db).unwrap();
        db.execute("INSERT INTO scan_sessions(id,source_key,status) VALUES('run-1','https://portal.example/|u','running')",[]).unwrap();
        db.execute("INSERT INTO scan_queue(session_id,folder_id,name,parent_path) VALUES('run-1','1','Portal','')",[]).unwrap();
        let root=Node{id:"folder:1".into(),portal_id:"1".into(),kind:"folder".into(),name:"Portal".into(),path:"Portal".into(),active:true};
        let folder=Node{id:"folder:2".into(),portal_id:"2".into(),kind:"folder".into(),name:"Child".into(),path:"Portal/Child".into(),active:true};
        let file=Node{id:"file:3".into(),portal_id:"3".into(),kind:"file".into(),name:"Doc".into(),path:"Portal/Doc".into(),active:true};
        let session=commit_scan_page(&mut db,"run-1",&root,&[folder.clone(),file.clone()],&[Edge{source:root.id.clone(),target:folder.id.clone()},Edge{source:root.id.clone(),target:file.id.clone()}]).unwrap();
        assert_eq!((session.folders_done,session.folders_pending,session.files_seen),(1,1,1));
        assert_eq!(scan_next(&db,"run-1").unwrap().unwrap().0,"2");
        drop(db);
        let db=Connection::open(&path).unwrap();init_db(&db).unwrap();
        recover_interrupted(&db).unwrap();
        let restored=scan_session(&db,"https://portal.example/|u").unwrap().unwrap();
        assert_eq!((restored.status.as_str(),restored.folders_pending,restored.files_seen),("paused",1,1));
        drop(db);
        let _=std::fs::remove_file(path);
    }
}
