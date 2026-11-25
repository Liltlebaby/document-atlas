import { useEffect, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, FileText, LoaderCircle, X, ZoomIn, ZoomOut } from 'lucide-react';
import { api, pdfPage } from './api';
import './pdf-reader.css';

export default function PdfReader({ nodeId, name, initialPage = 1, onClose }: { nodeId: string; name: string; initialPage?: number; onClose: () => void }) {
  const [pages, setPages] = useState(0);
  const [page, setPage] = useState(initialPage);
  const [zoom, setZoom] = useState(1);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [image, setImage] = useState<{ url: string; page: number } | null>(null);
  const dialog = useRef<HTMLElement>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const imageUrl = useRef('');

  useEffect(() => {
    let active = true;
    api.ragDocumentDetail(nodeId).then(detail => {
      if (!active) return;
      if (detail.pageCount < 1) throw new Error('เอกสารนี้ยังไม่มีข้อมูลหน้า PDF');
      setPage(Math.max(1, Math.min(initialPage, detail.pageCount)));
      setPages(detail.pageCount);
    }).catch(reason => { if (active) { setError(String(reason)); setLoading(false); } });
    return () => { active = false; };
  }, [nodeId, initialPage]);

  useEffect(() => {
    if (!pages) return;
    const controller = new AbortController();
    setLoading(true); setError('');
    pdfPage(nodeId, page, controller.signal).then(async blob => {
      const url = URL.createObjectURL(blob);
      try {
        const picture = new Image();
        picture.src = url;
        await picture.decode();
        if (controller.signal.aborted) { URL.revokeObjectURL(url); return; }
        const previous = imageUrl.current;
        imageUrl.current = url;
        setImage({ url, page });
        if (previous) URL.revokeObjectURL(previous);
        viewport.current?.scrollTo(0, 0);
      } catch (reason) { URL.revokeObjectURL(url); throw reason; }
    }).catch(reason => { if (!controller.signal.aborted) setError(String(reason)); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [nodeId, pages, page]);

  useEffect(() => () => {
    if (imageUrl.current) URL.revokeObjectURL(imageUrl.current);
  }, []);

  useEffect(() => {
    const focus = document.activeElement as HTMLElement | null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden'; dialog.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && ['s', 'p'].includes(event.key.toLowerCase())) { event.preventDefault(); event.stopImmediatePropagation(); }
      if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); onClose(); }
      if (event.key === 'Tab') {
        const controls = dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled)');
        if (!controls?.length) return;
        const first = controls[0], last = controls[controls.length - 1];
        if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog.current)) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && (document.activeElement === last || document.activeElement === dialog.current)) { event.preventDefault(); first.focus(); }
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => { document.body.style.overflow = overflow; window.removeEventListener('keydown', onKey, true); focus?.focus(); };
  }, [onClose]);

  return <div className="pdf-reader-backdrop" onContextMenu={event => event.preventDefault()}>
    <section className="pdf-reader" ref={dialog} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="pdf-reader-title">
      <header><FileText className="pdf-reader-mobile-icon" size={23} aria-hidden="true"/><div><span className="eyebrow">อ่านเอกสาร</span><h2 id="pdf-reader-title">{name}</h2><small className="pdf-reader-mobile-readonly">อ่านอย่างเดียว</small></div><button className="secondary" aria-label="ปิดเอกสาร" onClick={onClose}><X size={20}/></button></header>
      <nav aria-label="ควบคุมหน้าเอกสาร">
        <div className="pdf-reader-page-nav">
          <button className="secondary" aria-label="หน้า PDF ก่อนหน้า" disabled={!pages || page <= 1} onClick={() => setPage(p => p - 1)}><ChevronLeft size={18}/></button>
          <span>หน้า {pages ? page : '–'} / {pages || '–'}</span>
          <button className="secondary" aria-label="หน้า PDF ถัดไป" disabled={!pages || page >= pages} onClick={() => setPage(p => p + 1)}><ChevronRight size={18}/></button>
        </div>
        <div className="pdf-reader-zoom"><button className="secondary" aria-label="ย่อหน้าเอกสาร" disabled={!pages || zoom <= .5} onClick={() => setZoom(z => Math.max(.5, z - .25))}><ZoomOut size={17}/></button><span>{Math.round(zoom * 100)}%</span><button className="secondary" aria-label="ขยายหน้าเอกสาร" disabled={!pages || zoom >= 2} onClick={() => setZoom(z => Math.min(2, z + .25))}><ZoomIn size={17}/></button></div><small>อ่านอย่างเดียว</small>
      </nav>
      <div className="pdf-reader-pages" ref={viewport} aria-busy={loading}>
        {(loading) && <p className="pdf-reader-status" role="status"><LoaderCircle size={18} className="spin"/> กำลังแสดงเอกสาร…</p>}
        {error && <p className="pdf-reader-error" role="alert">เปิดเอกสารไม่สำเร็จ: {error}</p>}
        {image && !error && <img src={image.url} alt={`เอกสารหน้า ${image.page}`} draggable={false}
          style={{ width: `${zoom * 100}%`, maxWidth: `${Math.round(900 * zoom)}px` }} />}
      </div>
    </section>
  </div>;
}
