import { createCanvas } from '@napi-rs/canvas';
import { getDocument, AnnotationMode, VerbosityLevel } from 'pdfjs-dist/legacy/build/pdf.mjs';
import { fileURLToPath } from 'node:url';

const pdfjs = fileURLToPath(new URL('./node_modules/pdfjs-dist/', import.meta.url)).replaceAll('\\', '/');
const pageNumber = Number(process.argv[2]);
const chunks = [];
let size = 0;
for await (const chunk of process.stdin) {
  size += chunk.length;
  if (size > 100 * 1024 * 1024) throw Error('PDF exceeds size limit');
  chunks.push(chunk);
}
if (!Number.isSafeInteger(pageNumber) || pageNumber < 1) throw Error('Invalid page');
const task = getDocument({
  data: new Uint8Array(Buffer.concat(chunks)), verbosity: VerbosityLevel.ERRORS,
  useSystemFonts: true, enableXfa: false, isOffscreenCanvasSupported: false, isImageDecoderSupported: false,
  cMapUrl: `${pdfjs}cmaps/`, cMapPacked: true, standardFontDataUrl: `${pdfjs}standard_fonts/`, wasmUrl: `${pdfjs}wasm/`, useWorkerFetch: false,
});
try {
  const pdf = await task.promise;
  const page = await pdf.getPage(pageNumber);
  const base = page.getViewport({ scale: 1 });
  const viewport = page.getViewport({ scale: Math.min(1850 / base.width, 2500 / base.height, 3) });
  const canvas = createCanvas(Math.max(1, Math.ceil(viewport.width)), Math.max(1, Math.ceil(viewport.height)));
  await page.render({ canvas, canvasContext: canvas.getContext('2d'), viewport, annotationMode: AnnotationMode.DISABLE }).promise;
  process.stdout.write(await canvas.encode('png'));
} finally {
  await task.destroy();
}
