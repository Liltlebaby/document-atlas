import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import cytoscape from 'cytoscape';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { Activity, ArrowDownToLine, Bot, BookOpenText, Check, ChevronRight, CircleAlert, ClipboardCheck, Database, FileText, Folder, FolderOpen, GitBranch, HardDrive, LayoutDashboard, LoaderCircle, PanelLeftClose, PanelLeftOpen, Pause, Play, RotateCw, Search, Settings, Square, Terminal, Trash2, X } from 'lucide-react';
import { api, type AtlasNode, type DownloadedDocument, type Graph, type GraphPage, type Job, type Plan, type Profile, type Progress, type ScanLog, type ScanSession, type Workspace, type WorkspaceDeletePreview } from './api';
import { RagPage, RagSettings } from './Rag';
import { IsoPage } from './Iso';
import { IsoChatPage } from './IsoChat';
import { AgentPage } from './Agent';

type View = 'chat' | 'overview' | 'search' | 'graph' | 'downloads' | 'spaces' | 'rag' | 'iso' | 'agent' | 'settings';
const blank: Profile = { baseUrl: '', username: '', downloadDir: '', hasCredential: false };
const isTauri = Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
const statusLabel: Record<string,string> = { running: 'กำลังสแกน', pausing: 'กำลังหยุด', paused: 'หยุดชั่วคราว', completed: 'สแกนเสร็จ', error: 'สแกนสะดุด' };
function short(id: string) { return id.replace('folder:', '#').replace('file:', '#'); }
function scopeLabel(scope: string) { return ({ document: 'เอกสารนี้', folder_direct: 'ไฟล์ในโฟลเดอร์', folder_subtree: 'ทั้งโฟลเดอร์และโฟลเดอร์ย่อย' } as Record<string,string>)[scope]; }
function duration(seconds:number){const value=Math.max(0,Math.floor(seconds));const h=Math.floor(value/3600),m=Math.floor(value%3600/60),s=value%60;return h?`${h} ชม. ${m} นาที ${s} วิ`:m?`${m} นาที ${s} วิ`:`${s} วิ`}
function liveElapsed(session:ScanSession|null,now:number){if(!session)return 0;const updated=Date.parse(session.updatedAt.replace(' ','T')+'Z');return session.elapsedSeconds+(session.status==='running'&&Number.isFinite(updated)?Math.max(0,Math.floor((now-updated)/1000)):0)}
function NodeIcon({kind, size=18}:{kind:string,size?:number}) { return kind === 'folder' ? <Folder size={size}/> : <FileText size={size}/>; }

function GraphCanvas({data, selected, expanded, onSelect}:{data:Graph|null;selected:string;expanded:Set<string>;onSelect:(id:string)=>void}) {
  const box = useRef<HTMLDivElement>(null);
  const cyRef = useRef<cytoscape.Core|null>(null);
  const onSelectRef = useRef(onSelect);
  onSelectRef.current=onSelect;
  useEffect(() => {
    if (!box.current) return;
    const cy=cytoscape({container:box.current,elements:[],style:[
      {selector:'node',style:{label:'data(label)','font-family':'Anuphan, sans-serif','font-size':11,'font-weight':600,'text-wrap':'wrap','text-max-width':'110px','text-valign':'bottom','text-margin-y':8,color:'#536472',width:48,height:48,'background-color':'#d8eae4','border-width':2,'border-color':'#91bcaa'}},
      {selector:'node[kind="file"]',style:{shape:'round-rectangle',width:34,height:40,'background-color':'#e8ecf8','border-color':'#aab8dd'}},
      {selector:'node.expanded',style:{'background-color':'#b5e5d3','border-color':'#28a47f'}},
      {selector:'node.selected',style:{'background-color':'#0e8268','border-color':'#075c4a','border-width':3,color:'#0d5647','font-weight':700}},
      {selector:'edge',style:{width:1.6,'line-color':'#b8c8c1','curve-style':'bezier','target-arrow-shape':'triangle','target-arrow-color':'#b8c8c1','arrow-scale':0.7}}
    ],layout:{name:'preset'},minZoom:0.2,maxZoom:2.5,wheelSensitivity:0.2});
    cy.on('tap','node',e=>onSelectRef.current(e.target.id()));
    cyRef.current=cy;
    return ()=>{cy.destroy();cyRef.current=null};
  },[]);
  useEffect(()=>{
    const cy=cyRef.current;if(!cy||!data)return;
    const ids=new Set(data.nodes.map(n=>n.id));
    const edgeIds=new Set(data.edges.map(e=>`edge:${e.source}:${e.target}`));
    cy.batch(()=>{
      cy.edges().forEach(e=>{if(!edgeIds.has(e.id()))e.remove()});
      cy.nodes().forEach(n=>{if(!ids.has(n.id()))n.remove()});
      data.nodes.forEach(n=>{const existing=cy.getElementById(n.id);if(existing.length)existing.data({label:n.name,kind:n.kind});else cy.add({group:'nodes',data:{id:n.id,label:n.name,kind:n.kind}})});
      data.edges.forEach(e=>{const id=`edge:${e.source}:${e.target}`;if(!cy.getElementById(id).length&&ids.has(e.source)&&ids.has(e.target))cy.add({group:'edges',data:{id,source:e.source,target:e.target}})});
    });
    cy.nodes().removeClass('expanded');
    expanded.forEach(id=>cy.getElementById(id).addClass('expanded'));
    if(cy.nodes().length)cy.layout({name:'cose',animate:true,animationDuration:550,fit:true,padding:55,nodeRepulsion:()=>6500,idealEdgeLength:()=>110,edgeElasticity:()=>80,gravity:0.45,numIter:800}).run();
  },[data,expanded]);
  useEffect(()=>{const cy=cyRef.current;if(!cy)return;cy.nodes().removeClass('selected');if(selected)cy.getElementById(selected).addClass('selected')},[selected,data]);
  return <div className="graph-canvas" ref={box}>{data?.nodes.length===0&&<div className="empty-overlay">เริ่มสำรวจ Portal เพื่อแสดง Folder หลัก</div>}</div>;
}

function ScanPanel({session,progress,onStart,onPause,onResume,now,disabled=false}:{session:ScanSession|null;progress:Progress|null;onStart:()=>void;onPause:()=>void;onResume:()=>void;now:number;disabled?:boolean}) {
  const running=session?.status==='running', pausing=session?.status==='pausing';
  const resumable=session?.status==='paused'||session?.status==='error';
  const seen=(session?.foldersDone||0)+(session?.foldersPending||0);
  const pct=seen ? Math.max(3,Math.round((session?.foldersDone||0)/seen*100)) : 0;
  return <section className="panel scan-panel">
    <div className="scan-top"><div className="scan-title"><span className="scan-symbol"><Activity size={20}/></span><div><h2>การสำรวจ Portal</h2><p>ค้นหาโฟลเดอร์และเอกสารที่บัญชีนี้เข้าถึงได้โดยอัตโนมัติ</p></div></div><span className={`scan-badge ${session?.status||'idle'}`}>{session?statusLabel[session.status]:'ยังไม่เริ่ม'}</span></div>
    <div className="scan-detail">
      <div><small>ตำแหน่งล่าสุด</small><strong>{session?.currentFolder||'ยังไม่ได้เริ่มสำรวจ'}</strong></div>
      <div><small>โฟลเดอร์ที่อ่านแล้ว</small><strong>{session?.foldersDone||0}</strong></div>
      <div><small>รออ่าน</small><strong>{session?.foldersPending||0}</strong></div>
      <div><small>เอกสารที่พบ</small><strong>{session?.filesSeen||0}</strong></div>
      <div><small>เวลาที่ใช้สแกน</small><strong>{duration(liveElapsed(session,now))}</strong></div>
      <div><small>การลองล่าสุด</small><strong>{session?.retryAttempt?`${session.retryAttempt}/4`:'—'}</strong></div>
    </div>
    {session&&<div className="scan-track"><span style={{width:`${pct}%`}}/></div>}
    <div className="scan-foot"><span>{session?.lastError||((running||pausing)&&progress?.kind==='scan'?progress.message:session?.updatedAt?`อัปเดตล่าสุด ${session.updatedAt}`:'ระบบจะค้นหา Root Folder ให้เอง')}</span><div>
      {running&&<button className="secondary" onClick={onPause}><Pause size={15}/> หยุดชั่วคราว</button>}
      {pausing&&<button className="secondary" disabled><LoaderCircle size={15} className="spin"/> กำลังหยุด</button>}
      {resumable&&<button className="primary" onClick={onResume} disabled={disabled}><Play size={15}/> สแกนต่อ</button>}
      {(!session||session.status==='completed')&&<button className="primary" onClick={onStart} disabled={disabled}><RotateCw size={15}/> {session?'สแกนใหม่':'เริ่มสำรวจทั้ง Portal'}</button>}
    </div></div>
  </section>;
}

function ScanConsole({session}:{session:ScanSession|null}) {
  const [expanded,setExpanded]=useState(true);
  const [logs,setLogs]=useState<ScanLog[]>([]);
  const [error,setError]=useState('');
  const body=useRef<HTMLDivElement>(null);
  useEffect(()=>{if(!session){setLogs([]);return}setExpanded(true)},[session?.id]);
  useEffect(()=>{
    if(!session || !expanded)return;
    let active=true;
    const refresh=()=>api.scanLogs(session.id).then(rows=>{if(active){setLogs(rows);setError('')}}).catch(e=>{if(active)setError(String(e))});
    refresh();const timer=setInterval(refresh,1500);
    return()=>{active=false;clearInterval(timer)};
  },[session?.id,expanded]);
  useEffect(()=>{if(expanded&&body.current)body.current.scrollTop=body.current.scrollHeight},[logs.length,expanded]);
  if(!session)return null;
  return <section className="scan-console" aria-label="Console การสแกน"><div className="scan-console-head"><div><Terminal size={17}/><strong>SCAN CONSOLE</strong><span className={session.status==='running'?'scan-console-live':''}>{session.status==='running'?'LIVE':statusLabel[session.status]||session.status}</span></div><button onClick={()=>setExpanded(value=>!value)} aria-expanded={expanded}>{expanded?'พับ Logs':'แสดง Logs'}</button></div>{expanded&&<div className="scan-console-body" ref={body} role="log" aria-live="off">{logs.length?logs.map(item=><div className={`scan-log ${item.level}`} key={item.id}><time>{item.createdAt}</time><span>{item.level.toUpperCase()}</span><p>{item.message}</p></div>):<div className="scan-console-empty">{session.status==='running'?'กำลังรอข้อความจากการสแกน…':'ยังไม่มี Logs สำหรับ Session นี้'}</div>}{error&&<div className="scan-console-error">อ่าน Logs ไม่สำเร็จ: {error}</div>}</div>}</section>;
}

export default function App() {
  const autoScanStarted = useRef(false);
  const [sidebarCollapsed,setSidebarCollapsed] = useState(()=>{try{return localStorage.getItem('atlas.sidebar.collapsed')==='true'}catch{return false}});
  const [view,setView] = useState<View>('overview');
  const [profile,setProfile] = useState<Profile>(blank);
  const [password,setPassword] = useState('');
  const [query,setQuery] = useState('');
  const [kind,setKind] = useState('');
  const [results,setResults] = useState<AtlasNode[]>([]);
  const [searchTab,setSearchTab] = useState<'portal'|'downloaded'|'content'>('portal');
  const [downloadedQuery,setDownloadedQuery] = useState('');
  const [downloadedResults,setDownloadedResults] = useState<DownloadedDocument[]>([]);
  const [selected,setSelected] = useState<AtlasNode|null>(null);
  const [graphSeed,setGraphSeed] = useState<Graph|null>(null);
  const [children,setChildren] = useState<Record<string,GraphPage>>({});
  const [expanded,setExpanded] = useState<Set<string>>(new Set());
  const [graphLoading,setGraphLoading] = useState('');
  const [plan,setPlan] = useState<Plan|null>(null);
  const [jobs,setJobs] = useState<Job[]>([]);
  const [session,setSession] = useState<ScanSession|null>(null);
  const [history,setHistory] = useState<ScanSession[]>([]);
  const [inspectedSession,setInspectedSession] = useState<ScanSession|null>(null);
  const [workspaces,setWorkspaces] = useState<Workspace[]>([]);
  const [spaceName,setSpaceName] = useState('');
  const [deletePlan,setDeletePlan] = useState<WorkspaceDeletePreview|null>(null);
  const [deleteMode,setDeleteMode] = useState<'scan_only'|'all_files'>('scan_only');
  const [deleteError,setDeleteError] = useState('');
  const [now,setNow] = useState(Date.now());
  const [activityLoaded,setActivityLoaded] = useState(false);
  const [progress,setProgress] = useState<Progress|null>(null);
  const [busy,setBusy] = useState('');
  const [notice,setNotice] = useState<{text:string;error:boolean}|null>(null);
  const show = (text:string,error=false) => setNotice({text,error});
  const notify = useCallback((text:string,error=false) => setNotice({text,error}),[]);
  useEffect(()=>{try{localStorage.setItem('atlas.sidebar.collapsed',String(sidebarCollapsed))}catch{}},[sidebarCollapsed]);
  useEffect(()=>{if(!notice||notice.error)return;const timer=setTimeout(()=>setNotice(current=>current===notice?null:current),6000);return()=>clearTimeout(timer)},[notice]);
  const refreshActivity = useCallback(async () => { if (!isTauri) return; try { const [s,j,h]=await Promise.all([api.scanSession(),api.jobs(),api.scanSessions()]);setSession(s);setJobs(j);setHistory(h); } catch(e) { show(String(e),true); } finally { setActivityLoaded(true); } }, []);
  useEffect(() => { if (!isTauri) return; api.getProfile().then(setProfile).catch(e=>show(String(e),true));api.workspaces().then(setWorkspaces).catch(e=>show(String(e),true));refreshActivity(); },[refreshActivity]);
  useEffect(()=>{const timer=setInterval(()=>setNow(Date.now()),1000);return()=>clearInterval(timer)},[]);
  useEffect(() => { if (!isTauri) return; let off: (()=>void)|undefined; listen<Progress>('atlas-progress',ev=>{setProgress(ev.payload);refreshActivity();if(ev.payload.status==='error')show(ev.payload.message,true);}).then(fn=>off=fn); return ()=>off?.(); },[refreshActivity]);
  useEffect(() => { if (!isTauri) return;let active=true;const timer=setTimeout(()=>api.search(query,kind).then(rows=>{if(active)setResults(rows)}).catch(e=>{if(active)show(String(e),true)}),180); return ()=>{active=false;clearTimeout(timer)}; },[query,kind,session?.foldersDone,workspaces.find(w=>w.active)?.id]);
  useEffect(() => { if (!isTauri || view!=='search' || searchTab!=='downloaded') return;let active=true;const timer=setTimeout(()=>api.searchDownloaded(downloadedQuery).then(rows=>{if(active)setDownloadedResults(rows)}).catch(e=>{if(active)show(String(e),true)}),180);return()=>{active=false;clearTimeout(timer)};},[view,searchTab,downloadedQuery,jobs,workspaces.find(w=>w.active)?.id]);
  useEffect(() => {
    if (!isTauri || !activityLoaded || view!=='search' || searchTab!=='portal' || !query.trim() || !profile.hasCredential || session || autoScanStarted.current) return;
    autoScanStarted.current=true;
    api.startScan().then(setSession).catch(e=>show(String(e),true));
  },[activityLoaded,view,searchTab,query,profile.hasCredential,session]);
  const graph=useMemo<Graph|null>(()=>{
    if(!graphSeed)return null;
    const nodes=new Map(graphSeed.nodes.map(n=>[n.id,n]));
    const edges=new Map(graphSeed.edges.map(e=>[`${e.source}:${e.target}`,e]));
    const visited=new Set<string>();
    function addChildren(id:string){if(visited.has(id)||!expanded.has(id))return;visited.add(id);const page=children[id];if(!page)return;for(const n of page.nodes){nodes.set(n.id,n);if(n.kind==='folder')addChildren(n.id)}for(const e of page.edges)edges.set(`${e.source}:${e.target}`,e)}
    graphSeed.nodes.forEach(n=>{if(n.kind==='folder')addChildren(n.id)});
    return {nodes:[...nodes.values()],edges:[...edges.values()]};
  },[graphSeed,children,expanded]);
  const loadRoots=useCallback(async()=>{if(!isTauri)return;try{setGraphLoading('roots');const roots=await api.graphRoots();setGraphSeed(roots);setSelected(null);setExpanded(new Set());setChildren({});}catch(e){show(String(e),true)}finally{setGraphLoading('')}},[]);
  useEffect(()=>{if(view==='graph'&&!graphSeed)loadRoots()},[view,graphSeed,loadRoots]);
  const select = useCallback(async (node:AtlasNode, nextView:View='graph') => {setSelected(node);setView(nextView);setPlan(null);if(isTauri){try{setGraphLoading(node.id);setGraphSeed(await api.graphContext(node.id));setExpanded(new Set());setChildren({});}catch(e){show(String(e),true)}finally{setGraphLoading('')}}},[]);
  const toggleFolder=useCallback(async(id:string)=>{if(expanded.has(id)){setExpanded(prev=>{const next=new Set(prev);next.delete(id);return next});return}if(!children[id]){try{setGraphLoading(id);const page=await api.graphChildren(id,0);setChildren(prev=>({...prev,[id]:page}));}catch(e){show(String(e),true);return}finally{setGraphLoading('')}}setExpanded(prev=>new Set(prev).add(id))},[expanded,children]);
  const selectFromGraph = useCallback((id:string) => { const found=graph?.nodes.find(n=>n.id===id);if(found){setSelected(found);setPlan(null);if(found.kind==='folder')toggleFolder(id)} },[graph,toggleFolder]);
  async function loadMore(){if(!selected)return;const previous=children[selected.id];if(!previous?.hasMore)return;try{setGraphLoading(selected.id);const next=await api.graphChildren(selected.id,previous.nodes.length);setChildren(current=>({...current,[selected.id]:{nodes:[...previous.nodes,...next.nodes],edges:[...previous.edges,...next.edges],hasMore:next.hasMore}}));}catch(e){show(String(e),true)}finally{setGraphLoading('')}}
  async function save() {try{setBusy('save');setProfile(await api.saveProfile(profile,password));setPassword('');setResults([]);autoScanStarted.current=false;setWorkspaces(await api.workspaces());show('บันทึกการตั้งค่าแล้ว');await refreshActivity();}catch(e){show(String(e),true)}finally{setBusy('')}}
  async function switchSpace(id:string){if(workspaces.find(w=>w.active)?.id===id)return;try{setBusy('space');const next=await api.switchWorkspace(id);setProfile(next);setPassword('');setResults([]);setDownloadedResults([]);setSelected(null);setGraphSeed(null);setChildren({});setExpanded(new Set());setPlan(null);setProgress(null);setSession(null);setInspectedSession(null);setHistory([]);setJobs([]);setQuery('');setKind('');autoScanStarted.current=false;setWorkspaces(await api.workspaces());await refreshActivity();setView(next.baseUrl?'overview':'settings');}catch(e){show(String(e),true)}finally{setBusy('')}}
  async function createSpace(){try{setBusy('space');const spaces=await api.createWorkspace(spaceName);const created=spaces.find(w=>!workspaces.some(old=>old.id===w.id));setWorkspaces(spaces);setSpaceName('');if(created)await switchSpace(created.id);}catch(e){show(String(e),true)}finally{setBusy('')}}
  async function previewDeleteSpace(id:string){try{setBusy('delete-preview');setDeleteError('');setDeleteMode('scan_only');setDeletePlan(await api.previewWorkspaceDelete(id))}catch(e){show(String(e),true)}finally{setBusy('')}}
  async function deleteSpace(){if(!deletePlan)return;try{setBusy('delete-space');setDeleteError('');const spaces=await api.deleteWorkspace(deletePlan.workspaceId,deleteMode);setWorkspaces(spaces);setProfile(await api.getProfile());setDeletePlan(null);setResults([]);setDownloadedResults([]);setSelected(null);setGraphSeed(null);setChildren({});setExpanded(new Set());setPlan(null);setProgress(null);setInspectedSession(null);setQuery('');setKind('');autoScanStarted.current=false;await refreshActivity();show(deletePlan.isDefault?'ล้างข้อมูลพื้นที่หลักแล้ว':'ลบพื้นที่สแกนแล้ว')}catch(e){setDeleteError(String(e))}finally{setBusy('')}}
  async function test() {try{setBusy('test');show(await api.testConnection());}catch(e){show(String(e),true)}finally{setBusy('')}}
  async function scan() {try{setSession(await api.startScan());setProgress({kind:'scan',status:'running',current:0,total:0,message:'กำลังค้นหา Root Folder…'});}catch(e){show(String(e),true)}}
  async function pause() {try{await api.pauseScan();setSession(s=>s?{...s,status:'pausing'}:s);}catch(e){show(String(e),true)}}
  async function resume() {if(!session)return;try{setSession(await api.resumeScan(session.id));setProgress({kind:'scan',status:'running',current:session.foldersDone,total:session.foldersDone+session.foldersPending,message:'กำลังเชื่อมต่อ Portal เพื่อสแกนต่อ…'});}catch(e){show(String(e),true)}}
  async function chooseDirectory() {const path=await open({directory:true,multiple:false,title:'เลือกโฟลเดอร์ดาวน์โหลด'});if(typeof path==='string')setProfile({...profile,downloadDir:path});}
  async function preview(scope:string) {if(!selected)return;try{setPlan(await api.plan(selected.id,scope));}catch(e){show(String(e),true)}}
  async function download() {if(!plan)return;try{setBusy('download');await api.download(plan.nodeId,plan.scope);setPlan(null);setView('downloads');setProgress({kind:'download',status:'running',current:0,total:plan.items.length,message:'กำลังดาวน์โหลด…'});await refreshActivity();}catch(e){setBusy('');show(String(e),true)}}
  const nav: {id:View;label:string;icon:typeof LayoutDashboard}[]=[{id:'overview',label:'ภาพรวม',icon:LayoutDashboard},{id:'search',label:'ค้นหาเอกสาร',icon:Search},{id:'graph',label:'กราฟความสัมพันธ์',icon:GitBranch},{id:'downloads',label:'ดาวน์โหลด',icon:ArrowDownToLine},{id:'rag',label:'Document Index',icon:BookOpenText},{id:'chat',label:'ถาม–ตอบ ISO',icon:Bot},{id:'iso',label:'คัดแยก ISO',icon:ClipboardCheck},{id:'agent',label:'AI กำลังทำงาน',icon:Bot},{id:'spaces',label:'พื้นที่สแกน',icon:Database},{id:'settings',label:'ตั้งค่า',icon:Settings}];
  const folders=results.filter(n=>n.kind==='folder'),files=results.filter(n=>n.kind==='file');
  return <div className={`shell ${sidebarCollapsed?'sidebar-collapsed':''}`}>
    <aside className="sidebar" aria-label="เมนูหลัก"><div className="brand"><div className="brand-icon"><GitBranch size={22}/></div><div className="brand-copy"><strong>Document Atlas</strong><small>PORTAL EXPLORER</small></div></div><div className="workspace-label">WORKSPACE</div><nav>{nav.map(item=><button key={item.id} title={sidebarCollapsed?item.label:undefined} aria-label={item.label} aria-current={view===item.id?'page':undefined} className={view===item.id?'active':''} onClick={()=>setView(item.id)}><item.icon size={19}/><span>{item.label}</span></button>)}</nav><div className="sidebar-bottom"><div className="connection" title={sidebarCollapsed?(workspaces.find(w=>w.active)?.name||'พื้นที่หลัก'):undefined}><span className={profile.hasCredential?'status-dot':'status-dot off'}/><div><strong>{workspaces.find(w=>w.active)?.name||'พื้นที่หลัก'}</strong><small>{profile.baseUrl||'เพิ่ม Portal ในหน้าตั้งค่า'}</small></div></div><div className="sidebar-foot">LOCAL DOCUMENT SYSTEM <span>v0.7</span></div></div></aside>
    <main className="main"><header className="topbar"><div className="topbar-leading"><button className="sidebar-toggle" onClick={()=>setSidebarCollapsed(value=>!value)} aria-label={sidebarCollapsed?'ขยายเมนูด้านข้าง':'พับเมนูด้านข้าง'} aria-expanded={!sidebarCollapsed} title={sidebarCollapsed?'ขยายเมนู':'พับเมนู'}>{sidebarCollapsed?<PanelLeftOpen size={19}/>:<PanelLeftClose size={19}/>}</button><div className="breadcrumb">WORKSPACE <ChevronRight size={14}/> <strong>{nav.find(n=>n.id===view)?.label}</strong></div></div><div className="top-actions"><span className="local-pill"><span/> ดัชนีในเครื่อง</span><button className="avatar" onClick={()=>setView('settings')}>{profile.username?.charAt(0).toUpperCase()||'A'}</button></div></header>
      <div className="content">
        {notice&&<div className={`notice app-toast ${notice.error?'error':'success'}`} role={notice.error?'alert':'status'}><span className="notice-icon">{notice.error?<CircleAlert size={19}/>:<Check size={19}/>}</span><span className="notice-copy"><strong>{notice.error?'เกิดข้อผิดพลาด':'ดำเนินการสำเร็จ'}</strong><span>{notice.text}</span></span><button className="notice-close" aria-label="ปิดการแจ้งเตือน" onClick={()=>setNotice(null)}><X size={17}/></button></div>}
        {!isTauri&&<div className="notice notice-inline error"><span className="notice-icon"><CircleAlert size={18}/></span><span className="notice-copy"><strong>ต้องเปิดผ่านแอปเดสก์ท็อป</strong><span>ใช้ <code>npm run tauri dev</code> เพื่อเชื่อมฐานข้อมูลและ Portal</span></span></div>}
        {view==='overview'&&<>
          <div className="page-heading"><div><div className="eyebrow">DOCUMENT EXPLORER</div><h1>ข้อมูลใน Portal</h1><p>สำรวจโฟลเดอร์ทั้งหมดที่บัญชีนี้เข้าถึงได้ แล้วค้นหาเอกสารจากดัชนีในเครื่อง</p></div><button className="secondary" onClick={()=>{setSearchTab('portal');setView('search')}}><Search size={17}/> ค้นหาเอกสาร</button></div>
          <ScanPanel session={session} progress={progress} onStart={scan} onPause={pause} onResume={resume} now={now} disabled={!profile.hasCredential}/>
          <ScanConsole session={session}/>
          <div className="overview-grid"><section className="panel"><div className="panel-head"><div><h2>โฟลเดอร์ที่ค้นพบ</h2><p>แสดงจากดัชนีที่สแกนแล้ว</p></div><button className="text-button" onClick={()=>{setKind('folder');setView('search')}}>ดูทั้งหมด <ChevronRight size={16}/></button></div>{folders.slice(0,7).map(n=><button className="recent-row" key={n.id} onClick={()=>select(n)}><span className="file-icon folder-list"><Folder size={18}/></span><div><strong>{n.name}</strong><small>{n.path||short(n.id)}</small></div><ChevronRight size={16}/></button>)}{folders.length===0&&<div className="empty-small">เริ่มสำรวจ Portal เพื่อแสดงโฟลเดอร์</div>}</section><section className="panel recent-panel"><div className="panel-head"><div><h2>เอกสารที่ค้นพบ</h2><p>เลือกเพื่อดูกราฟและดาวน์โหลด</p></div><button className="text-button" onClick={()=>{setKind('file');setView('search')}}>ดูทั้งหมด <ChevronRight size={16}/></button></div>{files.slice(0,7).map(n=><button className="recent-row" key={n.id} onClick={()=>select(n)}><span className="file-icon"><FileText size={18}/></span><div><strong>{n.name}</strong><small>{n.path||short(n.id)}</small></div><ChevronRight size={16}/></button>)}{files.length===0&&<div className="empty-small">ยังไม่มีเอกสารในดัชนี</div>}</section></div>
        </>}
        {view==='search'&&<>
          <div className="page-heading"><div><div className="eyebrow">DOCUMENT SEARCH</div><h1>ค้นหาเอกสาร</h1><p>ค้นจากดัชนี Portal, ไฟล์ PDF ที่ดาวน์โหลดไว้ หรือความหมายของเนื้อหาเอกสาร</p></div></div>
          <div className="document-search-tabs" role="tablist" aria-label="หมวดการค้นหา"><button role="tab" aria-selected={searchTab==='portal'} className={searchTab==='portal'?'active':''} onClick={()=>setSearchTab('portal')}><Database size={17}/> ดัชนี Portal</button><button role="tab" aria-selected={searchTab==='downloaded'} className={searchTab==='downloaded'?'active':''} onClick={()=>setSearchTab('downloaded')}><ArrowDownToLine size={17}/> PDF ที่ดาวน์โหลด</button><button role="tab" aria-selected={searchTab==='content'} className={searchTab==='content'?'active':''} onClick={()=>setSearchTab('content')}><BookOpenText size={17}/> ค้นหาจากเนื้อหา</button></div>
          {searchTab==='portal'&&<><ScanPanel session={session} progress={progress} onStart={scan} onPause={pause} onResume={resume} now={now} disabled={!profile.hasCredential}/><ScanConsole session={session}/><section className="panel search-panel"><div className="search-box"><Search size={21}/><input value={query} onChange={e=>setQuery(e.target.value)} placeholder="ค้นหาเอกสาร โฟลเดอร์ หรือ ID..." autoFocus/></div><div className="filter-row">{[['','ทั้งหมด'],['folder','โฟลเดอร์'],['file','เอกสาร']].map(([value,label])=><button className={kind===value?'chosen':''} key={value} onClick={()=>setKind(value)}>{label}</button>)}<span>แสดง {results.length} รายการ</span></div><div className="result-list">{results.map(n=><button className="result-row" key={n.id} onClick={()=>select(n)}><span className={`result-icon ${n.kind}`}><NodeIcon kind={n.kind}/></span><div><strong>{n.name}</strong><small>{n.path||'ยังไม่มีเส้นทาง'}</small></div><span className="id-pill">{short(n.id)}</span><ChevronRight size={18}/></button>)}{results.length===0&&<div className="empty-state"><Search size={34}/><h3>ไม่พบเอกสาร</h3><p>{query?'ลองคำค้นอื่น หรือสแกนต่อเพื่อค้นหาเอกสารเพิ่ม':'เริ่มสำรวจ Portal เพื่อสร้างดัชนีทั้งระบบ'}</p></div>}</div></section></>}
          {searchTab==='downloaded'&&<section className="panel search-panel"><div className="panel-head"><div><h2>PDF ที่ดาวน์โหลดไว้</h2><p>ค้นจากไฟล์ที่ดาวน์โหลดผ่านแอปและยังอยู่บนเครื่อง</p></div><ArrowDownToLine size={19}/></div><div className="search-box"><Search size={21}/><input value={downloadedQuery} onChange={e=>setDownloadedQuery(e.target.value)} placeholder="ค้นหาชื่อเอกสาร เส้นทาง หรือชื่อไฟล์..." autoFocus/></div><div className="filter-row"><span>แสดง {downloadedResults.length} เอกสาร</span></div><div className="result-list">{downloadedResults.map(doc=><button className="result-row" key={doc.nodeId} onClick={()=>api.openDownloaded(doc.nodeId).catch(e=>show(String(e),true))}><span className="result-icon file"><FileText size={18}/></span><div><strong>{doc.name}</strong><small>{doc.path||doc.filePath}</small><small>{doc.filePath}</small></div><span className="id-pill">เปิด PDF</span><ChevronRight size={18}/></button>)}{downloadedResults.length===0&&<div className="empty-state"><FileText size={34}/><h3>ไม่พบ PDF ที่ดาวน์โหลด</h3><p>ดาวน์โหลดเอกสารจากกราฟหรือดัชนี Portal ก่อน</p></div>}</div></section>}
          {searchTab==='content'&&<RagPage mode="search" workspaceId={workspaces.find(w=>w.active)?.id||'default'} initialNode={selected} onSettings={()=>setView('settings')} onOpenNode={async id=>{try{const rows=await api.search(id);const found=rows.find(n=>n.id===id);if(found)await select(found)}catch(e){show(String(e),true)}}}/>}
        </>}
        {view==='graph'&&<>
          <div className="page-heading"><div><div className="eyebrow">RELATIONSHIP MAP</div><h1>กราฟความสัมพันธ์</h1><p>เริ่มที่ Folder หลัก · กด Folder เพื่อขยายหรือพับโหนดลูก · ลากโหนดเพื่อจัดตำแหน่งได้</p></div><button className="secondary" onClick={loadRoots} disabled={graphLoading==='roots'}><FolderOpen size={17}/> กลับโหนดหลัก</button></div>
          <div className="graph-layout"><section className="panel graph-panel"><div className="graph-toolbar"><div><GitBranch size={17}/><strong>{selected?.name||'โฟลเดอร์หลัก'}</strong></div><span>{graphLoading?'กำลังโหลดโหนด…':`${graph?.nodes.length||0} โหนด · ${graph?.edges.length||0} ความสัมพันธ์`}</span></div><GraphCanvas data={graph} selected={selected?.id||''} expanded={expanded} onSelect={selectFromGraph}/></section>
            <aside className="panel detail-panel">{selected?<><div className="detail-kicker">รายละเอียดโหนด</div><div className={`detail-icon ${selected.kind}`}><NodeIcon kind={selected.kind} size={26}/></div><h2>{selected.name}</h2><span className="type-tag">{selected.kind==='folder'?'Folder':'Document'}</span><div className="detail-fields"><div><small>PORTAL ID</small><strong>{selected.portal_id}</strong></div><div><small>ตำแหน่ง</small><strong>{selected.path||'—'}</strong></div><div><small>สถานะ</small><strong><span className="status-dot"/> พร้อมใช้งาน</strong></div></div><div className="detail-actions"><h3>สำรวจ</h3>{selected.kind==='folder'&&<><button className="secondary full" onClick={()=>toggleFolder(selected.id)} disabled={graphLoading===selected.id}><GitBranch size={17}/> {expanded.has(selected.id)?'พับโหนดลูก':'แสดงโหนดลูก'}</button>{expanded.has(selected.id)&&children[selected.id]?.hasMore&&<button className="secondary full" onClick={loadMore} disabled={graphLoading===selected.id}>แสดงโหนดลูกเพิ่ม ({children[selected.id].nodes.length} รายการแล้ว)</button>}</>}<h3>ดาวน์โหลด</h3>{selected.kind==='file'?<button className="primary full" onClick={()=>preview('document')}><ArrowDownToLine size={17}/> เลือกดาวน์โหลดเอกสาร</button>:<><button className="primary full" onClick={()=>preview('folder_direct')}><ArrowDownToLine size={17}/> ไฟล์ในโฟลเดอร์นี้</button><button className="secondary full" onClick={()=>preview('folder_subtree')}><GitBranch size={17}/> รวมโฟลเดอร์ย่อย</button></>}</div></>:<div className="empty-detail"><GitBranch size={32}/><h3>เลือก Folder หลัก</h3><p>กดโหนดในกราฟเพื่อดูรายละเอียดและขยายสาขา</p></div>}</aside></div>
        </>}
        {view==='downloads'&&<><div className="page-heading"><div><div className="eyebrow">DOWNLOAD CENTER</div><h1>ดาวน์โหลด</h1><p>ติดตามงานดาวน์โหลดเอกสารจาก Portal</p></div>{busy==='download'&&<button className="secondary" onClick={()=>api.cancelDownload()}><Square size={15}/> ยกเลิกงาน</button>}</div>{progress?.kind==='download'&&<div className="progress-panel"><div><strong>{progress.message}</strong><span>{progress.current}/{progress.total}</span></div><div className="progress-track"><span style={{width:`${progress.total?progress.current/progress.total*100:0}%`}}/></div></div>}<section className="panel"><div className="panel-head"><div><h2>ประวัติงาน</h2><p>เก็บไว้ในฐานข้อมูลบนเครื่องนี้</p></div></div>{jobs.map(job=><div className="job-row" key={job.id}><span className="job-icon"><ArrowDownToLine size={18}/></span><div><strong>งาน {job.id.slice(0,8)}</strong><small>{job.createdAt} · {job.status}</small></div><span>{job.completed}/{job.total} สำเร็จ{job.failed>0?` · ${job.failed} ล้มเหลว`:''}</span></div>)}{jobs.length===0&&<div className="empty-state"><ArrowDownToLine size={34}/><h3>ยังไม่มีงานดาวน์โหลด</h3><p>เลือกเอกสารจากหน้าค้นหาหรือกราฟเพื่อเริ่มต้น</p><button className="secondary" onClick={()=>setView('search')}>ค้นหาเอกสาร</button></div>}</section></>}
        {view==='spaces'&&<>
          <div className="page-heading"><div><div className="eyebrow">SCAN WORKSPACES</div><h1>พื้นที่สแกน</h1><p>แต่ละพื้นที่เก็บ Portal ดัชนี กราฟ และประวัติ Session แยกกัน</p></div></div>
          <section className="panel spaces-panel"><div className="panel-head"><div><h2>Portal ที่บันทึกไว้</h2><p>สลับพื้นที่เพื่อดูโครงสร้างเดิมหรือสแกนต่อ</p></div></div>
            {workspaces.map(space=><div className="space-row" key={space.id}><span className="file-icon folder-list"><Database size={18}/></span><div><strong>{space.name} {space.active&&<span className="space-current">กำลังใช้งาน</span>}</strong><small>{space.baseUrl||'ยังไม่ได้ตั้งค่า Portal'}{space.username?` · ${space.username}`:''}</small></div><button className="secondary" disabled={space.active||Boolean(busy)} onClick={()=>switchSpace(space.id)}>เปิดพื้นที่</button><button className="secondary space-delete-button" disabled={Boolean(busy)} onClick={()=>previewDeleteSpace(space.id)} title={space.id==='default'?'ล้างข้อมูลพื้นที่หลัก':'ลบพื้นที่สแกน'}><Trash2 size={15}/> {space.id==='default'?'ล้างข้อมูล':'ลบพื้นที่'}</button></div>)}
            <div className="space-create"><input value={spaceName} onChange={e=>setSpaceName(e.target.value)} placeholder="ชื่อพื้นที่ใหม่ เช่น Portal ฝ่ายบัญชี" maxLength={80}/><button className="primary" onClick={createSpace} disabled={!spaceName.trim()||Boolean(busy)}>เพิ่มพื้นที่สแกน</button></div>
          </section>
          <section className="panel session-history"><div className="panel-head"><div><h2>ประวัติการสแกนของพื้นที่นี้</h2><p>เปิดแอปใหม่แล้วกดสแกนต่อจากคิวที่บันทึกไว้ได้</p></div></div>
            {history.map(item=><div className="history-row" key={item.id}><div><strong>{statusLabel[item.status]||item.status}</strong><small>เริ่ม {item.startedAt} · Session {item.id.slice(0,8)}</small><small>{item.currentFolder||'ยังไม่พบ Root Folder'}</small></div><div><span>{item.foldersDone} โฟลเดอร์ · {item.filesSeen} เอกสาร</span><small>ใช้เวลา {duration(item.elapsedSeconds)}</small>{item.lastError&&<small className="history-error">{item.lastError}</small>}<button className="text-button history-log-button" onClick={()=>setInspectedSession(item)}><Terminal size={14}/> ดู Logs</button></div></div>)}
            {history.length===0&&<div className="empty-small">ยังไม่มีประวัติการสแกนในพื้นที่นี้</div>}
          </section>
          <ScanConsole session={inspectedSession||session}/>
        </>}
        {view==='agent'&&<AgentPage onNotice={notify}/>}
        {view==='chat'&&<IsoChatPage key={workspaces.find(w=>w.active)?.id||'default'} workspaceId={workspaces.find(w=>w.active)?.id||'default'}/>}
        {view==='iso'&&<IsoPage onNotice={notify} onDownloadStarted={()=>{setView('downloads');refreshActivity()}} onIndexStarted={()=>setView('rag')}/>}
        {view==='rag'&&<RagPage workspaceId={workspaces.find(w=>w.active)?.id||'default'} initialNode={selected} onSettings={()=>setView('settings')} onOpenNode={async id=>{try{const rows=await api.search(id);const found=rows.find(n=>n.id===id);if(found)await select(found)}catch(e){show(String(e),true)}}}/>}
        {view==='settings'&&<><div className="page-heading"><div><div className="eyebrow">CONFIGURATION</div><h1>ตั้งค่า Portal</h1><p>ระบุบัญชีครั้งเดียว แอปจะค้นหาโฟลเดอร์เริ่มต้นเอง</p></div></div><div className="settings-grid"><section className="panel form-panel"><div className="panel-head"><div><h2>การเชื่อมต่อ</h2><p>ข้อมูลบัญชีสำหรับเข้า Portal</p></div><span className="section-icon"><Settings size={19}/></span></div><label>Portal URL<input value={profile.baseUrl} onChange={e=>setProfile({...profile,baseUrl:e.target.value})} placeholder="http://portal.example.local"/></label><div className="two-fields"><label>ชื่อผู้ใช้<input value={profile.username} onChange={e=>setProfile({...profile,username:e.target.value})} placeholder="Username"/></label><label>รหัสผ่าน<input type="password" value={password} onChange={e=>setPassword(e.target.value)} placeholder={profile.hasCredential?'บันทึกไว้แล้ว · เว้นว่างเพื่อใช้เดิม':'Password'}/></label></div><div className="form-divider"/><div className="panel-head"><div><h2>ตำแหน่งดาวน์โหลด</h2><p>ไฟล์ PDF จะถูกเก็บในโฟลเดอร์นี้</p></div><span className="section-icon"><HardDrive size={19}/></span></div><div className="directory-row"><input readOnly value={profile.downloadDir} placeholder="ยังไม่ได้เลือกโฟลเดอร์"/><button className="secondary" onClick={chooseDirectory}>เลือกโฟลเดอร์</button></div><div className="form-actions"><button className="secondary" onClick={test} disabled={Boolean(busy)||!profile.hasCredential}>{busy==='test'?<LoaderCircle size={16} className="spin"/>:null} ทดสอบการเชื่อมต่อ</button><button className="primary" onClick={save} disabled={Boolean(busy)||!isTauri}>{busy==='save'?<LoaderCircle size={16} className="spin"/>:<Check size={16}/>} บันทึกการตั้งค่า</button></div></section><aside className="settings-aside"><div className="tip-card"><Database size={24}/><h3>ค้นหาโดยไม่ต้องระบุโฟลเดอร์</h3><p>แอปอ่าน Root Folder จาก Portal แล้วเดินสำรวจโหนดที่บัญชีนี้เข้าถึงได้ สามารถหยุดและสแกนต่อจาก session เดิมได้</p></div><div className="tip-card warning"><CircleAlert size={24}/><h3>การเชื่อมต่อ HTTP</h3><p>หาก Portal ใช้ HTTP ข้อมูลล็อกอินและเอกสารจะเดินทางโดยไม่มีการเข้ารหัส ควรใช้บนเครือข่ายที่เชื่อถือได้</p></div></aside></div><RagSettings workspaceId={workspaces.find(w=>w.active)?.id||'default'}/></>}
      </div>
    </main>
    {plan&&<div className="modal-backdrop" onMouseDown={e=>{if(e.target===e.currentTarget)setPlan(null)}}><div className="modal"><div className="modal-head"><span className="modal-icon"><ArrowDownToLine size={23}/></span><button onClick={()=>setPlan(null)}><X size={19}/></button></div><h2>ยืนยันการดาวน์โหลด</h2><p>ตรวจสอบรายการก่อนเริ่มดาวน์โหลดจาก Portal</p><div className="plan-summary"><div><small>ขอบเขต</small><strong>{scopeLabel(plan.scope)}</strong></div><div><small>จำนวนเอกสาร</small><strong>{plan.items.length} ไฟล์</strong></div><div><small>รายการซ้ำ</small><strong>{plan.duplicateCount} รายการ</strong></div><div><small>ปลายทาง</small><strong>{profile.downloadDir||'ยังไม่ได้เลือก'}</strong></div></div><div className="plan-preview">{plan.items.slice(0,5).map(n=><div key={n.id}><FileText size={15}/><span>{n.name}</span></div>)}{plan.items.length>5&&<small>และอีก {plan.items.length-5} ไฟล์</small>}</div><div className="modal-actions"><button className="secondary" onClick={()=>setPlan(null)}>ยกเลิก</button><button className="primary" onClick={download} disabled={plan.items.length===0||!profile.downloadDir}>เริ่มดาวน์โหลด <ArrowDownToLine size={17}/></button></div></div></div>}
    {deletePlan&&<div className="modal-backdrop" onMouseDown={e=>{if(e.target===e.currentTarget&&!busy)setDeletePlan(null)}}><div className="modal workspace-delete-modal" role="dialog" aria-modal="true" aria-labelledby="workspace-delete-title"><div className="modal-head"><span className="modal-icon danger"><Trash2 size={23}/></span><button aria-label="ปิด" disabled={Boolean(busy)} onClick={()=>setDeletePlan(null)}><X size={19}/></button></div><h2 id="workspace-delete-title">{deletePlan.isDefault?'ล้างข้อมูลพื้นที่หลัก':'ลบพื้นที่สแกน'} “{deletePlan.name}”</h2><p>{deletePlan.isDefault?'พื้นที่หลักและการตั้งค่า Portal จะยังอยู่':'พื้นที่นี้จะถูกนำออกจากรายการ และสลับไปพื้นที่หลักหากกำลังใช้งาน'}</p><div className="workspace-delete-counts"><span>{deletePlan.nodes} โหนด</span><span>{deletePlan.scanSessions} Session</span><span>{deletePlan.ragDocuments} เอกสาร RAG</span></div><div className="workspace-delete-options"><label className={deleteMode==='scan_only'?'selected':''}><input type="radio" name="delete-mode" checked={deleteMode==='scan_only'} onChange={()=>setDeleteMode('scan_only')}/><span><strong>ลบข้อมูลการสแกน</strong><small>ลบดัชนี กราฟ ข้อความ RAG และประวัติงานของพื้นที่นี้ · คง PDF ที่ดาวน์โหลดไว้</small></span></label><label className={deleteMode==='all_files'?'selected':''}><input type="radio" name="delete-mode" checked={deleteMode==='all_files'} onChange={()=>setDeleteMode('all_files')}/><span><strong>ลบข้อมูลและไฟล์ทั้งหมดของพื้นที่นี้</strong><small>รวม PDF ที่แอปดาวน์โหลดเอง {deletePlan.downloadFiles.length} ไฟล์ และแคช PDF {deletePlan.cacheFiles} ไฟล์</small></span></label></div>{deleteMode==='all_files'&&<div className="workspace-delete-file-list"><strong>ไฟล์ PDF ที่จะลบ</strong>{deletePlan.downloadFiles.length?deletePlan.downloadFiles.slice(0,8).map(path=><small key={path}>{path}</small>):<small>ไม่มีไฟล์ดาวน์โหลดที่ยืนยันได้</small>}{deletePlan.downloadFiles.length>8&&<small>และอีก {deletePlan.downloadFiles.length-8} ไฟล์</small>}<p>ไฟล์ที่ใช้ร่วมกับพื้นที่อื่น ไฟล์ที่แอปไม่ได้สร้าง และไฟล์โมเดล Embedding จะคงไว้</p></div>}<div className="modal-actions"><button className="secondary" disabled={Boolean(busy)} onClick={()=>setDeletePlan(null)}>ยกเลิก</button><button className="danger-button" disabled={Boolean(busy)} onClick={deleteSpace}>{busy==='delete-space'?<LoaderCircle size={16} className="spin"/>:<Trash2 size={16}/>} {deletePlan.isDefault?'ล้างข้อมูล':'ยืนยันลบพื้นที่'}</button></div></div></div>}
    {deletePlan&&deleteError&&<div className="delete-error-toast" role="alert"><CircleAlert size={16}/>{deleteError}</div>}
  </div>;
}
