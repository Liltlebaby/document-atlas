import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { BookOpenText, ChevronLeft, ChevronRight, LoaderCircle, X } from 'lucide-react';
import { api, type RagDocumentDetail, type RagStoredChunk } from './api';
import './rag-inspector.css';

const PdfReader = lazy(() => import('./PdfReader'));

const labels: Record<string, string> = { ready: 'พร้อมค้นหา', partial_text: 'ข้อความบางหน้า', needs_ocr: 'ต้องใช้ OCR', error: 'เกิดข้อผิดพลาด', stale: 'รอทำดัชนีใหม่' };

export function RagInspector({ nodeId, onClose, initialPage = 1 }: { nodeId: string; onClose: () => void; initialPage?: number }) {
  const [detail, setDetail] = useState<RagDocumentDetail | null>(null);
  const [page, setPage] = useState(1);
  const [chunks, setChunks] = useState<RagStoredChunk[]>([]);
  const [loading, setLoading] = useState(true);
  const [pageLoading, setPageLoading] = useState(false);
  const [error, setError] = useState('');
  const [pageError, setPageError] = useState('');
  const [reading, setReading] = useState(false);
  const closeReader = useCallback(() => setReading(false), []);
  const dialog = useRef<HTMLElement>(null);

  useEffect(() => {
    let active = true;
    setDetail(null); setPage(1); setLoading(true); setError('');
    api.ragDocumentDetail(nodeId).then(value => { if (active) { setPage(Math.max(1, Math.min(initialPage, value.pageCount || 1))); setDetail(value); } })
      .catch(e => { if (active) setError(String(e)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [nodeId, initialPage]);

  useEffect(() => {
    let active = true;
    setChunks([]); setPageError('');
    if (!detail || detail.pageCount < 1) return;
    setPageLoading(true);
    api.ragDocumentPage(nodeId, page).then(rows => { if (active) setChunks(rows); })
      .catch(e => { if (active) setPageError(String(e)); })
      .finally(() => { if (active) setPageLoading(false); });
    return () => { active = false; };
  }, [nodeId, page, detail]);

  useEffect(() => {
    if (reading) return;
    const previousFocus = document.activeElement as HTMLElement | null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    dialog.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); onClose(); }
      if (event.key === 'Tab') {
        const elements = dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled), select, summary, [tabindex="0"]');
        if (!elements?.length) return;
        const first = elements[0], last = elements[elements.length - 1];
        if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog.current)) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && (document.activeElement === last || document.activeElement === dialog.current)) { event.preventDefault(); first.focus(); }
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => { document.body.style.overflow = previousOverflow; window.removeEventListener('keydown', onKey, true); previousFocus?.focus(); };
  }, [onClose, reading]);

  const current = detail?.pages.find(p => p.page === page);
  if (reading) return <Suspense fallback={<div className="pdf-reader-backdrop" role="status">กำลังเปิดตัวอ่านเอกสาร…</div>}><PdfReader nodeId={nodeId} name={detail?.name || "เอกสาร PDF"} initialPage={page} onClose={closeReader}/></Suspense>;
  return <div className="rag-inspector-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <section ref={dialog} tabIndex={-1} className="rag-inspector" role="dialog" aria-modal="true" aria-labelledby="rag-inspector-title">
      <header className="rag-inspector-header"><div><span className="eyebrow">ตรวจข้อมูล RAG รายไฟล์</span><h2 id="rag-inspector-title">{detail?.name || 'ข้อมูลที่เก็บจาก PDF'}</h2></div><button className="secondary" aria-label="ปิดหน้าตรวจข้อมูล RAG" onClick={onClose}><X size={20}/></button></header>
      <div className="rag-inspector-body">
        {loading ? <p role="status"><LoaderCircle size={18} className="spin"/> กำลังอ่านข้อมูล RAG…</p> : error ? <div className="rag-inspector-warning" role="alert">{error}</div> : detail && <>
          <p className="rag-inspector-path">{detail.path}</p>
          <div className="rag-inspector-stats"><span><strong>{labels[detail.status] || detail.status}</strong>สถานะดัชนี</span><span><strong>{detail.pageCount}</strong>หน้า PDF</span><span><strong>{detail.chunkCount}</strong>ช่วงข้อความ</span><span><strong>{detail.vectorCount}/{detail.chunkCount}</strong>ช่วงที่มีเวกเตอร์</span><span><strong>{detail.pages.filter(p => p.chunkCount === 0).length}</strong>หน้าที่ไม่มีช่วงข้อความ</span></div>
          <p className="rag-inspector-help">ข้อความด้านล่างอ่านจากดัชนีที่บันทึกไว้จริง ใช้ตรวจคำภาษาไทย ตัวเลข และเลขหน้าเทียบกับ PDF ต้นฉบับ สถานะพร้อมค้นหาไม่ได้ยืนยันว่าข้อความถูกต้องทั้งหมด</p>
          {detail.warning && <div className="rag-inspector-warning">{detail.warning}</div>}
          {detail.vectorCount !== detail.chunkCount && <div className="rag-inspector-warning">จำนวนเวกเตอร์ไม่ครบทุกช่วงข้อความ บางช่วงอาจไม่ถูกค้นพบด้วย RAG</div>}
          <details className="rag-inspector-metadata"><summary>ข้อมูลอ้างอิงของดัชนี</summary><p>ทำดัชนีล่าสุด: {detail.updatedAt}</p><p>รหัสไฟล์: <code>{detail.nodeId}</code></p><p>SHA-256 ของ PDF ที่ทำดัชนี: <code>{detail.sha256 || 'ยังไม่มีค่า'}</code></p><p>หน้าที่ตัวอ่านแจ้งว่าไม่มีข้อความหรือสั้นเกินไป: {detail.emptyPages}</p></details>
          {detail.pageCount > 0 ? <>
            <nav className="rag-inspector-paging" aria-label="เลือกหน้า PDF"><button className="secondary" aria-label="หน้าก่อนหน้า" disabled={page <= 1} onClick={() => setPage(p => p - 1)}><ChevronLeft size={17}/></button><label>หน้า PDF <select value={page} onChange={event => setPage(Number(event.target.value))}>{detail.pages.map(p => <option key={p.page} value={p.page}>หน้า {p.page} · {p.chunkCount} ช่วง{p.chunkCount === 0 ? ' · ไม่มีข้อความ' : ''}</option>)}</select></label><button className="secondary" aria-label="หน้าถัดไป" disabled={page >= detail.pageCount} onClick={() => setPage(p => p + 1)}><ChevronRight size={17}/></button><span>{current?.chunkCount || 0} ช่วง · {current?.characterCount || 0} ตัวอักษรรวมช่วงซ้อน</span></nav>
            <div className="rag-inspector-chunks" aria-live="polite" aria-busy={pageLoading}>
              {pageLoading ? <p role="status"><LoaderCircle size={18} className="spin"/> กำลังโหลดหน้า {page}…</p> : pageError ? <p role="alert" className="rag-inspector-warning">{pageError}</p> : chunks.length ? chunks.map((chunk, index) => <article className="rag-inspector-chunk" key={chunk.chunkId}><header><strong>หน้า {chunk.page} · ช่วงที่ {index + 1}</strong><span>Chunk #{chunk.chunkId} · {Array.from(chunk.text).length} ตัวอักษร · {chunk.hasEmbedding ? 'มีเวกเตอร์' : 'ไม่มีเวกเตอร์'}</span></header><p>{chunk.text}</p></article>) : <div className="rag-inspector-empty"><BookOpenText size={24}/><strong>หน้า {page} ไม่มีช่วงข้อความในดัชนี</strong><p>หน้านี้อาจเป็นภาพสแกน ข้อความสั้นกว่าเกณฑ์ หรือยังดึงข้อความไม่สำเร็จ กรุณาเทียบกับ PDF ก่อนพิจารณาทำ OCR หรือทำดัชนีใหม่</p></div>}
            </div>
            <p className="rag-inspector-help">แต่ละช่วงมีข้อความทับซ้อนกันเพื่อช่วยค้นหา จึงอาจเห็นข้อความซ้ำบริเวณรอยต่อ และการจัดบรรทัดอาจต่างจาก PDF</p>
          </> : <p className="rag-inspector-warning">ยังไม่มีข้อมูลหน้า PDF กรุณาตรวจสถานะการทำดัชนีของไฟล์นี้</p>}
        </>}
      </div>
      <footer className="rag-inspector-footer"><span>อ่าน PDF เพื่อเทียบกับหน้าเดียวกัน ระบบตรวจ SHA-256 ให้ตรงกับฉบับที่ทำดัชนี</span><button className="primary" disabled={!detail?.sha256} onClick={()=>setReading(true)}><BookOpenText size={16}/> อ่าน PDF ต้นฉบับ</button></footer>
    </section>
  </div>;
}
