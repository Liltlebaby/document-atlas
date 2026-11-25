import { useEffect, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, LoaderCircle, X, ZoomIn, ZoomOut } from 'lucide-react';
import { getDocument, GlobalWorkerOptions, type PDFDocumentProxy, type RenderTask } from 'pdfjs-dist';
import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url';
import { api } from './api';
import './pdf-reader.css';

GlobalWorkerOptions.workerSrc = workerUrl;

export default function PdfReader({ nodeId, name, initialPage = 1, onClose }: { nodeId: string; name: string; initialPage?: number; onClose: () => void }) {
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [page, setPage] = useState(initialPage);
  const [zoom, setZoom] = useState(1);
  const [width, setWidth] = useState(800);
  const [loading, setLoading] = useState(true);
  const [rendering, setRendering] = useState(false);
  const [hasPage, setHasPage] = useState(false);
  const [error, setError] = useState('');
  const dialog = useRef<HTMLElement>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let active = true;
    let task: ReturnType<typeof getDocument> | undefined;
    setLoading(true); setError(''); setPdf(null); setHasPage(false);
    api.readRagDocument(nodeId).then(bytes => {
      if (!active) return;
      task = getDocument({ data: new Uint8Array(bytes), useSystemFonts: true });
      return task.promise;
    }).then(value => {
      if (value && active) { setPage(Math.max(1, Math.min(initialPage, value.numPages))); setPdf(value); }
    }).catch(reason => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; void task?.destroy().catch(() => {}); };
  }, [nodeId, initialPage]);

  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const observer = new ResizeObserver(entries => {
      const nextWidth = Math.max(200, Math.floor(entries[0].contentRect.width - 48));
      setWidth(previous => previous === nextWidth ? previous : nextWidth);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!pdf || !canvas.current) return;
    let active = true;
    let task: RenderTask | undefined;
    setRendering(true); setError('');
    const target = canvas.current;
    // A separate canvas prevents rapid paging/zoom from drawing into an active render.
    const buffer = document.createElement('canvas');
    pdf.getPage(page).then(async value => {
      if (!active) return;
      const base = value.getViewport({ scale: 1 });
      const view = value.getViewport({ scale: Math.min(width / base.width, 1.5) * zoom });
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      buffer.width = Math.ceil(view.width * ratio); buffer.height = Math.ceil(view.height * ratio);
      const context = buffer.getContext('2d');
      if (!context) throw new Error('ไม่สามารถแสดงหน้า PDF ได้');
      task = value.render({ canvas: buffer, canvasContext: context, viewport: view, transform: [ratio, 0, 0, ratio, 0, 0] });
      await task.promise;
      if (!active) return;
      target.width = buffer.width; target.height = buffer.height;
      target.style.width = `${view.width}px`; target.style.height = `${view.height}px`;
      target.getContext('2d')?.drawImage(buffer, 0, 0);
      setHasPage(true);
      viewport.current?.scrollTo(0, 0);
    }).catch(reason => { if (active && reason?.name !== 'RenderingCancelledException') setError(String(reason)); })
      .finally(() => { if (active) setRendering(false); });
    return () => { active = false; task?.cancel(); };
  }, [pdf, page, width, zoom]);

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
      <header><div><span className="eyebrow">อ่านเอกสาร</span><h2 id="pdf-reader-title">{name}</h2></div><button className="secondary" aria-label="ปิดเอกสาร" onClick={onClose}><X size={20}/></button></header>
      <nav aria-label="ควบคุมหน้าเอกสาร"><button className="secondary" aria-label="หน้า PDF ก่อนหน้า" disabled={!pdf || page <= 1} onClick={() => setPage(p => p - 1)}><ChevronLeft size={18}/></button><span>หน้า {pdf ? page : '–'} / {pdf?.numPages || '–'}</span><button className="secondary" aria-label="หน้า PDF ถัดไป" disabled={!pdf || page >= pdf.numPages} onClick={() => setPage(p => p + 1)}><ChevronRight size={18}/></button><div className="pdf-reader-zoom"><button className="secondary" aria-label="ย่อหน้าเอกสาร" disabled={!pdf || zoom <= .5} onClick={() => setZoom(z => Math.max(.5, z - .25))}><ZoomOut size={17}/></button><span>{Math.round(zoom * 100)}%</span><button className="secondary" aria-label="ขยายหน้าเอกสาร" disabled={!pdf || zoom >= 2} onClick={() => setZoom(z => Math.min(2, z + .25))}><ZoomIn size={17}/></button></div><small>อ่านอย่างเดียว</small></nav>
      <div className="pdf-reader-pages" ref={viewport} aria-busy={loading || rendering}>
        {(loading || rendering) && <p className="pdf-reader-status" role="status"><LoaderCircle size={18} className="spin"/> กำลังแสดงเอกสาร…</p>}
        {error && <p className="pdf-reader-error" role="alert">เปิดเอกสารไม่สำเร็จ: {error}</p>}
        <canvas ref={canvas} role="img" aria-label={`เอกสารหน้า ${page}`} hidden={!hasPage || !!error}/>
      </div>
    </section>
  </div>;
}
