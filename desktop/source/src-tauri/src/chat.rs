use super::*;
use std::{io::{Read, Write}, process::{Command, Stdio}, thread};

const MODEL: &str = "gpt-6-luna";

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct ChatStatus { ready: bool, model: &'static str, indexed_documents: i64, message: String }
#[derive(Clone, Serialize)]
#[serde(rename_all="camelCase")]
struct ChatProgress { request_id: String, workspace_id: String, stage: &'static str }
#[derive(Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct Citation { id: usize, node_id: String, name: String, path: String, page: i64, text: String, chunk_id: i64 }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct ChatAnswer { request_id: String, answer: String, citations: Vec<Citation>, insufficient_evidence: bool, model: &'static str }
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct ModelAnswer { answer: String, citations: Vec<usize>, insufficient_evidence: bool }

fn runtime_dir() -> Result<PathBuf> {
    let executable=std::env::current_exe().map_err(err)?;
    let bundled=executable.parent().ok_or("ไม่พบโฟลเดอร์โปรแกรม")?.join("qa-runtime");
    let dev=Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("ไม่พบโฟลเดอร์โครงการ")?.join("qa-runtime");
    [bundled,dev].into_iter().find(|dir|dir.join("bridge.mjs").is_file() && dir.join("node_modules/@roberttlange/headless/dist/agents.js").is_file())
        .ok_or("ไม่พบส่วนเสริมถามตอบ กรุณาวางโฟลเดอร์ qa-runtime ไว้ข้าง DocumentAtlas.exe".into())
}
fn node_binary() -> Result<PathBuf> {
    let mut candidates=Vec::new();
    if let Some(program_files)=std::env::var_os("ProgramFiles") { candidates.push(PathBuf::from(program_files).join("nodejs/node.exe")); }
    if let Some(paths)=std::env::var_os("PATH") { candidates.extend(std::env::split_paths(&paths).map(|p|p.join(if cfg!(windows){"node.exe"}else{"node"}))); }
    candidates.into_iter().find(|p|p.is_file()).ok_or("ไม่พบ Node.js ที่ใช้เรียกโมเดล กรุณาติดตั้ง Node.js 22 ขึ้นไป".into())
}
fn codex_available() -> bool {
    std::env::var_os("APPDATA").is_some_and(|p|PathBuf::from(p).join("npm/node_modules/@openai/codex/bin/codex.js").is_file())
}
#[tauri::command]
pub(crate) fn get_iso_chat_status(state: State<'_,AppState>) -> Result<ChatStatus> {
    let db=state.db.lock().map_err(err)?;
    let indexed_documents=db.query_row("SELECT COUNT(*) FROM rag_documents WHERE status IN ('ready','partial_text') AND chunk_count>0",[],|r|r.get(0)).map_err(err)?;
    let error=runtime_dir().err().or_else(||node_binary().err()).or_else(||(!codex_available()).then(||"ไม่พบ Codex CLI ในเครื่อง".into()));
    Ok(ChatStatus{ready:error.is_none(),model:MODEL,indexed_documents,message:error.unwrap_or_else(||"พร้อมถามตอบผ่านบัญชี ChatGPT ที่ล็อกอินในเครื่อง".into())})
}

fn choose_sources(db: &Connection, hits: Vec<rag::RagHit>) -> Result<Vec<Citation>> {
    let mut sources=Vec::new();
    let mut seen=HashSet::new();
    let mut counts=std::collections::HashMap::<String,usize>::new();
    for hit in hits {
        let current:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM rag_documents d JOIN nodes n ON n.id=d.node_id WHERE d.node_id=?1 AND d.status IN ('ready','partial_text') AND n.active=1)",[&hit.node_id],|r|r.get(0)).map_err(err)?;
        if !current || hit.text.trim().is_empty() || !seen.insert(hit.chunk_id) { continue; }
        let count=counts.entry(hit.node_id.clone()).or_default();
        if *count>=3 { continue; }
        *count+=1;
        sources.push(Citation{id:sources.len()+1,node_id:hit.node_id,name:hit.name,path:hit.path,page:hit.page,text:hit.text,chunk_id:hit.chunk_id});
        if sources.len()==10 { break; }
    }
    Ok(sources)
}
fn parse_answer(text: &str, sources: &[Citation]) -> Result<ModelAnswer> {
    let result:ModelAnswer=serde_json::from_str(text.trim()).map_err(|_|"รูปแบบคำตอบไม่สมบูรณ์ กรุณาลองถามใหม่".to_string())?;
    if result.answer.trim().is_empty() || result.answer.chars().count()>16000 { return Err("ความยาวคำตอบไม่ถูกต้อง กรุณาลองถามใหม่".into()); }
    let ids:HashSet<usize>=result.citations.iter().copied().collect();
    if ids.len()!=result.citations.len() || ids.iter().any(|id|!sources.iter().any(|s|s.id==*id)) {
        return Err("คำตอบอ้างอิงเอกสารที่ไม่ได้มาจากผลค้นหา กรุณาลองถามใหม่".into());
    }
    let mut inline=HashSet::new();
    for part in result.answer.split('[').skip(1) {
        if let Some((id,_))=part.split_once(']') {
            if !id.is_empty() && id.bytes().all(|c|c.is_ascii_digit()) {
                inline.insert(id.parse::<usize>().map_err(|_|"เลขอ้างอิงไม่ถูกต้อง".to_string())?);
            }
        }
    }
    if inline!=ids || (!result.insufficient_evidence && ids.is_empty()) {
        return Err("คำตอบยังไม่มีแหล่งอ้างอิงที่ตรวจสอบได้ กรุณาลองถามใหม่".into());
    }
    Ok(result)
}
fn hidden(command: &mut Command) {
    #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    #[cfg(not(windows))] let _=command;
}
fn stop_child(child: &mut std::process::Child) {
    #[cfg(windows)] {
        let mut command=Command::new("taskkill.exe");
        command.args(["/PID",&child.id().to_string(),"/T","/F"]).stdout(Stdio::null()).stderr(Stdio::null());hidden(&mut command);
        let _=command.status();
    }
    let _=child.kill();let _=child.wait();
}
fn run_headless(runtime: PathBuf, node: PathBuf, prompt: String, cancel: Arc<AtomicBool>) -> Result<String> {
    let payload=serde_json::to_vec(&serde_json::json!({"prompt":prompt})).map_err(err)?;
    let mut command=Command::new(node);
    command.arg(runtime.join("bridge.mjs")).current_dir(&runtime).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for name in ["OPENAI_API_KEY","CODEX_API_KEY","OPENAI_BASE_URL","NODE_OPTIONS"] { command.env_remove(name); }
    hidden(&mut command);
    let mut child=command.spawn().map_err(|_|"เปิดส่วนเสริม headless ไม่สำเร็จ กรุณาตรวจ Node.js".to_string())?;
    let stdout=child.stdout.take().ok_or("อ่านคำตอบไม่ได้")?;
    let stderr=child.stderr.take().ok_or("อ่านสถานะโมเดลไม่ได้")?;
    let output=thread::spawn(move|| {let mut bytes=Vec::new();let _=stdout.take(2_000_001).read_to_end(&mut bytes);bytes});
    let errors=thread::spawn(move|| {let mut bytes=Vec::new();let _=stderr.take(64_000).read_to_end(&mut bytes);bytes});
    let write_result=child.stdin.take().ok_or_else(||"ส่งคำถามไม่ได้".to_string()).and_then(|mut pipe|pipe.write_all(&payload).map_err(err));
    if let Err(error)=write_result {stop_child(&mut child);let _=output.join();let _=errors.join();return Err(error);}
    let start=Instant::now();
    let status=loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed()>Duration::from_secs(150) {
            stop_child(&mut child);let _=output.join();let _=errors.join();
            return Err(if cancel.load(Ordering::Relaxed){"ยกเลิกคำถามแล้ว"}else{"ใช้เวลาตอบนานเกินกำหนด กรุณาลองใหม่"}.into());
        }
        match child.try_wait() {Ok(Some(status))=>break status,Ok(None)=>thread::sleep(Duration::from_millis(100)),Err(error)=>{stop_child(&mut child);let _=output.join();let _=errors.join();return Err(err(error));}}
    };
    let stdout=output.join().map_err(|_|"อ่านคำตอบไม่สำเร็จ")?;
    let stderr=errors.join().map_err(|_|"อ่านสถานะไม่สำเร็จ")?;
    if !status.success() { return Err(String::from_utf8_lossy(&stderr).chars().take(600).collect()); }
    if stdout.len()>2_000_000 {return Err("คำตอบมีขนาดเกินกำหนด".into());}
    String::from_utf8(stdout).map_err(|_|"คำตอบมีอักขระไม่ถูกต้อง".into())
}

struct ChatLease<'a> {state:&'a AppState,cancel:Arc<AtomicBool>}
impl Drop for ChatLease<'_> {
    fn drop(&mut self) {if let Ok(mut slot)=self.state.chat_cancel.lock(){if slot.as_ref().is_some_and(|c|Arc::ptr_eq(c,&self.cancel)){*slot=None;}}}
}
#[tauri::command]
pub(crate) fn cancel_iso_chat(state: State<'_,AppState>, request_id:String) -> Result<()> {
    let slot=state.chat_cancel.lock().map_err(err)?;
    let id=state.chat_request.lock().map_err(err)?;
    if *id==request_id {if let Some(cancel)=slot.as_ref(){cancel.store(true,Ordering::Relaxed);}}
    Ok(())
}
#[tauri::command]
pub(crate) async fn ask_iso_chat(app: AppHandle, state: State<'_,AppState>, question:String, workspace_id:String, request_id:String) -> Result<ChatAnswer> {
    let question=question.trim();
    if question.is_empty() || question.chars().count()>2000 {return Err("กรุณาพิมพ์คำถาม 1–2,000 ตัวอักษร".into());}
    if request_id.len()>80 || request_id.is_empty(){return Err("รหัสคำถามไม่ถูกต้อง".into());}
    let runtime=runtime_dir()?;let node=node_binary()?;
    let cancel=Arc::new(AtomicBool::new(false));
    {let mut slot=state.chat_cancel.lock().map_err(err)?;if slot.is_some(){return Err("มีคำถามกำลังทำงานอยู่ กรุณารอหรือกดยกเลิกก่อน".into());}*slot=Some(cancel.clone());*state.chat_request.lock().map_err(err)?=request_id.clone();}
    let _lease=ChatLease{state:state.inner(),cancel:cancel.clone()};
    {let registry=state.registry.lock().map_err(err)?;if active_workspace_id(&registry)?!=workspace_id{return Err("พื้นที่สแกนเปลี่ยน กรุณาถามใหม่ในพื้นที่ปัจจุบัน".into());}}
    let progress=|stage|{let _=app.emit("iso-chat-progress",ChatProgress{request_id:request_id.clone(),workspace_id:workspace_id.clone(),stage});};
    progress("retrieving");
    let hits=rag::search_hits(state.inner(),question).await?;
    let sources={let registry=state.registry.lock().map_err(err)?;if active_workspace_id(&registry)?!=workspace_id{return Err("พื้นที่สแกนเปลี่ยน กรุณาถามใหม่".into());}let db=state.db.lock().map_err(err)?;choose_sources(&db,hits)?};
    if cancel.load(Ordering::Relaxed){return Err("ยกเลิกคำถามแล้ว".into());}
    let response=if sources.is_empty() {
        ModelAnswer{answer:"ยังไม่พบข้อความที่ใช้อ้างอิงได้ในดัชนี RAG ของพื้นที่นี้ กรุณาตรวจการทำดัชนีหรือระบุชื่อเอกสารและหัวข้อให้ชัดเจนขึ้น".into(),citations:vec![],insufficient_evidence:true}
    } else {
        progress("answering");
        let prompt=serde_json::to_string(&serde_json::json!({"question":question,"sources":sources})).map_err(err)?;
        let text=tauri::async_runtime::spawn_blocking(move||run_headless(runtime,node,prompt,cancel)).await.map_err(err)??;
        parse_answer(&text,&sources)?
    };
    {let registry=state.registry.lock().map_err(err)?;if active_workspace_id(&registry)?!=workspace_id{return Err("พื้นที่สแกนเปลี่ยนระหว่างตอบ กรุณาถามใหม่".into());}}
    if _lease.cancel.load(Ordering::Relaxed){return Err("ยกเลิกคำถามแล้ว".into());}
    let citations=sources.into_iter().filter(|s|response.citations.contains(&s.id)).collect();
    Ok(ChatAnswer{request_id,answer:response.answer,citations,insufficient_evidence:response.insufficient_evidence,model:MODEL})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(id:usize)->Citation{Citation{id,node_id:format!("file:{id}"),name:"ISO".into(),path:"Portal/ISO".into(),page:2,text:"ข้อความต้นฉบับ".into(),chunk_id:id as i64}}
    #[test]
    fn accepts_grounded_answers_and_missing_evidence() {
        let result=parse_answer(r#"{"answer":"ทำตามเอกสาร [1]","citations":[1],"insufficientEvidence":false}"#,&[source(1)]).unwrap();
        assert_eq!(result.citations,vec![1]);
        assert!(parse_answer(r#"{"answer":"ข้อมูลไม่พอ","citations":[],"insufficientEvidence":true}"#,&[]).is_ok());
    }
    #[test]
    fn rejects_invented_citations_and_uncited_claims() {
        for value in [r#"{"answer":"คำตอบ [9]","citations":[9],"insufficientEvidence":false}"#,
            r#"{"answer":"คำตอบ [2]","citations":[1],"insufficientEvidence":false}"#,
            r#"{"answer":"ไม่มีหลักฐานแต่อ้างว่ารู้","citations":[],"insufficientEvidence":false}"#,
            r#"{"answer":"คำตอบ [1]","citations":[1,1],"insufficientEvidence":false}"#] {
            assert!(parse_answer(value,&[source(1)]).is_err());
        }
    }
    #[test]
    fn excludes_stale_inactive_and_duplicate_passages() {
        let db=Connection::open_in_memory().unwrap();init_db(&db).unwrap();
        for (id,status,active) in [("file:1","ready",1),("file:2","stale",1),("file:3","ready",0)] {
            db.execute("INSERT INTO nodes(id,portal_id,kind,name,path,active) VALUES(?1,'1','file','ISO','/ISO',?2)",params![id,active]).unwrap();
            db.execute("INSERT INTO rag_documents(node_id,status) VALUES(?1,?2)",params![id,status]).unwrap();
        }
        let mut hits=Vec::new();
        for (chunk_id,node_id) in [(1,"file:1"),(1,"file:1"),(2,"file:2"),(3,"file:3")] {
            hits.push(rag::RagHit{chunk_id,node_id:node_id.into(),name:"ISO".into(),path:"/ISO".into(),page:1,text:"ข้อความต้นฉบับ".into(),distance:0.1});
        }
        let sources=choose_sources(&db,hits).unwrap();assert_eq!(sources.len(),1);assert_eq!(sources[0].chunk_id,1);
    }
}
