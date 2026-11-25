use super::*;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

const E5: &str = "multilingual-e5-small";
const BGE: &str = "bge-m3";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagConfig {
    model_id: String,
    model_path: String,
    installed: bool,
    provider: &'static str,
    embedding_provider: String,
    api_url: String,
    api_model: String,
    api_dimensions: i64,
    has_api_key: bool,
    indexed_documents: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagJob {
    id: String, status: String, total: i64, completed: i64, failed: i64,
    current_document: String, last_error: String, updated_at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagDocument {
    node_id: String, name: String, status: String, page_count: i64,
    empty_pages: i64, chunk_count: i64, warning: String, updated_at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagHit {
    pub(crate) chunk_id: i64,
    pub(crate) node_id: String, pub(crate) name: String, pub(crate) path: String, pub(crate) page: i64,
    pub(crate) text: String, pub(crate) distance: f64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagStorage {
    database_path: String,
    pdf_cache_path: String,
    indexed_documents: i64,
    chunks: i64,
}

pub(crate) fn init_rag_db(db: &Connection) -> Result<()> {
    db.execute_batch("
      CREATE TABLE IF NOT EXISTS rag_documents (
        node_id TEXT PRIMARY KEY, sha256 TEXT NOT NULL DEFAULT '',
        status TEXT NOT NULL, page_count INTEGER NOT NULL DEFAULT 0,
        empty_pages INTEGER NOT NULL DEFAULT 0, chunk_count INTEGER NOT NULL DEFAULT 0,
        warning TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      );
      CREATE TABLE IF NOT EXISTS rag_chunks (
        id INTEGER PRIMARY KEY, node_id TEXT NOT NULL, page INTEGER NOT NULL,
        text TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_rag_chunks_node ON rag_chunks(node_id);
      CREATE TABLE IF NOT EXISTS rag_jobs (
        id TEXT PRIMARY KEY, status TEXT NOT NULL, total INTEGER NOT NULL,
        completed INTEGER NOT NULL DEFAULT 0, failed INTEGER NOT NULL DEFAULT 0,
        current_document TEXT NOT NULL DEFAULT '', last_error TEXT NOT NULL DEFAULT '',
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      );
      CREATE TABLE IF NOT EXISTS rag_queue (
        job_id TEXT NOT NULL, node_id TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending',
        PRIMARY KEY(job_id,node_id)
      );
      CREATE INDEX IF NOT EXISTS idx_rag_queue ON rag_queue(job_id,status);
    ").map_err(err)
}

fn model_spec(id: &str) -> Result<(EmbeddingModel, usize)> {
    match id {
        E5 => Ok((EmbeddingModel::MultilingualE5Small, 384)),
        BGE => Ok((EmbeddingModel::BGEM3, 1024)),
        _ => Err("โมเดลไม่อยู่ในรายการที่รองรับ".into()),
    }
}
pub(crate) enum EngineConfig {
    Local { model_id:String, path:PathBuf, dim:usize },
    Api { url:String, model:String, key:String, dim:usize },
}
impl EngineConfig {
    fn dim(&self)->usize {match self {Self::Local{dim,..}|Self::Api{dim,..}=>*dim}}
    pub(crate) fn e5(&self)->bool {matches!(self,Self::Local{model_id,..} if model_id==E5)}
}
fn api_credential(url:&str,model:&str)->Result<keyring::Entry>{
    keyring::Entry::new("document-atlas-rag-api",&format!("{url}|{model}")).map_err(err)
}
pub(crate) fn selected_engine(db:&Connection)->Result<EngineConfig>{
    if setting(db,"rag_embedding_provider")?=="api" {
        let url=setting(db,"rag_api_url")?;let model=setting(db,"rag_api_model")?;
        let dim=setting(db,"rag_api_dimensions")?.parse::<usize>().map_err(|_|"กรุณาตั้งค่า API Embedding ก่อน".to_string())?;
        let key=api_credential(&url,&model)?.get_password().unwrap_or_default();
        Ok(EngineConfig::Api{url,model,key,dim})
    }else{
        let (model_id,path)=selected_model(db)?;
        let dim=model_spec(&model_id)?.1;
        Ok(EngineConfig::Local{model_id,path,dim})
    }
}
fn validated_api_url(value:&str)->Result<String>{
    let url=Url::parse(value.trim()).map_err(err)?;
    if !matches!(url.scheme(),"http"|"https") || url.username()!="" || url.password().is_some() || url.host_str().is_none() || url.fragment().is_some(){
        return Err("API URL ต้องเป็น HTTP/HTTPS และไม่ใส่ข้อมูลล็อกอินใน URL".into());
    }
    Ok(url.to_string())
}
async fn api_embed(url:&str,model:&str,key:&str,inputs:&[String],dim:Option<usize>)->Result<Vec<Vec<f32>>>{
    #[derive(Deserialize)] struct Datum {index:usize,embedding:Vec<f32>}
    #[derive(Deserialize)] struct Response {data:Vec<Datum>}
    let client=Client::builder().timeout(Duration::from_secs(60)).build().map_err(err)?;
    let mut last=String::new();
    for attempt in 0..3 {
        let mut request=client.post(url).json(&serde_json::json!({"model":model,"input":inputs}));
        if !key.is_empty(){request=request.bearer_auth(key);}
        match request.send().await {
            Ok(response) if response.status().is_success()=>{
                let mut rows=response.json::<Response>().await.map_err(err)?.data;
                rows.sort_by_key(|row|row.index);
                if rows.len()!=inputs.len() || rows.iter().enumerate().any(|(i,row)|row.index!=i){return Err("API ส่งจำนวน Embedding หรือ index ไม่ตรงกับคำขอ".into());}
                let vectors:Vec<Vec<f32>>=rows.into_iter().map(|row|row.embedding).collect();
                if vectors.iter().any(|v|v.is_empty()||v.len()>4096||v.iter().any(|n|!n.is_finite())||dim.is_some_and(|d|v.len()!=d)){return Err("API ส่งเวกเตอร์ที่มีมิติไม่ตรงหรือค่าไม่ถูกต้อง".into());}
                return Ok(vectors);
            }
            Ok(response)=>{
                let status=response.status();
                let body=response.text().await.unwrap_or_default();
                last=format!("API HTTP {status}: {}",body.chars().take(250).collect::<String>());
                if status.as_u16()!=429 && !status.is_server_error(){return Err(last);}
            }
            Err(e)=>last=format!("API Embedding: {e}"),
        }
        if attempt<2 {tokio::time::sleep(Duration::from_secs(1<<attempt)).await;}
    }
    Err(last)
}
fn marker(path: &Path, model_id: &str) -> PathBuf { path.join(format!(".document-atlas-{model_id}.installed")) }
fn model(path: PathBuf, model_id: String) -> Result<TextEmbedding> {
    let (kind, _) = model_spec(&model_id)?;
    if let Some(home)=std::env::var_os("HF_HOME") {
        let home=PathBuf::from(home);
        if home.canonicalize().ok()!=path.canonicalize().ok() {
            return Err(format!("HF_HOME ชี้ไปที่ {} ซึ่งต่างจาก Path ที่เลือก กรุณาเลือก Path นี้หรือปรับ HF_HOME",home.display()));
        }
    }
    TextEmbedding::try_new(TextInitOptions::new(kind).with_cache_dir(path).with_intra_threads(2))
        .map_err(|e| format!("โหลดโมเดลไม่ได้: {e}"))
}
fn selected_model(db: &Connection) -> Result<(String, PathBuf)> {
    let id = setting(db, "rag_model_id")?;
    let path = PathBuf::from(setting(db, "rag_model_path")?);
    if id.is_empty() || !marker(&path, &id).is_file() { return Err("กรุณาติดตั้งโมเดล Embedding ในหน้าตั้งค่าก่อน".into()); }
    Ok((id, path))
}
pub(crate) fn vec_blob(values: &[f32]) -> Vec<u8> { values.iter().flat_map(|n| n.to_le_bytes()).collect() }
pub(crate) enum Engine { Local(TextEmbedding), Api {url:String,model:String,key:String,dim:usize} }
impl Engine {
    pub(crate) async fn embed(&mut self, inputs:&[String])->Result<Vec<Vec<f32>>>{
        match self {
            Self::Local(engine)=>engine.embed(inputs,Some(16)).map_err(err),
            Self::Api{url,model,key,dim}=>{
                let mut result=Vec::new();
                for group in inputs.chunks(16){result.extend(api_embed(url,model,key,group,Some(*dim)).await?);}
                Ok(result)
            }
        }
    }
}
pub(crate) async fn start_engine(config:EngineConfig)->Result<Engine>{
    match config {
        EngineConfig::Local{model_id,path,..}=>tokio::task::spawn_blocking(move || model(path,model_id)).await.map_err(err)?.map(Engine::Local),
        EngineConfig::Api{url,model,key,dim}=>Ok(Engine::Api{url,model,key,dim}),
    }
}
fn ensure_vec(db: &Connection, dim: usize) -> Result<()> {
    db.execute_batch(&format!("CREATE VIRTUAL TABLE IF NOT EXISTS rag_vec USING vec0(embedding float[{dim}]);")).map_err(err)
}
fn rag_config(db: &Connection) -> Result<RagConfig> {
    let id = setting(db, "rag_model_id")?;
    let path = setting(db, "rag_model_path")?;
    let indexed_documents = db.query_row("SELECT COUNT(*) FROM rag_documents WHERE status IN ('ready','partial_text')", [], |r| r.get(0)).map_err(err)?;
    let embedding_provider=setting(db,"rag_embedding_provider")?;
    let api_url=setting(db,"rag_api_url")?;let api_model=setting(db,"rag_api_model")?;
    let api_dimensions=setting(db,"rag_api_dimensions")?.parse().unwrap_or(0);
    let has_api_key=if api_url.is_empty()||api_model.is_empty(){false}else{api_credential(&api_url,&api_model)?.get_password().is_ok()};
    let installed=if embedding_provider=="api"{!api_url.is_empty()&&!api_model.is_empty()&&api_dimensions>0}else{!id.is_empty() && marker(Path::new(&path), &id).is_file()};
    Ok(RagConfig { installed, model_id: id, model_path: path, provider: "sqlite-vec",embedding_provider:if embedding_provider=="api"{"api".into()}else{"local".into()},api_url,api_model,api_dimensions,has_api_key,indexed_documents })
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn get_rag_config(state: State<'_, AppState>) -> Result<RagConfig> {
    let db = state.db.lock().map_err(err)?; rag_config(&db)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn install_rag_model(state: State<'_, AppState>, model_id: String, model_path: String) -> Result<RagConfig> {
    model_spec(&model_id)?;
    let path = PathBuf::from(model_path.trim());
    if !path.is_absolute() || !path.is_dir() { return Err("กรุณาเลือกโฟลเดอร์โมเดลที่มีอยู่จริง".into()); }
    if state.rag_pause.lock().map_err(err)?.is_some() { return Err("กรุณาหยุดงานทำดัชนีก่อนเปลี่ยนโมเดล".into()); }
    let workspace={let registry=state.registry.lock().map_err(err)?;active_workspace_id(&registry)?};
    let load_path = path.clone(); let load_id = model_id.clone();
    tokio::task::spawn_blocking(move || model(load_path, load_id))
        .await.map_err(err)??;
    fs::write(marker(&path, &model_id), b"installed").map_err(err)?;
    let registry=state.registry.lock().map_err(err)?;
    if active_workspace_id(&registry)?!=workspace{return Err("พื้นที่สแกนเปลี่ยนระหว่างติดตั้งโมเดล กรุณาเปิดพื้นที่เดิมแล้วเลือกโมเดลอีกครั้ง".into());}
    let mut db = state.db.lock().map_err(err)?;
    let old_id = setting(&db, "rag_model_id")?;
    let old_provider=setting(&db,"rag_embedding_provider")?;
    let tx = db.transaction().map_err(err)?;
    if (!old_id.is_empty() && old_id != model_id) || old_provider=="api" {
        tx.execute_batch("DROP TABLE IF EXISTS rag_vec; DELETE FROM rag_chunks; UPDATE rag_documents SET status='stale',chunk_count=0,warning='เปลี่ยนโมเดลแล้ว กรุณาทำดัชนีใหม่';").map_err(err)?;
    }
    tx.execute("INSERT INTO settings(key,value) VALUES('rag_model_id',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&model_id]).map_err(err)?;
    tx.execute("INSERT INTO settings(key,value) VALUES('rag_model_path',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [path.to_string_lossy().as_ref()]).map_err(err)?;
    tx.execute("INSERT INTO settings(key,value) VALUES('rag_embedding_provider','local') ON CONFLICT(key) DO UPDATE SET value='local'",[]).map_err(err)?;
    tx.commit().map_err(err)?;
    rag_config(&db)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn save_rag_api(state:State<'_,AppState>,api_url:String,api_model:String,api_key:Option<String>)->Result<RagConfig>{
    let url=validated_api_url(&api_url)?;
    let model=api_model.trim().to_string();
    if model.is_empty()||model.len()>160{return Err("กรุณาระบุชื่อโมเดล API ไม่เกิน 160 ตัวอักษร".into());}
    if state.rag_pause.lock().map_err(err)?.is_some(){return Err("กรุณาหยุดงานทำดัชนีก่อนเปลี่ยน API".into());}
    let workspace={let registry=state.registry.lock().map_err(err)?;active_workspace_id(&registry)?};
    let key=api_key.filter(|s|!s.is_empty()).unwrap_or_else(||api_credential(&url,&model).ok().and_then(|e|e.get_password().ok()).unwrap_or_default());
    let vectors=api_embed(&url,&model,&key,&["Document Atlas connection check".into()],None).await?;
    let dim=vectors[0].len();
    let registry=state.registry.lock().map_err(err)?;
    if active_workspace_id(&registry)?!=workspace{return Err("พื้นที่สแกนเปลี่ยนระหว่างทดสอบ API กรุณาเปิดพื้นที่เดิมแล้วบันทึกอีกครั้ง".into());}
    if !key.is_empty(){api_credential(&url,&model)?.set_password(&key).map_err(err)?;}
    let mut db=state.db.lock().map_err(err)?;
    let old_provider=setting(&db,"rag_embedding_provider")?;
    let changed=old_provider!="api" || setting(&db,"rag_api_url")?!=url || setting(&db,"rag_api_model")?!=model || setting(&db,"rag_api_dimensions")?!=dim.to_string();
    let tx=db.transaction().map_err(err)?;
    if changed{tx.execute_batch("DROP TABLE IF EXISTS rag_vec; DELETE FROM rag_chunks; UPDATE rag_documents SET status='stale',chunk_count=0,warning='เปลี่ยน Embedding แล้ว กรุณาทำดัชนีใหม่';").map_err(err)?;}
    for (name,value) in [("rag_embedding_provider","api"),("rag_api_url",url.as_str()),("rag_api_model",model.as_str()),("rag_api_dimensions",&dim.to_string())]{
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![name,value]).map_err(err)?;
    }
    tx.commit().map_err(err)?;
    rag_config(&db)
}
fn row_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<RagJob> {
    Ok(RagJob { id:r.get(0)?,status:r.get(1)?,total:r.get(2)?,completed:r.get(3)?,failed:r.get(4)?,current_document:r.get(5)?,last_error:r.get(6)?,updated_at:r.get(7)? })
}
fn row_hit(r:&rusqlite::Row<'_>)->rusqlite::Result<RagHit>{
    Ok(RagHit {chunk_id:r.get(0)?,node_id:r.get(1)?,name:r.get(2)?,path:r.get(3)?,page:r.get(4)?,text:r.get(5)?,distance:r.get(6)?})
}
fn latest_job(db: &Connection) -> Result<Option<RagJob>> {
    db.query_row("SELECT id,status,total,completed,failed,current_document,last_error,updated_at FROM rag_jobs ORDER BY rowid DESC LIMIT 1", [], row_job).optional().map_err(err)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn get_rag_job(state: State<'_, AppState>) -> Result<Option<RagJob>> {
    let db=state.db.lock().map_err(err)?;latest_job(&db)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn list_rag_documents(state: State<'_, AppState>) -> Result<Vec<RagDocument>> {
    let db=state.db.lock().map_err(err)?;
    let mut stmt=db.prepare("SELECT d.node_id,COALESCE(n.name,d.node_id),d.status,d.page_count,d.empty_pages,d.chunk_count,d.warning,d.updated_at FROM rag_documents d LEFT JOIN nodes n ON n.id=d.node_id ORDER BY d.updated_at DESC LIMIT 200").map_err(err)?;
    let rows=stmt.query_map([],|r|Ok(RagDocument {node_id:r.get(0)?,name:r.get(1)?,status:r.get(2)?,page_count:r.get(3)?,empty_pages:r.get(4)?,chunk_count:r.get(5)?,warning:r.get(6)?,updated_at:r.get(7)?}))
        .map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    Ok(rows)
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagPageSummary {
    page: i64, chunk_count: i64, character_count: i64, vector_count: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagDocumentDetail {
    #[serde(flatten)]
    document: RagDocument,
    path: String, sha256: String, vector_count: i64, pages: Vec<RagPageSummary>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagStoredChunk {
    chunk_id: i64, page: i64, text: String, has_embedding: bool,
}
fn has_rag_vectors(db: &Connection) -> Result<bool> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='rag_vec')", [], |r| r.get(0)).map_err(err)
}
pub(crate) fn document_detail(db: &Connection, node_id: &str) -> Result<RagDocumentDetail> {
    let mut detail = db.query_row(
        "SELECT d.node_id,COALESCE(n.name,d.node_id),d.status,d.page_count,d.empty_pages,d.chunk_count,d.warning,d.updated_at,COALESCE(n.path,''),d.sha256 FROM rag_documents d LEFT JOIN nodes n ON n.id=d.node_id WHERE d.node_id=?1",
        [node_id], |r| Ok(RagDocumentDetail {
            document: RagDocument { node_id:r.get(0)?, name:r.get(1)?, status:r.get(2)?, page_count:r.get(3)?, empty_pages:r.get(4)?, chunk_count:r.get(5)?, warning:r.get(6)?, updated_at:r.get(7)? },
            path:r.get(8)?, sha256:r.get(9)?, vector_count:0, pages:Vec::new(),
        }),
    ).optional().map_err(err)?.ok_or("เอกสารนี้ยังไม่มีข้อมูล RAG ในพื้นที่สแกนนี้ กรุณาทำดัชนีก่อน")?;
    let vector_sql = if has_rag_vectors(db)? { "EXISTS(SELECT 1 FROM rag_vec v WHERE v.rowid=c.id)" } else { "0" };
    let sql = format!("SELECT c.page,COUNT(*),SUM(length(c.text)),SUM({vector_sql}) FROM rag_chunks c WHERE c.node_id=?1 GROUP BY c.page ORDER BY c.page");
    let mut stmt = db.prepare(&sql).map_err(err)?;
    let summaries = stmt.query_map([node_id], |r| Ok(RagPageSummary { page:r.get(0)?, chunk_count:r.get(1)?, character_count:r.get(2)?, vector_count:r.get(3)? }))
        .map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    detail.vector_count = summaries.iter().map(|p|p.vector_count).sum();
    let mut summaries = summaries.into_iter().peekable();
    for page in 1..=detail.document.page_count {
        while summaries.peek().is_some_and(|p|p.page<page) { summaries.next(); }
        let summary = if summaries.peek().is_some_and(|p|p.page==page) { summaries.next().unwrap() }
            else { RagPageSummary { page, chunk_count:0, character_count:0, vector_count:0 } };
        detail.pages.push(summary);
    }
    Ok(detail)
}
pub(crate) fn document_page(db: &Connection, node_id: &str, page: i64) -> Result<Vec<RagStoredChunk>> {
    let count: Option<i64> = db.query_row("SELECT page_count FROM rag_documents WHERE node_id=?1", [node_id], |r|r.get(0)).optional().map_err(err)?;
    let count = count.ok_or("เอกสารนี้ยังไม่มีข้อมูล RAG ในพื้นที่สแกนนี้")?;
    if page<1 || page>count { return Err("เลขหน้าอยู่นอกขอบเขตของ PDF ที่ทำดัชนี".into()); }
    let vector_sql = if has_rag_vectors(db)? { "EXISTS(SELECT 1 FROM rag_vec v WHERE v.rowid=c.id)" } else { "0" };
    let sql = format!("SELECT c.id,c.page,c.text,{vector_sql} FROM rag_chunks c WHERE c.node_id=?1 AND c.page=?2 ORDER BY c.id");
    let mut stmt = db.prepare(&sql).map_err(err)?;
    let chunks = stmt.query_map(params![node_id,page], |r|Ok(RagStoredChunk { chunk_id:r.get(0)?, page:r.get(1)?, text:r.get(2)?, has_embedding:r.get(3)? }))
        .map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    Ok(chunks)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn get_rag_document_detail(state: State<'_, AppState>, node_id: String) -> Result<RagDocumentDetail> {
    let db=state.db.lock().map_err(err)?; document_detail(&db,&node_id)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn get_rag_document_page(state: State<'_, AppState>, node_id: String, page: i64) -> Result<Vec<RagStoredChunk>> {
    let db=state.db.lock().map_err(err)?; document_page(&db,&node_id,page)
}

fn chunks_for_page(text: &str, page: usize) -> Vec<(i64,String)> {
    let chars: Vec<char> = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    let mut result=Vec::new(); let mut start=0;
    while start<chars.len() {
        let end=(start+900).min(chars.len());
        let part: String=chars[start..end].iter().collect();
        if part.trim().chars().count()>=20 { result.push((page as i64,part)); }
        if end==chars.len(){break;}
        start=end.saturating_sub(100);
    }
    result
}
pub(crate) fn extract_pdf(bytes: &[u8]) -> Result<(usize,usize,Vec<(i64,String)>)> {
    let document=lopdf::Document::load_mem(bytes).map_err(|e|format!("อ่านโครงสร้าง PDF ไม่ได้: {e}"))?;
    let page_count=document.get_pages().len();
    if page_count==0 { return Err("PDF ไม่มีหน้า".into()); }
    // pdf-extract panics on some malformed Portal PDFs (index out of bounds / unwrap on None); lopdf's extractor is the fallback.
    let pages=first_text(vec![
        Box::new(||pdf_extract::extract_text_from_mem_by_pages(bytes).map_err(|e|e.to_string())),
        Box::new(||Ok((1..=page_count as u32).map(|p|document.extract_text(&[p]).unwrap_or_default()).collect())),
    ])?;
    let mut chunks=Vec::new();let mut empty=0;
    for page in 1..=page_count {
        let content=pages.get(page-1).map(String::as_str).unwrap_or("");
        if content.trim().chars().count()<20 { empty+=1;continue; }
        chunks.extend(chunks_for_page(content,page));
    }
    Ok((page_count,empty,chunks))
}
type PageTexts<'a>=Box<dyn Fn()->std::result::Result<Vec<String>,String>+'a>;
/// Page texts from the first extractor that yields text, surviving extractor panics.
/// A PDF that every extractor reads without text (image-only) returns empty pages so it is flagged for OCR.
fn first_text(extractors:Vec<PageTexts<'_>>)->Result<Vec<String>>{
    let mut errors=Vec::new();let mut textless=None;
    for extract in extractors {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(||extract())) {
            Ok(Ok(pages)) if pages.iter().any(|p|p.trim().chars().count()>=20)=>return Ok(pages),
            Ok(Ok(pages))=>{textless.get_or_insert(pages);}
            Ok(Err(e))=>errors.push(e),
            Err(panic)=>errors.push(panic.downcast_ref::<String>().cloned().or_else(||panic.downcast_ref::<&str>().map(|s|s.to_string())).unwrap_or_else(||"panic".into())),
        }
    }
    textless.ok_or_else(||format!("ดึงข้อความ PDF ไม่ได้ (โครงสร้างไฟล์ผิดรูป ต้องให้ AI อ่านภาพแทน): {}",errors.join(" / ")))
}
fn save_document(db: &mut Connection, item: &Node, bytes: &[u8], embeddings: Vec<Vec<f32>>, pages: usize, empty: usize, chunks: Vec<(i64,String)>) -> Result<()> {
    if embeddings.len()!=chunks.len() {return Err("จำนวนเวกเตอร์ไม่ตรงกับข้อความ".into());}
    let sha=hex::encode(Sha256::digest(bytes));
    let status=if chunks.is_empty() {"needs_ocr"} else if empty>0 {"partial_text"} else {"ready"};
    let warning=if chunks.is_empty() {"PDF ไม่มี text layer หรือเป็นภาพ ต้องใช้ OCR".to_string()} else if empty>0 {format!("มี {empty} หน้าที่อ่านข้อความไม่ได้ อาจเป็นภาพหรือไม่มี text layer")} else {String::new()};
    let tx=db.transaction().map_err(err)?;
    let mut old=tx.prepare("SELECT id FROM rag_chunks WHERE node_id=?1").map_err(err)?;
    let ids=old.query_map([&item.id],|r|r.get::<_,i64>(0)).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    drop(old);
    for id in ids {tx.execute("DELETE FROM rag_vec WHERE rowid=?1",[id]).map_err(err)?;}
    tx.execute("DELETE FROM rag_chunks WHERE node_id=?1",[&item.id]).map_err(err)?;
    for ((page,text),embedding) in chunks.iter().zip(embeddings.iter()) {
        tx.execute("INSERT INTO rag_chunks(node_id,page,text) VALUES(?1,?2,?3)",params![item.id,page,text]).map_err(err)?;
        let id=tx.last_insert_rowid();
        tx.execute("INSERT INTO rag_vec(rowid,embedding) VALUES(?1,?2)",params![id,vec_blob(embedding)]).map_err(err)?;
    }
    tx.execute("INSERT INTO rag_documents(node_id,sha256,status,page_count,empty_pages,chunk_count,warning,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,CURRENT_TIMESTAMP) ON CONFLICT(node_id) DO UPDATE SET sha256=excluded.sha256,status=excluded.status,page_count=excluded.page_count,empty_pages=excluded.empty_pages,chunk_count=excluded.chunk_count,warning=excluded.warning,updated_at=CURRENT_TIMESTAMP",
        params![item.id,sha,status,pages as i64,empty as i64,chunks.len() as i64,warning]).map_err(err)?;
    tx.commit().map_err(err)
}
#[cfg(feature = "desktop")]
async fn index_worker(app: AppHandle, job_id: String, flag: Arc<AtomicBool>, profile: Profile, config: EngineConfig) {
    let state=app.state::<AppState>();
    let portal=match Portal::connect(&profile).await {Ok(p)=>p,Err(e)=>{
        if let Ok(db)=state.db.lock(){let _=db.execute("UPDATE rag_jobs SET status='error',last_error=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![job_id,e]);}
        if let Ok(mut running)=state.rag_pause.lock(){*running=None;}
        return;
    }};
    let is_e5=config.e5();
    let engine=start_engine(config).await;
    let mut engine=match engine {Ok(m)=>m,Err(e)=>{
        if let Ok(db)=state.db.lock(){let _=db.execute("UPDATE rag_jobs SET status='error',last_error=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![job_id,e]);}
        if let Ok(mut running)=state.rag_pause.lock(){*running=None;} return;
    }};
    loop {
        if flag.load(Ordering::Relaxed) {break;}
        let next={let db=match state.db.lock(){Ok(d)=>d,Err(_)=>break};
            db.query_row("SELECT q.node_id FROM rag_queue q WHERE q.job_id=?1 AND q.status='pending' LIMIT 1",[&job_id],|r|r.get::<_,String>(0)).optional().ok().flatten()};
        let Some(node_id)=next else {break};
        let item={let db=match state.db.lock(){Ok(d)=>d,Err(_)=>break};node(&db,&node_id)};
        let result=if let Ok(ref item)=item {
            if let Ok(db)=state.db.lock(){let _=db.execute("UPDATE rag_jobs SET current_document=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![job_id,item.name]);}
            {
                    let mut fetched=Err("ดาวน์โหลด PDF ไม่สำเร็จ".to_string());
                    for attempt in 0..3 { if flag.load(Ordering::Relaxed){break;} fetched=portal.document(&item.portal_id).await; if fetched.is_ok(){break;} if attempt<2 {tokio::time::sleep(Duration::from_secs(1<<attempt)).await;} }
                    match fetched {
                        Ok(bytes)=>{
                            let parsed=tokio::task::spawn_blocking(move || {let outcome=extract_pdf(&bytes);(bytes,outcome)}).await.map_err(err);
                            match parsed {
                                Ok((bytes,Ok((pages,empty,chunks))))=>{
                                    let prefixed:Vec<String>=chunks.iter().map(|(_,text)|if is_e5 {format!("passage: {text}")}else{text.clone()}).collect();
                                    let vectors=if prefixed.is_empty(){Ok(Vec::new())}else{engine.embed(&prefixed).await};
                                    vectors.and_then(|vectors|{let mut db=state.db.lock().map_err(err)?;save_document(&mut db,item,&bytes,vectors,pages,empty,chunks)})
                                }
                                Ok((_,Err(e)))=>Err(e),Err(e)=>Err(e)
                            }
                        }
                        Err(e)=>Err(e)
                    }
            }
        } else {Err(item.err().unwrap())};
        if flag.load(Ordering::Relaxed) && result.is_err() {break;}
        if let Ok(db)=state.db.lock(){
            match result {
                Ok(())=>{let _=db.execute("UPDATE rag_queue SET status='done' WHERE job_id=?1 AND node_id=?2",params![job_id,node_id]);let _=db.execute("UPDATE rag_jobs SET completed=completed+1,updated_at=CURRENT_TIMESTAMP WHERE id=?1",[&job_id]);}
                Err(e)=>{let _=db.execute("UPDATE rag_queue SET status='failed' WHERE job_id=?1 AND node_id=?2",params![job_id,node_id]);let _=db.execute("UPDATE rag_jobs SET failed=failed+1,last_error=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![job_id,e]);let _=db.execute("INSERT INTO rag_documents(node_id,status,warning) VALUES(?1,'error',?2) ON CONFLICT(node_id) DO UPDATE SET warning=excluded.warning,updated_at=CURRENT_TIMESTAMP",params![node_id,e]);}
            }
        }
    }
    if let Ok(db)=state.db.lock(){
        let failed=db.query_row("SELECT failed FROM rag_jobs WHERE id=?1",[&job_id],|r|r.get::<_,i64>(0)).unwrap_or(0);
        let status=if flag.load(Ordering::Relaxed){"paused"}else if failed>0{"completed_with_errors"}else{"completed"};
        let _=db.execute("UPDATE rag_jobs SET status=?2,current_document='',updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![job_id,status]);
    }
    if let Ok(mut running)=state.rag_pause.lock(){*running=None;};
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn start_rag_index(app:AppHandle,state:State<'_,AppState>,node_id:String,scope:String)->Result<RagJob>{
    let planned={let db=state.db.lock().map_err(err)?;plan(&db,&node_id,&scope)?};
    launch_index(app,&state,planned.items)
}
#[cfg(feature = "desktop")]
pub(crate) fn launch_index(app:AppHandle,state:&AppState,items:Vec<Node>)->Result<RagJob>{
    let mut running=state.rag_pause.lock().map_err(err)?;
    if running.is_some(){return Err("งานทำดัชนี RAG กำลังทำงานอยู่".into());}
    let (profile,config)={let db=state.db.lock().map_err(err)?;(profile(&db)?,selected_engine(&db)?)};
    if items.is_empty(){return Err("ไม่มีเอกสารในขอบเขตที่เลือก".into());}
    let dim=config.dim();
    let job_id=Uuid::new_v4().to_string();
    {let mut db=state.db.lock().map_err(err)?;ensure_vec(&db,dim)?;let tx=db.transaction().map_err(err)?;
        tx.execute("INSERT INTO rag_jobs(id,status,total) VALUES(?1,'running',?2)",params![job_id,items.len()]).map_err(err)?;
        for item in &items {tx.execute("INSERT INTO rag_queue(job_id,node_id) VALUES(?1,?2)",params![job_id,item.id]).map_err(err)?;}tx.commit().map_err(err)?;}
    let flag=Arc::new(AtomicBool::new(false));*running=Some(flag.clone());drop(running);
    let job={let db=state.db.lock().map_err(err)?;latest_job(&db)?.ok_or("ไม่พบงาน RAG")?};
    tauri::async_runtime::spawn(index_worker(app,job_id,flag,profile,config));
    Ok(job)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn pause_rag_index(state:State<'_,AppState>)->Result<()> {
    let running=state.rag_pause.lock().map_err(err)?;
    let flag=running.as_ref().ok_or("ไม่มีงานทำดัชนีที่กำลังทำงาน")?;
    flag.store(true,Ordering::Relaxed);Ok(())
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn resume_rag_index(app:AppHandle,state:State<'_,AppState>,job_id:String)->Result<RagJob>{
    let mut running=state.rag_pause.lock().map_err(err)?;
    if running.is_some(){return Err("มีงานทำดัชนี RAG กำลังทำงาน".into());}
    let (profile,config,job)={let db=state.db.lock().map_err(err)?;let config=selected_engine(&db)?;
        let job=db.query_row("SELECT id,status,total,completed,failed,current_document,last_error,updated_at FROM rag_jobs WHERE id=?1",[&job_id],row_job).map_err(err)?;
        (profile(&db)?,config,job)};
    if !matches!(job.status.as_str(),"paused"|"error"|"completed_with_errors"){return Err("งานนี้ไม่อยู่ในสถานะที่ทำต่อได้".into());}
    {let mut db=state.db.lock().map_err(err)?;retry_failed_rag_queue(&mut db,&job_id)?;}
    let flag=Arc::new(AtomicBool::new(false));*running=Some(flag.clone());drop(running);
    tauri::async_runtime::spawn(index_worker(app,job_id.clone(),flag,profile,config));
    let db=state.db.lock().map_err(err)?;
    db.query_row("SELECT id,status,total,completed,failed,current_document,last_error,updated_at FROM rag_jobs WHERE id=?1",[&job_id],row_job).map_err(err)
}
fn retry_failed_rag_queue(db:&mut Connection,job_id:&str)->Result<()> {
    let tx=db.transaction().map_err(err)?;
    tx.execute("UPDATE rag_queue SET status='pending' WHERE job_id=?1 AND status='failed'",[job_id]).map_err(err)?;
    tx.execute("UPDATE rag_jobs SET status='running',failed=0,last_error='',updated_at=CURRENT_TIMESTAMP WHERE id=?1",[job_id]).map_err(err)?;
    tx.commit().map_err(err)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn rag_search(state:State<'_,AppState>,query:String)->Result<Vec<RagHit>>{
    search_hits(state.inner(), &query).await
}
pub(crate) async fn search_hits(state:&AppState,query:&str)->Result<Vec<RagHit>>{
    let query=query.trim();
    if query.is_empty(){return Ok(Vec::new());}
    if query.chars().count()>2000{return Err("คำค้นยาวเกิน 2,000 ตัวอักษร".into());}
    let (workspace,config)={let registry=state.registry.lock().map_err(err)?;let workspace=active_workspace_id(&registry)?;let db=state.db.lock().map_err(err)?;(workspace,selected_engine(&db)?)};
    let is_e5=config.e5();
    let text=if is_e5 {format!("query: {query}")}else{query.to_string()};
    let engine = state.web_engine.get_or_try_init(|| async { start_engine(config).await.map(tokio::sync::Mutex::new) }).await?;
    let vector=engine.lock().await.embed(&[text]).await?.into_iter().next().ok_or_else(|| "โมเดลไม่ส่งเวกเตอร์กลับ".to_string())?;
    let registry=state.registry.lock().map_err(err)?;
    if active_workspace_id(&registry)?!=workspace{return Err("พื้นที่สแกนเปลี่ยนระหว่างค้นหา กรุณาค้นหาใหม่".into());}
    let db=state.db.lock().map_err(err)?;
    let mut stmt=match db.prepare("SELECT c.id,c.node_id,n.name,n.path,c.page,c.text,v.distance FROM (SELECT rowid,distance FROM rag_vec WHERE embedding MATCH ?1 AND k=20 ORDER BY distance) v JOIN rag_chunks c ON c.id=v.rowid JOIN nodes n ON n.id=c.node_id ORDER BY v.distance"){
        Ok(s)=>s,Err(e) if e.to_string().contains("no such table")=>return Ok(Vec::new()),Err(e)=>return Err(err(e))};
    let rows=stmt.query_map([vec_blob(&vector)],row_hit)
        .map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
    Ok(rows)
}

pub(crate) fn related_hits(db:&Connection,chunk_id:i64)->Result<Vec<RagHit>>{
    let source:String=db.query_row("SELECT node_id FROM rag_chunks WHERE id=?1",[chunk_id],|r|r.get(0))
        .map_err(|_|"ไม่พบข้อความที่เลือกในดัชนี".to_string())?;
    let vector:Vec<u8>=db.query_row("SELECT embedding FROM rag_vec WHERE rowid=?1",[chunk_id],|r|r.get(0)).map_err(err)?;
    let mut stmt=db.prepare("SELECT c.id,c.node_id,n.name,n.path,c.page,c.text,vec_distance_l2(v.embedding,?1) AS distance FROM rag_chunks c JOIN rag_vec v ON v.rowid=c.id JOIN nodes n ON n.id=c.node_id WHERE c.node_id<>?2 ORDER BY distance").map_err(err)?;
    let mut rows=stmt.query(params![vector,source]).map_err(err)?;
    let mut seen=HashSet::new();let mut hits=Vec::new();
    while let Some(row)=rows.next().map_err(err)?{
        let hit=row_hit(row).map_err(err)?;
        if seen.insert(hit.node_id.clone()){hits.push(hit);if hits.len()==6{break;}}
    }
    Ok(hits)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn rag_related(state:State<'_,AppState>,chunk_id:i64)->Result<Vec<RagHit>>{
    let db=state.db.lock().map_err(err)?;
    related_hits(&db,chunk_id)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn get_rag_storage(state:State<'_,AppState>)->Result<RagStorage>{
    let registry=state.registry.lock().map_err(err)?;
    let workspace=active_workspace_id(&registry)?;
    let filename:String=registry.query_row("SELECT db_file FROM workspaces WHERE id=?1",[&workspace],|r|r.get(0)).map_err(err)?;
    let db=state.db.lock().map_err(err)?;
    let indexed_documents=db.query_row("SELECT COUNT(*) FROM rag_documents WHERE status IN ('ready','partial_text')",[],|r|r.get(0)).map_err(err)?;
    let chunks=db.query_row("SELECT COUNT(*) FROM rag_chunks",[],|r|r.get(0)).map_err(err)?;
    Ok(RagStorage{database_path:state.data_dir.join(filename).to_string_lossy().to_string(),
        pdf_cache_path:state.data_dir.join("rag-pdf-cache").join(workspace).to_string_lossy().to_string(),
        indexed_documents,chunks})
}
pub(crate) async fn pdf_matches(path:&Path,expected:&str)->bool{
    let Ok(meta)=tokio::fs::metadata(path).await else{return false};
    if meta.len()>100*1024*1024{return false;}
    let Ok(bytes)=tokio::fs::read(path).await else{return false};
    bytes.starts_with(b"%PDF-") && hex::encode(Sha256::digest(&bytes))==expected
}
pub(crate) async fn resolve_rag_document(state:&AppState,node_id:&str)->Result<PathBuf>{
    let (workspace,item,expected,p,downloaded)={
        let registry=state.registry.lock().map_err(err)?;
        let workspace=active_workspace_id(&registry)?;
        let db=state.db.lock().map_err(err)?;
        let item=node(&db,&node_id)?;
        if item.kind!="file" || !item.active{return Err("โหนดนี้ไม่ใช่เอกสาร PDF".into());}
        let expected:String=db.query_row("SELECT sha256 FROM rag_documents WHERE node_id=?1",[&node_id],|r|r.get(0)).map_err(|_|"เอกสารนี้ยังไม่ได้ทำดัชนี PDF".to_string())?;
        if expected.len()!=64 || !expected.bytes().all(|c|c.is_ascii_hexdigit()){return Err("ไม่มี SHA-256 ของ PDF ที่ทำดัชนี".into());}
        let downloaded:Option<String>=db.query_row("SELECT file_path FROM job_items WHERE node_id=?1 AND sha256=?2 AND status IN ('completed','skipped') AND file_path IS NOT NULL ORDER BY rowid DESC LIMIT 1",params![&node_id,&expected],|r|r.get(0)).optional().map_err(err)?;
        (workspace,item,expected,profile(&db)?,downloaded)
    };
    let cache_dir=state.data_dir.join("rag-pdf-cache").join(&workspace);
    let cache=cache_dir.join(format!("document-{}-{}.pdf",item.portal_id,&expected[..16]));
    let path=if let Some(downloaded)=downloaded.filter(|path|!path.is_empty()){
        let source=PathBuf::from(downloaded);
        if pdf_matches(&source,&expected).await{source}
        else if pdf_matches(&cache,&expected).await{cache.clone()}
        else{fetch_rag_pdf(&p,&item,&expected,&cache).await?}
    }else if pdf_matches(&cache,&expected).await{cache.clone()}
    else{fetch_rag_pdf(&p,&item,&expected,&cache).await?};
    let registry=state.registry.lock().map_err(err)?;
    if active_workspace_id(&registry)?!=workspace{return Err("พื้นที่สแกนเปลี่ยนระหว่างเปิด PDF กรุณาลองอีกครั้ง".into());}
    Ok(path)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn read_rag_document(state:State<'_,AppState>,node_id:String)->Result<tauri::ipc::Response>{
    let path=resolve_rag_document(&state,&node_id).await?;
    let bytes=tokio::fs::read(path).await.map_err(err)?;
    // Verify the exact bytes returned, including against concurrent local changes.
    let db=state.db.lock().map_err(err)?;
    let expected:String=db.query_row("SELECT sha256 FROM rag_documents WHERE node_id=?1",[node_id],|r|r.get(0)).map_err(err)?;
    if !bytes.starts_with(b"%PDF-") || hex::encode(Sha256::digest(&bytes))!=expected{return Err("PDF เปลี่ยนจากฉบับที่ทำดัชนี กรุณาทำดัชนีใหม่".into());}
    Ok(tauri::ipc::Response::new(bytes))
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn open_rag_document(state:State<'_,AppState>,node_id:String)->Result<String>{
    let path=resolve_rag_document(&state,&node_id).await?;
    tauri_plugin_opener::open_path(&path,None::<&str>).map_err(err)?;
    Ok(path.to_string_lossy().to_string())
}
async fn fetch_rag_pdf(profile:&Profile,item:&Node,expected:&str,cache:&Path)->Result<PathBuf>{
    let portal=Portal::connect(profile).await?;
    let bytes=portal.document(&item.portal_id).await?;
    if hex::encode(Sha256::digest(&bytes))!=expected{
        return Err("PDF ใน Portal เปลี่ยนจากฉบับที่ทำดัชนี กรุณาทำดัชนีเอกสารนี้ใหม่ก่อนเปิด".into());
    }
    let parent=cache.parent().ok_or("ตำแหน่งแคช PDF ไม่ถูกต้อง")?;
    tokio::fs::create_dir_all(parent).await.map_err(err)?;
    let partial=cache.with_extension(format!("pdf.{}.part",Uuid::new_v4()));
    tokio::fs::write(&partial,&bytes).await.map_err(err)?;
    if cache.exists(){
        if pdf_matches(cache,expected).await {tokio::fs::remove_file(&partial).await.map_err(err)?;return Ok(cache.to_path_buf());}
        tokio::fs::remove_file(cache).await.map_err(err)?;
    }
    tokio::fs::rename(&partial,cache).await.map_err(err)?;
    Ok(cache.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, content::{Content,Operation}, Object, Stream};

    #[test]
    fn inspector_reads_exact_pages_and_excludes_other_documents() {
        unsafe {rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ())));}
        let db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();ensure_vec(&db,3).unwrap();
        db.execute("INSERT INTO nodes(id,portal_id,kind,name,path) VALUES('file:1','1','file','เอกสารไทย','Portal/QA')",[]).unwrap();
        db.execute("INSERT INTO rag_documents(node_id,status,page_count,empty_pages,chunk_count,sha256) VALUES('file:1','partial_text',3,1,2,'abc')",[]).unwrap();
        for (id,node,page,text) in [(1,"file:1",1,"ภาษาไทย 123"),(2,"file:1",3,"หน้าสาม"),(3,"file:2",1,"other document")] {
            db.execute("INSERT INTO rag_chunks(id,node_id,page,text) VALUES(?1,?2,?3,?4)",params![id,node,page,text]).unwrap();
        }
        db.execute("INSERT INTO rag_vec(rowid,embedding) VALUES(1,?1)",[vec_blob(&[1.0,0.0,0.0])]).unwrap();
        let detail=document_detail(&db,"file:1").unwrap();
        assert_eq!(detail.document.name,"เอกสารไทย");assert_eq!(detail.path,"Portal/QA");
        assert_eq!(detail.pages.iter().map(|p|(p.page,p.chunk_count,p.vector_count)).collect::<Vec<_>>(),vec![(1,1,1),(2,0,0),(3,1,0)]);
        assert_eq!(detail.vector_count,1);
        let chunks=document_page(&db,"file:1",1).unwrap();
        assert_eq!(chunks.len(),1);assert_eq!(chunks[0].text,"ภาษาไทย 123");assert!(chunks[0].has_embedding);
        assert!(document_page(&db,"file:1",2).unwrap().is_empty());
        assert!(!document_page(&db,"file:1",3).unwrap()[0].has_embedding);
        assert!(document_page(&db,"file:1",0).is_err());assert!(document_page(&db,"file:1",4).is_err());
        assert!(document_detail(&db,"missing").is_err());assert!(document_page(&db,"missing",1).is_err());
    }
    #[test]
    fn inspector_handles_missing_vector_table_and_ocr_only_documents() {
        let db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();
        db.execute("INSERT INTO rag_documents(node_id,status,page_count,empty_pages,chunk_count) VALUES('file:1','needs_ocr',2,2,0)",[]).unwrap();
        let detail=document_detail(&db,"file:1").unwrap();
        assert_eq!(detail.pages.len(),2);assert_eq!(detail.vector_count,0);
        assert!(document_page(&db,"file:1",1).unwrap().is_empty());
        db.execute("INSERT INTO rag_chunks(node_id,page,text) VALUES('file:1',1,'stored without vector')",[]).unwrap();
        let chunks=document_page(&db,"file:1",1).unwrap();assert_eq!(chunks.len(),1);assert!(!chunks[0].has_embedding);
    }
    #[test]
    fn chunks_preserve_unicode_and_overlap() {
        let text="ภาษาไทย ".repeat(250);
        let chunks=chunks_for_page(&text,3);
        assert!(chunks.len()>1);
        assert!(chunks.iter().all(|(page,part)|*page==3&&part.chars().count()<=900));
        let tail:Vec<char>=chunks[0].1.chars().rev().take(100).collect();
        assert!(tail.into_iter().rev().eq(chunks[1].1.chars().take(100)));
    }
    #[test]
    fn extractor_panic_falls_back_to_next_extractor() {
        let text="ข้อความในเอกสารที่อ่านได้จากตัวอ่านสำรอง".to_string();
        let pages=first_text(vec![Box::new(||panic!("index out of bounds: the len is 0 but the index is 0")),Box::new(||Ok(vec![text.clone()]))]).unwrap();
        assert_eq!(pages,vec![text]);
        let empty=first_text(vec![Box::new(||Ok(vec![String::new()])),Box::new(||Err("broken".into()))]).unwrap();
        assert_eq!(empty,vec![String::new()],"image-only PDFs stay readable as empty pages");
        let err=first_text(vec![Box::new(||panic!("called `Option::unwrap()` on a `None` value")),Box::new(||Err("broken".into()))]).unwrap_err();
        assert!(err.contains("Option::unwrap") && err.contains("broken"));
    }
    #[test]
    fn image_only_pdf_is_flagged_without_chunks() {
        let mut pdf=lopdf::Document::with_version("1.5");
        let pages_id=pdf.new_object_id();
        let content_id=pdf.add_object(Stream::new(dictionary! {},Vec::new()));
        let page_id=pdf.add_object(dictionary! {"Type"=>"Page","Parent"=>pages_id,"Contents"=>content_id,"MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()]});
        pdf.objects.insert(pages_id,dictionary! {"Type"=>"Pages","Kids"=>vec![page_id.into()],"Count"=>1}.into());
        let catalog_id=pdf.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages_id});
        pdf.trailer.set("Root",catalog_id);
        let mut bytes=Vec::new();pdf.save_to(&mut bytes).unwrap();
        let (pages,empty,chunks)=extract_pdf(&bytes).unwrap();
        assert_eq!((pages,empty,chunks.len()),(1,1,0));
    }
    #[test]
    fn text_pdf_preserves_page_reference() {
        let mut pdf=lopdf::Document::with_version("1.5");
        let pages_id=pdf.new_object_id();
        let font_id=pdf.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
        let resources=pdf.add_object(dictionary! {"Font"=>dictionary! {"F1"=>font_id}});
        let content=Content{operations:vec![
            Operation::new("BT",vec![]),Operation::new("Tf",vec!["F1".into(),12.into()]),
            Operation::new("Td",vec![50.into(),700.into()]),
            Operation::new("Tj",vec![Object::string_literal("Document Atlas searchable text example")]),
            Operation::new("ET",vec![])
        ]};
        let content_id=pdf.add_object(Stream::new(dictionary! {},content.encode().unwrap()));
        let page_id=pdf.add_object(dictionary! {"Type"=>"Page","Parent"=>pages_id,"Contents"=>content_id,"Resources"=>resources,"MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()]});
        pdf.objects.insert(pages_id,dictionary! {"Type"=>"Pages","Kids"=>vec![page_id.into()],"Count"=>1}.into());
        let catalog_id=pdf.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages_id});
        pdf.trailer.set("Root",catalog_id);
        let mut bytes=Vec::new();pdf.save_to(&mut bytes).unwrap();
        let (pages,empty,chunks)=extract_pdf(&bytes).unwrap();
        assert_eq!((pages,empty),(1,0));
        assert_eq!(chunks[0].0,1);
        assert!(chunks[0].1.contains("Document Atlas searchable"));
    }
    #[test]
    fn sqlite_vec_returns_nearest_chunk() {
        unsafe {rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ())));}
        let db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();
        db.execute("INSERT INTO nodes(id,portal_id,kind,name,path) VALUES('file:1','1','file','A','/A')",[]).unwrap();
        db.execute("INSERT INTO rag_chunks(id,node_id,page,text) VALUES(1,'file:1',2,'first')",[]).unwrap();
        db.execute("INSERT INTO rag_chunks(id,node_id,page,text) VALUES(2,'file:1',3,'second')",[]).unwrap();
        ensure_vec(&db,3).unwrap();
        db.execute("INSERT INTO rag_vec(rowid,embedding) VALUES(1,?1)",[vec_blob(&[1.0,0.0,0.0])]).unwrap();
        db.execute("INSERT INTO rag_vec(rowid,embedding) VALUES(2,?1)",[vec_blob(&[0.0,1.0,0.0])]).unwrap();
        let hit:i64=db.query_row("SELECT c.page FROM (SELECT rowid,distance FROM rag_vec WHERE embedding MATCH ?1 AND k=1 ORDER BY distance) v JOIN rag_chunks c ON c.id=v.rowid ORDER BY v.distance",[vec_blob(&[0.9,0.1,0.0])],|r|r.get(0)).unwrap();
        assert_eq!(hit,2);
    }
    #[test]
    fn related_documents_use_selected_chunk_and_exclude_same_document(){
        unsafe {rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ())));}
        let db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();ensure_vec(&db,3).unwrap();
        for (id,name) in [("file:1","First"),("file:2","Second"),("file:3","Third")] {
            db.execute("INSERT INTO nodes(id,portal_id,kind,name,path) VALUES(?1,?2,'file',?3,'/')",params![id,id.trim_start_matches("file:"),name]).unwrap();
        }
        for (id,node_id,vector) in [(1,"file:1",[1.0,0.0,0.0]),(2,"file:1",[0.99,0.01,0.0]),(3,"file:2",[0.9,0.1,0.0]),(4,"file:3",[0.0,1.0,0.0])] {
            db.execute("INSERT INTO rag_chunks(id,node_id,page,text) VALUES(?1,?2,1,'sample')",params![id,node_id]).unwrap();
            db.execute("INSERT INTO rag_vec(rowid,embedding) VALUES(?1,?2)",params![id,vec_blob(&vector)]).unwrap();
        }
        let rows=related_hits(&db,1).unwrap();
        assert_eq!(rows.len(),2);
        assert_eq!(rows[0].node_id,"file:2");
        assert_eq!(rows[1].node_id,"file:3");
    }
    #[test]
    fn reindex_replaces_vectors_and_tracks_ocr_warning() {
        unsafe {rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ())));}
        let mut db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();ensure_vec(&db,3).unwrap();
        let item=Node{id:"file:4".into(),portal_id:"4".into(),kind:"file".into(),name:"Example".into(),path:"/Example".into(),active:true};
        save_document(&mut db,&item,b"%PDF-test",vec![vec![1.0,0.0,0.0]],2,1,vec![(1,"Example searchable content".into())]).unwrap();
        let (status,count):(String,i64)=db.query_row("SELECT status,chunk_count FROM rag_documents WHERE node_id='file:4'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((status.as_str(),count),("partial_text",1));
        save_document(&mut db,&item,b"%PDF-image",vec![],2,2,vec![]).unwrap();
        let (status,count):(String,i64)=db.query_row("SELECT status,chunk_count FROM rag_documents WHERE node_id='file:4'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((status.as_str(),count),("needs_ocr",0));
        let vectors:i64=db.query_row("SELECT COUNT(*) FROM rag_vec",[],|r|r.get(0)).unwrap();
        assert_eq!(vectors,0);
    }
    #[test]
    fn interrupted_rag_job_keeps_pending_queue() {
        unsafe {rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ())));}
        let db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();
        db.execute("INSERT INTO rag_jobs(id,status,total) VALUES('job','running',1)",[]).unwrap();
        db.execute("INSERT INTO rag_queue(job_id,node_id) VALUES('job','file:7')",[]).unwrap();
        recover_interrupted(&db).unwrap();
        let job=latest_job(&db).unwrap().unwrap();
        assert_eq!(job.status,"paused");
        let pending:i64=db.query_row("SELECT COUNT(*) FROM rag_queue WHERE job_id='job' AND status='pending'",[],|r|r.get(0)).unwrap();
        assert_eq!(pending,1);
    }
    #[test]
    fn resume_requeues_failed_documents() {
        let mut db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();
        db.execute("INSERT INTO rag_jobs(id,status,total,completed,failed,last_error) VALUES('job','error',3,1,1,'timeout')",[]).unwrap();
        for (node,status) in [("file:1","done"),("file:2","failed"),("file:3","pending")] {
            db.execute("INSERT INTO rag_queue(job_id,node_id,status) VALUES('job',?1,?2)",params![node,status]).unwrap();
        }
        retry_failed_rag_queue(&mut db,"job").unwrap();
        let job=latest_job(&db).unwrap().unwrap();
        assert_eq!((job.status.as_str(),job.completed,job.failed,job.last_error.as_str()),("running",1,0,""));
        let pending:i64=db.query_row("SELECT COUNT(*) FROM rag_queue WHERE job_id='job' AND status='pending'",[],|r|r.get(0)).unwrap();
        assert_eq!(pending,2);
    }
    #[test]
    fn api_embeddings_sort_by_index_and_check_dimension() {
        use std::io::{Read,Write};
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint=format!("http://{}/v1/embeddings",listener.local_addr().unwrap());
        let server=std::thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap();
            let mut bytes=[0u8;4096];
            let count=stream.read(&mut bytes).unwrap();
            let request=String::from_utf8_lossy(&bytes[..count]);
            assert!(request.contains("POST /v1/embeddings"));
            let body=r#"{"data":[{"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]}"#;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let vectors=tokio::runtime::Runtime::new().unwrap().block_on(api_embed(&endpoint,"test","secret",&["a".into(),"b".into()],Some(2))).unwrap();
        server.join().unwrap();
        assert_eq!(vectors,vec![vec![1.0,0.0],vec![0.0,1.0]]);
    }
}
