import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ArrowDownToLine, BookOpenText, Check, ChevronDown, ChevronRight, ClipboardCheck, ExternalLink, FileDown, FilePlus2, FileUp, Folder, GitCompareArrows, LoaderCircle, Lock, RotateCw, Send, Settings2, X } from 'lucide-react';
import { api, type IsoDecision, type IsoDocument, type IsoOverview, type IsoStatus, type SimilarDoc, type SimilarVerdict } from './api';
import { exportReviewCsv, type WebSession } from './api';
const PdfReader = lazy(() => import('./PdfReader'));
import './iso.css';
import { RagInspector } from './RagInspector';

const statusLabel: Record<IsoStatus, string> = { ACTIVE: 'ใช้งานปัจจุบัน', ARCHIVE: 'Revision เก่า', DUPLICATE: 'เอกสารซ้ำ', REVIEW: 'รอเจ้าของตัดสิน' };
const decisionLabel: Record<IsoDecision, string> = { pending: 'รออนุมัติ', approved: 'อนุมัติ', rejected: 'ไม่อนุมัติ' };
const verdictLabel: Record<SimilarVerdict, string> = {
  duplicate: 'เนื้อหาเกือบเหมือนกัน',
  contained: 'เนื้อหาฉบับนี้อยู่ในฉบับนั้นเกือบทั้งหมด',
  contains: 'ฉบับนั้นอยู่ในฉบับนี้เกือบทั้งหมด',
  related: 'เกี่ยวข้อง ควรให้ AI เทียบ',
  different: 'ต่างกัน',
};
const REVIEWER_KEY = 'iso-reviewer';
const CASE_DIR_KEY = 'iso-case-dir';

function readStored(key: string) { try { return localStorage.getItem(key) || ''; } catch { return ''; } }
function writeStored(key: string, value: string) { try { localStorage.setItem(key, value); } catch { /* storage unavailable */ } }
function thaiDate(value: string) { if (!value) return '—'; const d = new Date(value); return Number.isNaN(d.getTime()) ? value : d.toLocaleDateString('th-TH', { day: 'numeric', month: 'short', year: '2-digit' }); }
const pct = (n: number) => `${Math.round(n * 100)}%`;
const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

function SimilarList({ items, sourceLabel, onOpen }: { items: SimilarDoc[]; sourceLabel: string; onOpen: (nodeId: string) => void }) {
  if (items.length === 0) return <div className="empty-small">ยังไม่มีผลเทียบ กด “หาคู่คล้าย” ที่การ์ดกลุ่มก่อน (ต้องทำดัชนี RAG แล้ว)</div>;
  return <div className="iso-similar">{items.map(s => <div key={s.nodeId} className={`iso-similar-item ${s.verdict}`}>
    <div className="iso-similar-head">
      <span className="iso-rank">#{s.rank}</span>
      <div><strong>{s.name}</strong><small>{s.path}</small></div>
      <button className="text-button" onClick={() => onOpen(s.nodeId)}><ExternalLink size={13} /> เปิด</button>
    </div>
    <div className="iso-scores">
      <span className={`iso-verdict ${s.verdict}`}>{verdictLabel[s.verdict]}</span>
      <span title={`เนื้อหาของ${sourceLabel}ที่พบในฉบับนี้`}>{sourceLabel}→ฉบับนี้ <b>{pct(s.forward)}</b></span>
      <span title={`เนื้อหาของฉบับนี้ที่พบใน${sourceLabel}`}>ฉบับนี้→{sourceLabel} <b>{pct(s.backward)}</b></span>
    </div>
    {s.evidence.map((e, i) => <div key={i} className="iso-evidence">
      <div><small>{sourceLabel} · หน้า {e.sourcePage}</small><p>{e.sourceText}</p></div>
      <div><small>ฉบับนี้ · หน้า {e.otherPage} · ตรงกัน {pct(e.score)}</small><p>{e.otherText}</p></div>
    </div>)}
  </div>)}</div>;
}

export function IsoPage({ onNotice, session, compact = false }: { onNotice: (text: string, error?: boolean) => void; session: WebSession; compact?: boolean }) {
  const [inspectId, setInspectId] = useState<string | null>(null);
  const closeInspector = useCallback(() => setInspectId(null), []);
  const [overview, setOverview] = useState<IsoOverview | null>(null);
  const [department, setDepartment] = useState('');
  const [docs, setDocs] = useState<IsoDocument[]>([]);
  const [filter, setFilter] = useState<IsoStatus | 'ALL' | 'UNREVIEWED'>('ALL');
  const canReview = session.role === 'reviewer';
  const [reader, setReader] = useState<{ id: string; name: string } | null>(null);
  const closeReader = useCallback(() => setReader(null), []);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [similar, setSimilar] = useState<Record<string, SimilarDoc[] | 'loading'>>({});
  const [busy, setBusy] = useState('');
  const documentSection = useRef<HTMLElement>(null);

  const refresh = useCallback(async () => {
    try { setOverview(await api.isoOverview()); } catch (e) { onNotice(String(e), true); }
  }, [onNotice]);
  const loadDocs = useCallback(async (id: string) => {
    if (!id) { setDocs([]); return; }
    try { setDocs(await api.isoDocuments(id)); } catch (e) { onNotice(String(e), true); }
  }, [onNotice]);
  useEffect(() => { refresh(); }, [refresh]);
  useEffect(() => {
    if (!department) return;
    let active = true;
    api.isoDocuments(department).then(result => { if (active) setDocs(result); }).catch(error => { if (active) onNotice(String(error), true); });
    return () => { active = false; };
  }, [department, onNotice]);
  useEffect(() => {
    if (compact && department) documentSection.current?.scrollIntoView({ block: 'start' });
  }, [compact, department]);

  function chooseDepartment(id: string) {
    setFilter('ALL');
    if (id === department) {
      if (compact) documentSection.current?.scrollIntoView({ block: 'start' });
      return;
    }
    setDocs([]);
    setSimilar({});
    setDepartment(id);
  }

  async function run(label: string, task: () => Promise<void>) {
    try { setBusy(label); await task(); } catch (e) { onNotice(String(e), true); } finally { setBusy(''); }
  }
  async function exportCsv() {
    await run('export', async () => {
      const url = URL.createObjectURL(await exportReviewCsv());
      const link = document.createElement('a');
      link.href = url;
      link.download = 'iso-review-decisions.csv';
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    });
  }
  async function toggleSimilar(doc: IsoDocument) {
    if (similar[doc.nodeId]) { const next = { ...similar }; delete next[doc.nodeId]; setSimilar(next); return; }
    setSimilar(s => ({ ...s, [doc.nodeId]: 'loading' }));
    try { const items = await api.isoSimilar(doc.nodeId); setSimilar(s => ({ ...s, [doc.nodeId]: items })); }
    catch (e) { onNotice(String(e), true); setSimilar(s => { const next = { ...s }; delete next[doc.nodeId]; return next; }); }
  }
  async function decide(doc: IsoDocument, decision: IsoDecision) {
    if (!canReview || busy) return;
    await run('decision', async () => {
      await api.isoDecide(doc.nodeId, decision, notes[doc.nodeId] ?? doc.decisionNote, session.username);
      await loadDocs(department); await refresh();
    });
  }
  function openPdf(doc: IsoDocument) { setReader({ id: doc.nodeId, name: doc.title || doc.name }); }
  function openIndexed(nodeId: string) { setInspectId(nodeId); }

  const selected = overview?.departments.find(d => d.id === department);
  const visible = useMemo(() => docs.filter(d => filter === 'ALL' || (filter === 'UNREVIEWED' ? !d.status : d.status === filter)), [docs, filter]);
  const counts = useMemo(() => ({ ALL: docs.length, UNREVIEWED: docs.filter(d => !d.status).length, ACTIVE: docs.filter(d => d.status === 'ACTIVE').length, ARCHIVE: docs.filter(d => d.status === 'ARCHIVE').length, DUPLICATE: docs.filter(d => d.status === 'DUPLICATE').length, REVIEW: docs.filter(d => d.status === 'REVIEW').length }), [docs]);
  const spin = (label: string, icon: ReactNode) => busy === label ? <LoaderCircle size={15} className="spin" /> : icon;
  const groups = <div className="iso-groups">{overview?.groups.map(g => <section key={g.id} className="panel iso-group">
    <div className="iso-group-head"><span>กลุ่ม {g.id}</span><small>ครบกำหนด {thaiDate(g.deadline)}</small></div>
    <h2>{g.label}</h2>
    <div className="iso-group-stats"><div><strong>{g.documents}</strong><small>เอกสาร · {g.departments} แผนก</small></div><div><strong>{g.projected}</strong><small>คาดว่าเหลือ</small></div><div><strong>{g.target}</strong><small>เป้า</small></div></div>
    <div className="iso-track" aria-label={`ทำดัชนี RAG แล้ว ${g.indexed} จาก ${g.documents}`}><span style={{ width: `${g.documents ? (g.indexed / g.documents) * 100 : 0}%` }} /></div>
    <small className="iso-group-meta">PDF {g.downloaded}/{g.documents} · RAG {g.indexed}/{g.documents} · คัดแยกแล้ว {g.reviewed}</small>
  </section>)}</div>;

  return <div className="iso-page">
    <div className="page-heading">
      <div><div className="eyebrow">ISO DOCUMENT REVIEW</div><h1>คัดแยกเอกสาร ISO</h1><p>{compact ? 'จัดกลุ่มเอกสารตามแผนกและสถานะ' : overview ? `${overview.branch} · เป้าหมายเหลือ ${Math.round(overview.targetRatio * 100)}% ของเดิม · ระบบเสนอเท่านั้น ไม่ลบหรือย้ายไฟล์บน Portal` : 'นำเข้าไฟล์ตั้งค่าแผนกเพื่อเริ่มคัดแยก'}</p></div>
      {canReview && <button className="primary" onClick={exportCsv} disabled={Boolean(busy) || !overview}>{spin('export', <FileDown size={16} />)} ส่งออกผลอนุมัติ</button>}
    </div>
    {!overview && <section className="panel iso-empty"><ClipboardCheck size={34}/><h3>ยังไม่มีข้อมูลคัดแยก</h3><p>ติดต่อ IT เพื่อตรวจชุดข้อมูลบนเซิร์ฟเวอร์</p></section>}

    {overview && <>
      {!compact && groups}

      <section className="panel iso-table-panel">
        <div className="panel-head"><div><h2>สรุปรายแผนก</h2>{!compact && <p>เลือกแผนกเพื่อตรวจและอนุมัติรายฉบับ · “คาดว่าเหลือ” นับ ARCHIVE/DUPLICATE ออก และนับเอกสารที่รวมเข้าฉบับเดียวกันเป็น 1</p>}</div><button className="text-button iso-refresh" onClick={refresh} aria-label="รีเฟรชสรุปรายแผนก"><RotateCw size={16} /><span>รีเฟรช</span></button></div>
        {compact ? <div className="iso-department-list">{overview.departments.map(d => (
          <button type="button" className="iso-department" key={d.id} aria-pressed={d.id === department} onClick={() => chooseDepartment(d.id)}>
            <Folder size={21} aria-hidden="true" />
            <span className="iso-department-main">
              <strong>{d.id}</strong>
              <small>เอกสาร {d.found.toLocaleString()} รายการ</small>
              <span className="iso-department-counts">
                {d.active > 0 && <span className="iso-chip active">ใช้งาน {d.active}</span>}
                {d.archive > 0 && <span className="iso-chip archive">ฉบับเก่า {d.archive}</span>}
                {d.duplicate > 0 && <span className="iso-chip duplicate">ซ้ำ {d.duplicate}</span>}
                {d.review > 0 && <span className="iso-chip review">รอตรวจ {d.review}</span>}
              </span>
            </span>
            <ChevronRight size={19} aria-hidden="true" />
          </button>
        ))}</div> : <div className="iso-scroll"><table className="iso-table">
          <thead><tr><th>แผนก</th><th>กลุ่ม</th><th>ตาราง</th><th>พบ</th><th>PDF</th><th>RAG</th><th>ACTIVE</th><th>ARCHIVE</th><th>DUPLICATE</th><th>REVIEW</th><th>อนุมัติ / รอ</th><th>คาดว่าเหลือ</th><th>เป้า</th></tr></thead>
          <tbody>{overview.departments.map(d => <tr key={d.id} className={d.id === department ? 'chosen' : ''} onClick={() => chooseDepartment(d.id)}>
            <td><strong>{d.id}</strong></td><td>{d.group}</td><td>{d.memoTotal || '—'}</td><td className={d.memoTotal && d.found !== d.memoTotal ? 'iso-mismatch' : ''} title={d.memoTotal && d.found !== d.memoTotal ? 'จำนวนที่พบบน Portal ไม่ตรงกับตารางสรุป' : undefined}>{d.found}</td><td>{d.downloaded}</td><td>{d.indexed}</td>
            <td>{d.active || '—'}</td><td>{d.archive || '—'}</td><td>{d.duplicate || '—'}</td><td>{d.review || '—'}</td><td>{d.approved + d.rejected} / {d.pending}</td>
            <td><strong className={d.projected <= d.target ? 'iso-ok' : ''}>{d.projected}</strong></td><td>{d.target}</td></tr>)}</tbody>
        </table></div>}
      </section>
      {compact && overview.groups.length > 0 && <details className="iso-group-summary"><summary>สรุปตามกลุ่ม</summary>{groups}</details>}

      {selected && <section className="panel iso-docs" ref={documentSection}>
        <div className="panel-head"><div><h2>{selected.id} · {docs.length} ฉบับ</h2><p>ผลคัดแยกเป็นข้อเสนอ ต้องให้เจ้าของเอกสารหรือผู้รับผิดชอบอนุมัติทุกฉบับ</p></div>
          {canReview && <span className="local-pill">ผู้อนุมัติ {session.username}</span>}</div>
        <div className="filter-row">{(['ALL', 'UNREVIEWED', 'ACTIVE', 'ARCHIVE', 'DUPLICATE', 'REVIEW'] as const).map(f => <button key={f} className={filter === f ? 'chosen' : ''} onClick={() => setFilter(f)}>{f === 'ALL' ? 'ทั้งหมด' : f === 'UNREVIEWED' ? 'ยังไม่คัดแยก' : f} {counts[f]}</button>)}</div>
        <div className="iso-doc-list">{visible.map(doc => { const sim = similar[doc.nodeId]; return <article key={doc.nodeId} className="iso-doc">
          <div className="iso-doc-main">
            <div className="iso-doc-title">{doc.status ? <span className={`iso-chip ${doc.status.toLowerCase()}`} title={statusLabel[doc.status]}>{doc.status}</span> : <span className="iso-chip none">ยังไม่คัดแยก</span>}<strong>{doc.code || doc.name}</strong>{doc.revision && <small>Rev. {doc.revision}</small>}{doc.docType && <small>{doc.docType}</small>}{doc.protected && <span className="iso-protected" title="เอกสารตามตารางข้อกำหนด ISO 9001 ห้ามเสนอรวมหรือยกเลิก"><Lock size={11} /> ห้ามยุบ</span>}</div>
            <p className="iso-doc-name">{doc.title || doc.name}</p>
            {doc.reason && <p className="iso-reason">{doc.reason}</p>}
            {doc.mergeInto && <p className="iso-merge">เสนอรวมเป็น: <strong>{doc.mergeInto}</strong></p>}
            {doc.isoClauses.length > 0 && <div className="iso-clauses">{doc.isoClauses.map(c => <span key={c}>{c}</span>)}</div>}
            <small className="iso-path">{doc.path}</small>
            <button className="text-button iso-similar-toggle" onClick={() => toggleSimilar(doc)} aria-expanded={Boolean(sim)}>{sim === 'loading' ? <LoaderCircle size={13} className="spin" /> : sim ? <ChevronDown size={13} /> : <ChevronRight size={13} />} เอกสารคล้าย (RAG)</button>
            {sim && sim !== 'loading' && <SimilarList items={sim} sourceLabel="ฉบับที่เลือก" onOpen={openIndexed} />}
          </div>
          <div className="iso-doc-side">
            <button className="text-button" onClick={() => setInspectId(doc.nodeId)}><BookOpenText size={14} /> ตรวจข้อมูล RAG</button>
            {doc.downloaded && <button className="text-button" onClick={() => openPdf(doc)}><ExternalLink size={14} /> อ่าน PDF</button>}
            {doc.status && <>
              <span className={`iso-decision ${doc.decision}`}>{decisionLabel[doc.decision]}{doc.decidedBy && ` · ${doc.decidedBy}`}</span>
              {canReview && <><input className="iso-note" value={notes[doc.nodeId] ?? doc.decisionNote} onChange={e => setNotes({ ...notes, [doc.nodeId]: e.target.value })} placeholder="หมายเหตุ" />
              <div className="iso-decide">
                <button className="secondary" aria-label="อนุมัติข้อเสนอ" disabled={Boolean(busy) || doc.decision === 'approved'} onClick={() => decide(doc, 'approved')}><Check size={14} /> อนุมัติ</button>
                <button className="secondary" aria-label="ไม่อนุมัติข้อเสนอ" disabled={Boolean(busy) || doc.decision === 'rejected'} onClick={() => decide(doc, 'rejected')}><X size={14} /> ไม่อนุมัติ</button>
                {doc.decision !== 'pending' && <button className="text-button" onClick={() => decide(doc, 'pending')}>ยกเลิก</button>}
              </div></>}
            </>}
          </div>
        </article>; })}{visible.length === 0 && <div className="empty-small">ไม่มีเอกสารในตัวกรองนี้</div>}</div>
      </section>}
    </>}
    {reader && <Suspense fallback={<p role="status">กำลังเปิด PDF…</p>}><PdfReader key={reader.id} nodeId={reader.id} name={reader.name} onClose={closeReader} /></Suspense>}
    {inspectId && <RagInspector key={inspectId} nodeId={inspectId} onClose={closeInspector} />}
  </div>;
}
