import { useCallback, useEffect, useState } from 'react';
import { RagInspector } from './RagInspector';
import { open } from '@tauri-apps/plugin-dialog';
import { ArrowDownToLine, BookOpenText, ChevronDown, ChevronRight, CircleAlert, Database, ExternalLink, FileText, Folder, GitBranch, HardDrive, Link2, LoaderCircle, Pause, Play, Search, X } from 'lucide-react';
import { api, type AtlasNode, type RagConfig, type RagDocument, type RagHit, type RagJob, type RagStorage } from './api';
import './rag.css';

const models = [
  { id: 'multilingual-e5-small', name: 'Multilingual E5 Small', detail: '384 มิติ · ขนาดเล็ก · รองรับภาษาไทย', source: 'intfloat/multilingual-e5-small' },
  { id: 'bge-m3', name: 'BGE M3', detail: '1024 มิติ · หลายภาษา · ใช้พื้นที่มากกว่า', source: 'BAAI/bge-m3' },
];
const status: Record<string,string> = {ready:'พร้อมค้นหา',partial_text:'ข้อความบางหน้า',needs_ocr:'ต้องใช้ OCR',error:'เกิดข้อผิดพลาด',stale:'รอทำดัชนีใหม่'};
const jobStatus: Record<string,string> = {running:'กำลังทำดัชนี',paused:'หยุดชั่วคราว',error:'งานสะดุด',completed:'เสร็จแล้ว',completed_with_errors:'เสร็จบางส่วน'};

export function RagSettings({workspaceId}:{workspaceId:string}) {
  const [config,setConfig]=useState<RagConfig|null>(null);
  const [modelId,setModelId]=useState(models[0].id);
  const [path,setPath]=useState('');
  const [mode,setMode]=useState<'local'|'api'>('local');
  const [apiUrl,setApiUrl]=useState('');
  const [apiModel,setApiModel]=useState('');
  const [apiKey,setApiKey]=useState('');
  const [busy,setBusy]=useState(false);
  const [message,setMessage]=useState<{text:string;error:boolean}|null>(null);
  useEffect(()=>{api.ragConfig().then(c=>{setConfig(c);setModelId(c.modelId||models[0].id);setPath(c.modelPath);setMode(c.embeddingProvider);setApiUrl(c.apiUrl);setApiModel(c.apiModel)}).catch(e=>setMessage({text:String(e),error:true}))},[workspaceId]);
  async function choose(){const chosen=await open({directory:true,multiple:false,title:'เลือกโฟลเดอร์เก็บโมเดล Embedding'});if(typeof chosen==='string')setPath(chosen)}
  async function install(){setBusy(true);setMessage({text:'กำลังดาวน์โหลดและตรวจสอบโมเดล อาจใช้เวลาหลายนาที',error:false});try{const next=await api.installRagModel(modelId,path);setConfig(next);setMessage({text:'ติดตั้งโมเดลสำเร็จ พร้อมทำดัชนีในพื้นที่นี้',error:false})}catch(e){setMessage({text:String(e),error:true})}finally{setBusy(false)}}
  async function saveApi(){setBusy(true);setMessage({text:'กำลังทดสอบ API Embedding',error:false});try{const next=await api.saveRagApi(apiUrl,apiModel,apiKey);setConfig(next);setApiKey('');setMessage({text:`เชื่อมต่อ API สำเร็จ · ${next.apiDimensions} มิติ`,error:false})}catch(e){setMessage({text:String(e),error:true})}finally{setBusy(false)}}
  return <section className="panel rag-settings">
    <div className="panel-head"><div><h2>ส่วนเสริม Embedding</h2><p>เลือกโมเดลในเครื่องหรือ API ที่รองรับรูปแบบ Embeddings</p></div><BookOpenText size={22}/></div>
    <div className="rag-mode"><button className={mode==='local'?'chosen':''} onClick={()=>setMode('local')}>โมเดลในเครื่อง</button><button className={mode==='api'?'chosen':''} onClick={()=>setMode('api')}>Embedding API</button></div>
    {mode==='local'&&<>
    <div className="rag-models">{models.map(m=><label key={m.id} className={modelId===m.id?'rag-model chosen':'rag-model'}><input type="radio" name="rag-model" checked={modelId===m.id} onChange={()=>setModelId(m.id)}/><span><strong>{m.name}</strong><small>{m.detail}</small><small>แหล่งโมเดล: {m.source}</small></span></label>)}</div>
    <label className="rag-path-label">ตำแหน่งติดตั้งโมเดล</label>
    <div className="directory-row"><input readOnly value={path} placeholder="เลือกโฟลเดอร์สำหรับไฟล์โมเดล"/><button className="secondary" onClick={choose}>เลือก Path</button></div>
    <div className="rag-settings-foot"><span><Database size={17}/> Vector DB: sqlite-vec · เก็บแยกตามพื้นที่สแกน</span><button className="primary" disabled={busy||!path} onClick={install}>{busy?<LoaderCircle size={16} className="spin"/>:<ArrowDownToLine size={16}/>} {busy?'กำลังติดตั้ง…':'ติดตั้งส่วนเสริมโมเดล'}</button></div>
    </>}
    {mode==='api'&&<div className="rag-api"><label>Embeddings endpoint<input value={apiUrl} onChange={e=>setApiUrl(e.target.value)} placeholder="https://example.com/v1/embeddings"/></label><label>ชื่อโมเดล<input value={apiModel} onChange={e=>setApiModel(e.target.value)} placeholder="embedding-model"/></label><label>API key<input type="password" value={apiKey} onChange={e=>setApiKey(e.target.value)} placeholder={config?.hasApiKey?'บันทึกใน Keychain แล้ว · เว้นว่างเพื่อใช้เดิม':'เว้นว่างได้สำหรับ API ในเครื่อง'}/></label><p>ข้อความจาก PDF จะถูกส่งไปยัง endpoint นี้เพื่อสร้าง Embedding คีย์เก็บใน Keychain ของระบบ {apiUrl.startsWith('http://')&&'การเชื่อมต่อ HTTP ไม่เข้ารหัสข้อความและคีย์'}</p><div className="rag-settings-foot"><span><Database size={17}/> Vector DB: sqlite-vec · เก็บในเครื่อง</span><button className="primary" disabled={busy||!apiUrl||!apiModel} onClick={saveApi}>{busy?<LoaderCircle size={16} className="spin"/>:null} ทดสอบและบันทึก API</button></div></div>}
    {config?.installed&&<p className="rag-success">ใช้งานอยู่: {config.embeddingProvider==='api'?`${config.apiModel} · ${config.apiDimensions} มิติ`:models.find(m=>m.id===config.modelId)?.name} · {config.indexedDocuments} เอกสารในดัชนี</p>}
    {config?.installed&&((mode!==config.embeddingProvider)||(mode==='local'&&modelId!==config.modelId)||(mode==='api'&&(apiUrl!==config.apiUrl||apiModel!==config.apiModel)))&&<p className="rag-warning">เมื่อเปลี่ยน Embedding เวกเตอร์เดิมจะถูกล้างและต้องทำดัชนีเอกสารใหม่</p>}
    {message&&<div className={`notice notice-inline rag-message ${message.error?'error':''}`} role={message.error?'alert':'status'}><span className="notice-icon">{message.error?<CircleAlert size={17}/>:<BookOpenText size={17}/>}</span><span className="notice-copy"><span>{message.text}</span></span><button className="notice-close" aria-label="ปิดการแจ้งเตือน" onClick={()=>setMessage(null)}>×</button></div>}
  </section>;
}

export function RagPage({workspaceId,initialNode,onOpenNode,onSettings,mode='index'}:{workspaceId:string;initialNode:AtlasNode|null;onOpenNode:(nodeId:string)=>void;onSettings:()=>void;mode?:'index'|'search'}) {
  const [inspectId,setInspectId]=useState<string|null>(null);
  const closeInspector=useCallback(()=>setInspectId(null),[]);
  useEffect(()=>setInspectId(null),[workspaceId]);
  const [config,setConfig]=useState<RagConfig|null>(null);
  const [job,setJob]=useState<RagJob|null>(null);
  const [docs,setDocs]=useState<RagDocument[]>([]);
  const [nodeQuery,setNodeQuery]=useState('');
  const [nodes,setNodes]=useState<AtlasNode[]>([]);
  const [selected,setSelected]=useState<AtlasNode|null>(initialNode);
  const [scope,setScope]=useState('document');
  const [query,setQuery]=useState('');
  const [hits,setHits]=useState<RagHit[]>([]);
  const [activeHit,setActiveHit]=useState<RagHit|null>(null);
  const [related,setRelated]=useState<RagHit[]>([]);
  const [relatedBusy,setRelatedBusy]=useState(false);
  const [openBusy,setOpenBusy]=useState(false);
  const [modalError,setModalError]=useState('');
  const [storage,setStorage]=useState<RagStorage|null>(null);
  const [busy,setBusy]=useState('');
  const [error,setError]=useState('');
  useEffect(()=>{setSelected(initialNode);setScope(initialNode?.kind==='folder'?'folder_subtree':'document')},[initialNode]);
  useEffect(()=>{let active=true;api.ragConfig().then(c=>{if(active)setConfig(c)}).catch(e=>{if(active)setError(String(e))});async function refresh(){try{const [j,d]=await Promise.all([api.ragJob(),api.ragDocuments()]);if(active){setJob(j);setDocs(d)}}catch(e){if(active)setError(String(e))}}refresh();const timer=setInterval(refresh,1500);return()=>{active=false;clearInterval(timer)}},[workspaceId]);
  useEffect(()=>{let active=true;api.ragStorage().then(value=>{if(active)setStorage(value)}).catch(e=>{if(active)setError(String(e))});return()=>{active=false}},[workspaceId,job?.completed,job?.failed]);
  useEffect(()=>{if(!activeHit){setRelated([]);return}let active=true;setRelated([]);setRelatedBusy(true);api.ragRelated(activeHit.chunkId).then(rows=>{if(active)setRelated(rows)}).catch(e=>{if(active)setModalError(String(e))}).finally(()=>{if(active)setRelatedBusy(false)});return()=>{active=false}},[activeHit?.chunkId,workspaceId]);
  useEffect(()=>{if(!activeHit)return;const previous=document.body.style.overflow;document.body.style.overflow='hidden';const onKey=(event:KeyboardEvent)=>{if(event.key==='Escape')setActiveHit(null)};window.addEventListener('keydown',onKey);return()=>{document.body.style.overflow=previous;window.removeEventListener('keydown',onKey)}},[activeHit]);
  useEffect(()=>{let active=true;const timer=setTimeout(()=>api.search(nodeQuery).then(n=>{if(active)setNodes(n.slice(0,30))}).catch(e=>{if(active)setError(String(e))}),200);return()=>{active=false;clearTimeout(timer)}},[nodeQuery,workspaceId]);
  async function start(){if(!selected)return;setBusy('index');setError('');try{setJob(await api.startRagIndex(selected.id,scope))}catch(e){setError(String(e))}finally{setBusy('')}}
  async function pause(){try{await api.pauseRagIndex()}catch(e){setError(String(e))}}
  async function resume(){if(!job)return;setBusy('resume');try{setJob(await api.resumeRagIndex(job.id))}catch(e){setError(String(e))}finally{setBusy('')}}
  async function search(){setBusy('search');setError('');setActiveHit(null);try{setHits(await api.ragSearch(query))}catch(e){setError(String(e))}finally{setBusy('')}}
  function showHit(hit:RagHit){setActiveHit(hit);setModalError('')}
  async function openPdf(){if(!activeHit)return;setOpenBusy(true);setModalError('');try{await api.openRagDocument(activeHit.nodeId)}catch(e){setModalError(String(e))}finally{setOpenBusy(false)}}
  return <div className={`rag-page ${mode==='search'?'rag-page-search':''}`}>
    {mode==='index'&&<div className="page-heading"><div><div className="eyebrow">LOCAL RAG</div><h1>ทำดัชนี PDF</h1><p>แปลง PDF เป็นข้อความและสร้างเวกเตอร์ เพื่อใช้ค้นหาจากเนื้อหาเอกสาร</p></div><button className="secondary" onClick={onSettings}>ตั้งค่าโมเดล</button></div>}
    {!config?.installed&&<div className="notice notice-inline error"><span className="notice-icon"><CircleAlert size={18}/></span><span className="notice-copy"><strong>ยังไม่ได้ตั้งค่า Embedding</strong><span>ติดตั้งโมเดลหรือเชื่อมต่อ API ก่อนเริ่มทำดัชนี</span></span><button className="notice-action" onClick={onSettings}>ไปที่ตั้งค่า</button></div>}
    {error&&<div className="notice notice-inline error" role="alert"><span className="notice-icon"><CircleAlert size={18}/></span><span className="notice-copy"><strong>ดำเนินการไม่สำเร็จ</strong><span>{error}</span></span><button className="notice-close" aria-label="ปิดการแจ้งเตือน" onClick={()=>setError('')}>×</button></div>}
    {mode==='index'&&<section className="panel rag-index"><div className="panel-head"><div><h2>ทำดัชนี PDF</h2><p>เลือกเอกสารหรือโฟลเดอร์จากดัชนี Portal ที่สแกนแล้ว</p></div><FileText size={20}/></div>
      <div className="rag-node-search"><div className="search-box"><Search size={18}/><input value={nodeQuery} onChange={e=>setNodeQuery(e.target.value)} placeholder="ค้นหาเอกสารหรือโฟลเดอร์" aria-label="ค้นหาเอกสารหรือโฟลเดอร์ที่จะทำดัชนี"/></div>
      {nodeQuery&&<div className="rag-node-results">{nodes.map(n=><button key={n.id} onClick={()=>{setSelected(n);setScope(n.kind==='file'?'document':'folder_subtree');setNodeQuery('')}}>{n.kind==='folder'?<Folder size={16}/>:<FileText size={16}/>} <span>{n.name}<small>{n.path}</small></span></button>)}{nodes.length===0&&<div className="rag-no-nodes">ไม่พบเอกสารหรือโฟลเดอร์</div>}</div>}</div>
      {selected?<div className="rag-selection"><span className="rag-selection-label">รายการที่เลือก</span><strong>{selected.name}</strong><small>{selected.path}</small>{selected.kind==='folder'&&<label className="rag-scope-label">ขอบเขตการทำดัชนี<span className="rag-select-wrap"><select value={scope} onChange={e=>setScope(e.target.value)}><option value="folder_direct">เฉพาะไฟล์ในโฟลเดอร์นี้</option><option value="folder_subtree">รวมโฟลเดอร์ย่อยทั้งหมด</option></select><ChevronDown size={17}/></span></label>}</div>:<div className="rag-selection rag-selection-empty">เลือกเอกสารหรือโฟลเดอร์จากช่องค้นหาเพื่อเริ่มทำดัชนี</div>}
      <div className="rag-controls"><button className="primary" disabled={!config?.installed||!selected||job?.status==='running'||Boolean(busy)} onClick={start}>{busy==='index'?<LoaderCircle size={16} className="spin"/>:<Play size={16}/>} เริ่มทำดัชนี</button>{job?.status==='running'&&<button className="secondary" onClick={pause}><Pause size={16}/> หยุดหลังเอกสารปัจจุบัน</button>}{(job?.status==='paused'||job?.status==='error'||job?.status==='completed_with_errors')&&<button className="secondary" disabled={Boolean(busy)} onClick={resume}><Play size={16}/> ลองรายการที่ไม่สำเร็จอีกครั้ง</button>}</div>
      {job&&<div className="rag-job"><strong>Session {job.id.slice(0,8)} · {jobStatus[job.status]||job.status}</strong><span>{job.completed}/{job.total} สำเร็จ · {job.failed} ผิดพลาด {job.currentDocument&&'· '+job.currentDocument}</span><div className="progress-track"><span style={{width:`${job.total?((job.completed+job.failed)/job.total)*100:0}%`}}/></div>{job.lastError&&<small>{job.lastError}</small>}</div>}
    </section>}
    {mode==='search'&&<section className="panel rag-search"><div className="panel-head"><div><h2>ค้นหาจากเนื้อหา PDF</h2><p>ค้นหาความหมายในเอกสารที่ทำดัชนีแล้ว และเปิดผลลัพธ์เพื่อดูข้อความกับเอกสารที่เกี่ยวข้อง</p></div><Search size={20}/></div><div className="rag-search-row"><input value={query} onChange={e=>setQuery(e.target.value)} onKeyDown={e=>{if(e.key==='Enter')search()}} placeholder="ถามหรือค้นหาประเด็นในเอกสาร..."/><button className="primary" disabled={!query.trim()||!config?.installed||busy==='search'} onClick={search}>{busy==='search'?<LoaderCircle size={16} className="spin"/>:<Search size={16}/>} ค้นหา</button></div>
      <div className="rag-hits">{hits.map(hit=><button className="rag-hit" key={hit.chunkId} onClick={()=>showHit(hit)}><strong>{hit.name} · หน้า {hit.page}</strong><small>{hit.path}</small><p>{hit.text}</p><span className="rag-hit-link"><Link2 size={14}/> ดูเนื้อหาและเอกสารที่เกี่ยวข้อง</span></button>)}
      {hits.length===0&&<p className="empty-small">ทำดัชนีเอกสารก่อน แล้วค้นหาจากข้อความใน PDF ได้ที่นี่</p>}</div>
    </section>}
    {mode==='index'&&<section className="panel rag-library"><div className="panel-head"><div><h2>สถานะเอกสาร</h2><p>แจ้งเตือน PDF ที่ไม่มี text layer หรืออ่านได้เพียงบางหน้า</p></div><Database size={20}/></div><div className="rag-doc-list">{docs.map(d=><div className="rag-doc" key={d.nodeId}><FileText size={18}/><div><strong>{d.name}</strong><small>{d.pageCount} หน้า · {d.chunkCount} ช่วงข้อความ · {d.updatedAt}</small>{d.warning&&<small className="rag-warning">{d.warning}</small>}</div><span className={d.status==='ready'?'rag-status ok':'rag-status'}>{status[d.status]||d.status}</span><button className="secondary rag-doc-inspect" onClick={()=>setInspectId(d.nodeId)}><BookOpenText size={15}/> ตรวจข้อมูล RAG</button></div>)}{docs.length===0&&<p className="empty-small">ยังไม่มีเอกสารในดัชนี RAG</p>}</div>{storage&&<details className="rag-storage"><summary><HardDrive size={15}/> ข้อความทำดัชนีเก็บไว้ที่ไหน <span>{storage.indexedDocuments} เอกสาร · {storage.chunks} ช่วงข้อความ</span></summary><div><strong>ฐานข้อมูล SQLite ของพื้นที่นี้</strong><code>{storage.databasePath}</code><small>ข้อความอยู่ในตาราง rag_chunks และเวกเตอร์อยู่ใน rag_vec</small><strong>แคช PDF เมื่อกดเปิดเอกสาร</strong><code>{storage.pdfCachePath}</code></div></details>}</section>}
    {activeHit&&<div className="rag-modal-backdrop" onMouseDown={event=>{if(event.target===event.currentTarget)setActiveHit(null)}}><section className="rag-modal" role="dialog" aria-modal="true" aria-labelledby="rag-detail-title"><div className="rag-modal-top"><span className="rag-modal-icon"><BookOpenText size={22}/></span><button className="rag-modal-close" aria-label="ปิดรายละเอียด" onClick={()=>setActiveHit(null)}><X size={20}/></button></div><div className="rag-modal-kicker">ผลค้นหาจากความหมาย · หน้า {activeHit.page}</div><h2 id="rag-detail-title">{activeHit.name}</h2><p className="rag-modal-path">{activeHit.path}</p><div className="rag-modal-body"><section className="rag-passage"><h3>เนื้อหาที่ตรงกับความหมาย</h3><p>{activeHit.text}</p><small>ข้อความต้นฉบับจาก PDF หน้า {activeHit.page} ไม่ใช่คำตอบที่สร้างโดยโมเดลภาษา</small></section><section className="rag-related"><div className="rag-related-head"><div><Link2 size={18}/><h3>เอกสารที่มีเนื้อหาใกล้เคียง</h3></div><span>{related.length} เอกสาร</span></div>{relatedBusy?<div className="rag-related-empty"><LoaderCircle className="spin" size={18}/> กำลังค้นหาความเชื่อมโยง…</div>:related.length?related.map(hit=><button className="rag-related-item" key={hit.chunkId} onClick={()=>showHit(hit)}><FileText size={17}/><span><strong>{hit.name}</strong><small>หน้า {hit.page} · {hit.text}</small></span><ChevronRight size={16}/></button>):<div className="rag-related-empty">ยังไม่พบเอกสารอื่นที่เชื่อมโยงกับข้อความนี้</div>}</section></div>{modalError&&<div className="notice notice-inline error rag-modal-error" role="alert"><CircleAlert size={17}/>{modalError}</div>}<div className="rag-modal-actions"><button className="secondary" onClick={()=>setInspectId(activeHit.nodeId)}><BookOpenText size={16}/> ตรวจข้อมูล RAG ทั้งไฟล์</button><button className="secondary" onClick={()=>{const id=activeHit.nodeId;setActiveHit(null);onOpenNode(id)}}><GitBranch size={16}/> ดูในกราฟ</button><button className="primary" disabled={openBusy} onClick={openPdf}>{openBusy?<LoaderCircle size={16} className="spin"/>:<ExternalLink size={16}/>} {openBusy?'กำลังเปิด PDF…':'เปิด PDF'}</button></div></section></div>}
    {inspectId&&<RagInspector key={`${workspaceId}:${inspectId}`} nodeId={inspectId} onClose={closeInspector}/>}
  </div>;
}
