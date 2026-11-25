import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { ArrowDownToLine, BookOpenText, Check, ChevronDown, ChevronRight, ClipboardCheck, ExternalLink, FileDown, FilePlus2, FileUp, GitCompareArrows, LoaderCircle, Lock, RotateCw, Send, Settings2, X } from 'lucide-react';
import { api, type IsoDecision, type IsoDocument, type IsoOverview, type IsoStatus, type SimilarDoc, type SimilarVerdict } from './api';
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

export function IsoPage({ onNotice, onDownloadStarted, onIndexStarted }: { onNotice: (text: string, error?: boolean) => void; onDownloadStarted: () => void; onIndexStarted: () => void }) {
  const [inspectId, setInspectId] = useState<string | null>(null);
  const closeInspector = useCallback(() => setInspectId(null), []);
  const [overview, setOverview] = useState<IsoOverview | null>(null);
  const [department, setDepartment] = useState('');
  const [docs, setDocs] = useState<IsoDocument[]>([]);
  const [filter, setFilter] = useState<IsoStatus | 'ALL' | 'UNREVIEWED'>('ALL');
  const [reviewer, setReviewer] = useState(() => readStored(REVIEWER_KEY));
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [similar, setSimilar] = useState<Record<string, SimilarDoc[] | 'loading'>>({});
  const [check, setCheck] = useState<{ path: string; items: SimilarDoc[] } | null>(null);
  const [busy, setBusy] = useState('');

  const refresh = useCallback(async () => {
    try { setOverview(await api.isoOverview()); } catch (e) { onNotice(String(e), true); }
  }, [onNotice]);
  const loadDocs = useCallback(async (id: string) => {
    if (!id) { setDocs([]); return; }
    try { setDocs(await api.isoDocuments(id)); } catch (e) { onNotice(String(e), true); }
  }, [onNotice]);
  useEffect(() => { refresh(); }, [refresh]);
  useEffect(() => { loadDocs(department); setSimilar({}); }, [department, loadDocs]);

  async function run(label: string, task: () => Promise<void>) {
    try { setBusy(label); await task(); } catch (e) { onNotice(String(e), true); } finally { setBusy(''); }
  }
  async function importConfig() {
    const path = await open({ multiple: false, title: 'เลือกไฟล์ตั้งค่าแผนก (departments.json)', filters: [{ name: 'JSON', extensions: ['json'] }] });
    if (typeof path === 'string') await run('config', async () => { setOverview(await api.isoImportConfig(path)); onNotice('นำเข้าไฟล์ตั้งค่าแผนกแล้ว'); });
  }
  async function importResults() {
    const path = await open({ multiple: false, title: 'เลือกไฟล์ผลคัดแยก (iso_results_*.json)', filters: [{ name: 'JSON', extensions: ['json'] }] });
    if (typeof path === 'string') await run('results', async () => {
      const r = await api.isoImportResults(path);
      onNotice(`นำเข้าผลคัดแยก ${r.imported} ฉบับ${r.skipped ? ` · ข้าม ${r.skipped} ฉบับที่ไม่อยู่ในดัชนี` : ''}${r.resetDecisions ? ` · ${r.resetDecisions} ฉบับข้อเสนอเปลี่ยน ต้องอนุมัติใหม่` : ''}`);
      await refresh(); await loadDocs(department);
    });
  }
  async function exportCsv() {
    const path = await save({ title: 'บันทึกรายงานผลอนุมัติ', defaultPath: 'iso-review-decisions.csv', filters: [{ name: 'CSV', extensions: ['csv'] }] });
    if (path) await run('export', async () => onNotice(`บันทึกรายงานที่ ${await api.isoExport(path)}`));
  }
  const download = (group: number) => run(`download-${group}`, async () => { await api.isoDownload(group); onDownloadStarted(); });
  const index = (group: number) => run(`index-${group}`, async () => {
    const job = await api.isoStartIndex(group);
    onNotice(`เริ่มทำดัชนี RAG ${job.total} ฉบับ ดูความคืบหน้าที่เมนู Document Index`);
    onIndexStarted();
  });
  const compare = (group: number) => run(`compare-${group}`, async () => {
    const r = await api.isoComputeSimilarity(group);
    onNotice(`หาคู่คล้ายแล้ว ${r.indexedTargets}/${r.targets} ฉบับ (เทียบกับ ${r.poolDocuments} ฉบับในดัชนี · ${r.seconds.toFixed(1)} วินาที)${r.indexedTargets < r.targets ? ` · อีก ${r.targets - r.indexedTargets} ฉบับยังไม่มีดัชนี RAG` : ''}`);
    setSimilar({});
  });
  async function toggleSimilar(doc: IsoDocument) {
    if (similar[doc.nodeId]) { const next = { ...similar }; delete next[doc.nodeId]; setSimilar(next); return; }
    setSimilar(s => ({ ...s, [doc.nodeId]: 'loading' }));
    try { const items = await api.isoSimilar(doc.nodeId); setSimilar(s => ({ ...s, [doc.nodeId]: items })); }
    catch (e) { onNotice(String(e), true); setSimilar(s => { const next = { ...s }; delete next[doc.nodeId]; return next; }); }
  }
  async function checkNewDocument() {
    const path = await open({ multiple: false, title: 'เลือก PDF เอกสารใหม่ที่จะออก', filters: [{ name: 'PDF', extensions: ['pdf'] }] });
    if (typeof path === 'string') await run('check', async () => { setCheck(null); setCheck({ path, items: await api.isoCheckNewDocument(path) }); });
  }
  async function sendToAi() {
    if (!check) return;
    const dir = await open({ directory: true, multiple: false, title: 'เลือกโฟลเดอร์รับงานของ AI (เช่น iso-review/inbox)', defaultPath: readStored(CASE_DIR_KEY) || undefined });
    if (typeof dir !== 'string') return;
    writeStored(CASE_DIR_KEY, dir);
    await run('case', async () => onNotice(`บันทึกชุดเปรียบเทียบแล้ว: ${await api.isoWriteCase(dir, check.path, check.items)} · สั่ง Claude Code “ตรวจเอกสารใหม่ใน inbox”`));
  }
  async function decide(doc: IsoDocument, decision: IsoDecision) {
    if (decision !== 'pending' && !reviewer.trim()) { onNotice('กรุณาระบุชื่อผู้อนุมัติก่อน', true); return; }
    try {
      await api.isoDecide(doc.nodeId, decision, notes[doc.nodeId] ?? doc.decisionNote, reviewer);
      await loadDocs(department); await refresh();
    } catch (e) { onNotice(String(e), true); }
  }
  async function openPdf(doc: IsoDocument) { try { await api.openDownloaded(doc.nodeId); } catch (e) { onNotice(String(e), true); } }
  async function openIndexed(nodeId: string) { try { await api.openRagDocument(nodeId); } catch (e) { onNotice(String(e), true); } }

  const selected = overview?.departments.find(d => d.id === department);
  const visible = useMemo(() => docs.filter(d => filter === 'ALL' || (filter === 'UNREVIEWED' ? !d.status : d.status === filter)), [docs, filter]);
  const counts = useMemo(() => ({ ALL: docs.length, UNREVIEWED: docs.filter(d => !d.status).length, ACTIVE: docs.filter(d => d.status === 'ACTIVE').length, ARCHIVE: docs.filter(d => d.status === 'ARCHIVE').length, DUPLICATE: docs.filter(d => d.status === 'DUPLICATE').length, REVIEW: docs.filter(d => d.status === 'REVIEW').length }), [docs]);
  const spin = (label: string, icon: ReactNode) => busy === label ? <LoaderCircle size={15} className="spin" /> : icon;

  return <div className="iso-page">
    <div className="page-heading">
      <div><div className="eyebrow">ISO DOCUMENT REVIEW</div><h1>คัดแยกเอกสาร ISO</h1><p>{overview ? `${overview.branch} · เป้าหมายเหลือ ${Math.round(overview.targetRatio * 100)}% ของเดิม · ระบบเสนอเท่านั้น ไม่ลบหรือย้ายไฟล์บน Portal` : 'นำเข้าไฟล์ตั้งค่าแผนกเพื่อเริ่มคัดแยก'}</p></div>
      <div className="iso-actions">
        <button className="secondary" onClick={importConfig} disabled={Boolean(busy)}>{spin('config', <Settings2 size={16} />)} ตั้งค่าแผนก</button>
        <button className="secondary" onClick={importResults} disabled={Boolean(busy) || !overview}>{spin('results', <FileUp size={16} />)} นำเข้าผลคัดแยก</button>
        <button className="primary" onClick={exportCsv} disabled={Boolean(busy) || !overview}>{spin('export', <FileDown size={16} />)} ส่งออกผลอนุมัติ</button>
      </div>
    </div>

    {!overview && <section className="panel iso-empty"><ClipboardCheck size={34} /><h3>ยังไม่มีไฟล์ตั้งค่าแผนก</h3><p>เลือกไฟล์ <code>iso-review/config/departments.json</code> ซึ่งระบุแผนก กลุ่ม กำหนดเวลา และโฟลเดอร์บน Portal ของแต่ละแผนก</p><button className="primary" onClick={importConfig}><Settings2 size={16} /> นำเข้าไฟล์ตั้งค่าแผนก</button></section>}

    {overview && <>
      <div className="iso-groups">{overview.groups.map(g => <section key={g.id} className="panel iso-group">
        <div className="iso-group-head"><span>กลุ่ม {g.id}</span><small>ครบกำหนด {thaiDate(g.deadline)}</small></div>
        <h2>{g.label}</h2>
        <div className="iso-group-stats"><div><strong>{g.documents}</strong><small>เอกสาร · {g.departments} แผนก</small></div><div><strong>{g.projected}</strong><small>คาดว่าเหลือ</small></div><div><strong>{g.target}</strong><small>เป้า</small></div></div>
        <div className="iso-track" aria-label={`ทำดัชนี RAG แล้ว ${g.indexed} จาก ${g.documents}`}><span style={{ width: `${g.documents ? (g.indexed / g.documents) * 100 : 0}%` }} /></div>
        <small className="iso-group-meta">PDF {g.downloaded}/{g.documents} · RAG {g.indexed}/{g.documents} · คัดแยกแล้ว {g.reviewed}</small>
        <div className="iso-group-buttons">
          <button className="secondary" onClick={() => download(g.id)} disabled={Boolean(busy) || g.downloaded === g.documents} title="ดาวน์โหลด PDF ให้ AI อ่าน">{spin(`download-${g.id}`, <ArrowDownToLine size={15} />)} PDF</button>
          <button className="secondary" onClick={() => index(g.id)} disabled={Boolean(busy) || g.indexed === g.documents} title="แปลงเอกสารเป็นเวกเตอร์ด้วยโมเดล Embedding">{spin(`index-${g.id}`, <BookOpenText size={15} />)} ทำดัชนี RAG</button>
          <button className="secondary" onClick={() => compare(g.id)} disabled={Boolean(busy) || g.indexed === 0} title="หาเอกสารที่คล้ายที่สุด 3 อันดับของทุกฉบับในกลุ่ม">{spin(`compare-${g.id}`, <GitCompareArrows size={15} />)} หาคู่คล้าย</button>
        </div>
      </section>)}</div>

      <section className="panel iso-check">
        <div className="panel-head">
          <div><h2>ตรวจเอกสารใหม่ก่อนออก</h2><p>เลือก PDF ที่จะออกใหม่ ระบบหาเอกสารที่คล้ายที่สุด 3 อันดับจากดัชนี RAG ของ {overview.branch} แล้วส่งให้ AI ตัดสินว่าควรออกใหม่ แก้ฉบับเดิม รวม หรือไม่ต้องออก</p></div>
          <div className="iso-actions">
            <button className="secondary" onClick={() => index(0)} disabled={Boolean(busy)} title="ทำดัชนีเอกสารทุกแผนกที่ยังไม่มีเวกเตอร์ เพื่อให้เทียบได้ครบ">{spin('index-0', <BookOpenText size={15} />)} ทำดัชนีทั้งขอบเขต</button>
            <button className="primary" onClick={checkNewDocument} disabled={Boolean(busy)}>{spin('check', <FilePlus2 size={15} />)} เลือก PDF</button>
          </div>
        </div>
        {check && <div className="iso-check-body">
          <div className="iso-check-file"><span>เอกสารใหม่: <strong>{fileName(check.path)}</strong></span><button className="secondary" onClick={sendToAi} disabled={Boolean(busy) || check.items.length === 0}>{spin('case', <Send size={15} />)} ส่งให้ AI วิเคราะห์</button></div>
          <SimilarList items={check.items} sourceLabel="เอกสารใหม่" onOpen={openIndexed} />
        </div>}
      </section>

      <section className="panel iso-table-panel">
        <div className="panel-head"><div><h2>สรุปรายแผนก</h2><p>เลือกแผนกเพื่อตรวจและอนุมัติรายฉบับ · “คาดว่าเหลือ” นับ ARCHIVE/DUPLICATE ออก และนับเอกสารที่รวมเข้าฉบับเดียวกันเป็น 1</p></div><button className="text-button" onClick={refresh}><RotateCw size={14} /> รีเฟรช</button></div>
        <div className="iso-scroll"><table className="iso-table">
          <thead><tr><th>แผนก</th><th>กลุ่ม</th><th>ตาราง</th><th>พบ</th><th>PDF</th><th>RAG</th><th>ACTIVE</th><th>ARCHIVE</th><th>DUPLICATE</th><th>REVIEW</th><th>อนุมัติ / รอ</th><th>คาดว่าเหลือ</th><th>เป้า</th></tr></thead>
          <tbody>{overview.departments.map(d => <tr key={d.id} className={d.id === department ? 'chosen' : ''} onClick={() => { setDepartment(d.id); setFilter('ALL'); }}>
            <td><strong>{d.id}</strong></td><td>{d.group}</td><td>{d.memoTotal || '—'}</td><td className={d.memoTotal && d.found !== d.memoTotal ? 'iso-mismatch' : ''} title={d.memoTotal && d.found !== d.memoTotal ? 'จำนวนที่พบบน Portal ไม่ตรงกับตารางสรุป' : undefined}>{d.found}</td><td>{d.downloaded}</td><td>{d.indexed}</td>
            <td>{d.active || '—'}</td><td>{d.archive || '—'}</td><td>{d.duplicate || '—'}</td><td>{d.review || '—'}</td><td>{d.approved + d.rejected} / {d.pending}</td>
            <td><strong className={d.projected <= d.target ? 'iso-ok' : ''}>{d.projected}</strong></td><td>{d.target}</td></tr>)}</tbody>
        </table></div>
      </section>

      {selected && <section className="panel iso-docs">
        <div className="panel-head"><div><h2>{selected.id} · {docs.length} ฉบับ</h2><p>ผลคัดแยกเป็นข้อเสนอ ต้องให้เจ้าของเอกสารหรือผู้รับผิดชอบอนุมัติทุกฉบับ</p></div>
          <label className="iso-reviewer">ผู้อนุมัติ<input value={reviewer} onChange={e => { setReviewer(e.target.value); writeStored(REVIEWER_KEY, e.target.value); }} placeholder="ชื่อ / ตำแหน่ง" /></label></div>
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
            {doc.downloaded && <button className="text-button" onClick={() => openPdf(doc)}><ExternalLink size={14} /> เปิด PDF</button>}
            {doc.status && <>
              <span className={`iso-decision ${doc.decision}`}>{decisionLabel[doc.decision]}{doc.decidedBy && ` · ${doc.decidedBy}`}</span>
              <input className="iso-note" value={notes[doc.nodeId] ?? doc.decisionNote} onChange={e => setNotes({ ...notes, [doc.nodeId]: e.target.value })} placeholder="หมายเหตุ" />
              <div className="iso-decide">
                <button className="secondary" aria-label="อนุมัติข้อเสนอ" disabled={doc.decision === 'approved'} onClick={() => decide(doc, 'approved')}><Check size={14} /> อนุมัติ</button>
                <button className="secondary" aria-label="ไม่อนุมัติข้อเสนอ" disabled={doc.decision === 'rejected'} onClick={() => decide(doc, 'rejected')}><X size={14} /> ไม่อนุมัติ</button>
                {doc.decision !== 'pending' && <button className="text-button" onClick={() => decide(doc, 'pending')}>ยกเลิก</button>}
              </div>
            </>}
          </div>
        </article>; })}{visible.length === 0 && <div className="empty-small">ไม่มีเอกสารในตัวกรองนี้</div>}</div>
      </section>}
    </>}
    {inspectId && <RagInspector key={inspectId} nodeId={inspectId} onClose={closeInspector} />}
  </div>;
}
