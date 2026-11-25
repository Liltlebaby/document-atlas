import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';
import { BookOpenText, ClipboardCheck, FileSearch, FileText, GitBranch, MessageSquareText, Search } from 'lucide-react';
import { api, webSession, type AtlasNode, type RagHit, type WebSession } from './api';
import { IsoChatPage } from './IsoChat';
import { IsoPage } from './Iso';
import { RagInspector } from './RagInspector';

const views = [
  { id: 'chat', label: 'ถาม–ตอบ ISO', short: 'ถาม–ตอบ', icon: MessageSquareText },
  { id: 'search', label: 'ค้นหาเอกสาร', short: 'ค้นเอกสาร', icon: FileSearch },
  { id: 'review', label: 'คัดแยก ISO', short: 'คัดแยก ISO', icon: ClipboardCheck },
] as const;

const mobileLayout = window.matchMedia('(max-width: 760px)');
function subscribeLayout(onChange: () => void) {
  mobileLayout.addEventListener('change', onChange);
  return () => mobileLayout.removeEventListener('change', onChange);
}

function DocumentSearch() {
  const [query, setQuery] = useState('');
  const [mode, setMode] = useState<'title' | 'content'>('title');
  const [hits, setHits] = useState<(AtlasNode | RagHit)[]>([]);
  const [searched, setSearched] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [selected, setSelected] = useState<{ id: string; page: number } | null>(null);
  const close = useCallback(() => setSelected(null), []);

  async function search() {
    const text = query.trim();
    if (!text || pending) return;
    setPending(true); setError(''); setHits([]);
    try {
      setHits(mode === 'title' ? await api.search(text, 'file') : await api.ragSearch(text));
      setSearched(true);
    } catch (reason) { setError(String(reason)); }
    finally { setPending(false); }
  }

  return <>
    <div className="page-heading"><div><div className="eyebrow">DOCUMENT INDEX</div><h1>ค้นหาเอกสาร</h1><p>ค้นด้วยชื่อ รหัสเอกสาร หรือเนื้อหาที่ทำดัชนี</p></div></div>
    <section className="panel web-search">
      <form onSubmit={event => { event.preventDefault(); void search(); }}>
        <label className="web-search-query"><span>คำค้น</span><span className="web-search-field"><Search size={19} aria-hidden="true" /><input enterKeyHint="search" value={query} onChange={event => setQuery(event.target.value)} maxLength={2000} placeholder="ชื่อ รหัส หรือหัวข้อเอกสาร" /></span></label>
        <label>ค้นจาก<select value={mode} disabled={pending} onChange={event => setMode(event.target.value as typeof mode)}><option value="title">ชื่อและรหัส</option><option value="content">เนื้อหา RAG</option></select></label>
        <button className="primary" disabled={!query.trim() || pending}><Search size={16} /> ค้นหา</button>
      </form>
      {pending && <p role="status">กำลังค้นหา…</p>}
      {error && <p className="notice notice-inline error" role="alert">{error}</p>}
      {!pending && !error && searched && <p className="web-search-count" role="status">{hits.length ? `พบ ${hits.length} ผลลัพธ์` : 'ไม่พบเอกสารที่ตรงคำค้น'}</p>}
      {hits.map(hit => {
        const semantic = 'chunkId' in hit;
        return <article key={semantic ? `chunk-${hit.chunkId}` : hit.id} className="web-result">
          <FileText className="web-result-icon" size={21} aria-hidden="true" />
          <div><strong>{hit.name}</strong><small>{hit.path}{semantic ? ` · หน้า ${hit.page}` : ''}</small>{semantic && <p>{hit.text}</p>}</div>
          <button className="secondary" onClick={() => setSelected({ id: semantic ? hit.nodeId : hit.id, page: semantic ? hit.page : 1 })}><BookOpenText size={15} /> ตรวจข้อมูล RAG</button>
        </article>;
      })}
    </section>
    {selected && <RagInspector key={`${selected.id}:${selected.page}`} nodeId={selected.id} initialPage={selected.page} onClose={close} />}
  </>;
}

export default function WebApp() {
  const compact = useSyncExternalStore(subscribeLayout, () => mobileLayout.matches);
  const [session, setSession] = useState<WebSession | null>(null);
  const [view, setView] = useState<(typeof views)[number]['id']>('chat');
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  const show = useCallback((text: string, error = false) => setNotice({ text, error }), []);
  useEffect(() => {
    let active = true;
    webSession().then(value => { if (active) setSession(value); }).catch(error => { if (active) show(String(error), true); });
    return () => { active = false; };
  }, [show]);
  return <div className="shell web-shell">
    <aside className="sidebar">
      <div className="brand"><span className="brand-icon"><GitBranch size={22}/></span><div><strong>Document Atlas</strong><small>COMPANY PORTAL</small></div></div>
      <nav aria-label="เมนูหลัก">{views.map(({ id, label, short, icon: Icon }) => <button key={id} aria-label={label} aria-current={view === id ? 'page' : undefined} className={view === id ? 'active' : ''} onClick={() => { setView(id); setNotice(null); window.scrollTo(0, 0); }}><Icon size={compact ? 21 : 18} aria-hidden="true"/><span>{compact ? short : label}</span></button>)}</nav>
    </aside>
    <main className="main">
      <header className="topbar">
        {compact ? <div className="web-mobile-brand"><span className="brand-icon"><GitBranch size={20} aria-hidden="true"/></span><strong>Document Atlas</strong></div> : <div className="breadcrumb">DOCUMENT ATLAS <strong>{views.find(item => item.id === view)?.label}</strong></div>}
        {session && <span className="local-pill" title={session.username}>{!compact && `${session.username} · `}{session.role === 'reviewer' ? 'ผู้อนุมัติ' : 'ผู้อ่าน'}</span>}
      </header>
      <div className="content">
        {notice && <p className={`notice notice-inline ${notice.error ? 'error' : ''}`} role={notice.error ? 'alert' : 'status'}>{notice.text}</p>}
        {!session && !notice && <p role="status">กำลังเปิดระบบ…</p>}
        {session && (view === 'chat' ? <IsoChatPage workspaceId={session.workspaceId} compact={compact} /> : view === 'search' ? <DocumentSearch /> : <IsoPage onNotice={show} session={session} compact={compact} />)}
      </div>
    </main>
  </div>;
}
