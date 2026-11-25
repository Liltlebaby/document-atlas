import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { BookOpen, CircleAlert, Clock, Settings2 } from 'lucide-react';
import { api, type AgentStatus, type IsoOverview } from './api';
import officeBg from './assets/iso-office.webp';
import frontA from './assets/agent/front_a.png';
import frontB from './assets/agent/front_b.png';
import backA from './assets/agent/back_a.png';
import backB from './assets/agent/back_b.png';
import backStand from './assets/agent/back_stand.png';
import sideA from './assets/agent/side_a.png';
import sideB from './assets/agent/side_b.png';
import sideCarry from './assets/agent/side_carry.png';
import seatRead from './assets/agent/seat_read.png';
import seatThink from './assets/agent/seat_think.png';
import seatCheer from './assets/agent/seat_cheer.png';
import seatSleep from './assets/agent/seat_sleep.png';
import './agent.css';

// Scene coordinates are pixels of the 1672×941 background (iso-office.webp).
const IMG_W = 1672, IMG_H = 941;
const PLATE_FIRST = 133.5, PLATE_PITCH = 88.73, PLATE_Y = 147.5, PLATE_W = 64, SHELVES = 16;
// Character frames cut from the sprite sheet (scripts in the session: cut_sprites.py). Standing/walking frames share
// one 152×261 canvas with the feet at the bottom centre; seated frames end at the desk edge.
const K = 0.37;                                     // sprite pixels → background pixels
const WALK_W = 152 * K, WALK_H = 261 * K;
const SEAT_X = 355, SEAT_DESK = 438;                // chair centre and where the open book meets the table
const STAND = { x: 355, y: 404 };                   // feet when standing up from the chair
const AISLE_Y = 357;                                // feet in the aisle in front of the shelves
const TRIP_MS = 11000;                              // one walk to the shelf and back; keep in sync with px-trip in agent.css
const plateX = (i: number) => PLATE_FIRST + i * PLATE_PITCH;
const stageLabel: Record<string, string> = { starting: 'กำลังเตรียมตัว', reading: 'กำลังอ่านเอกสาร', judging: 'กำลังตัดสินการรวมเอกสาร', done: 'อ่านจบแผนกแล้ว', error: 'ติดปัญหา' };
const SEATED = {
  read: { src: seatRead, w: 140, h: 172, cx: 67 }, think: { src: seatThink, w: 124, h: 177, cx: 57.5 },
  cheer: { src: seatCheer, w: 189, h: 183, cx: 105.5 }, sleep: { src: seatSleep, w: 178, h: 183, cx: 97.5 },
};

/** Small pixel badges (check mark, arrow, thought bubble). */
function Pixels({ rows, x = 0, y = 0, palette }: { rows: string[]; x?: number; y?: number; palette: Record<string, string> }) {
  const rects: ReactNode[] = [];
  rows.forEach((row, ry) => {
    for (let cx = 0; cx < row.length;) {
      const c = row[cx];
      if (c === '.' || !palette[c]) { cx++; continue; }
      let len = 1;
      while (row[cx + len] === c) len++;
      rects.push(<rect key={`${ry}-${cx}`} x={x + cx} y={y + ry} width={len} height={1} fill={palette[c]} />);
      cx += len;
    }
  });
  return <g>{rects}</g>;
}
/** A standing/walking frame anchored at the feet (0,0). */
const Frame = ({ src, className }: { src: string; className?: string }) => <image href={src} className={className} x={-WALK_W / 2} y={-WALK_H} width={WALK_W} height={WALK_H} />;
/** A seated frame placed on the reading-table chair. */
function Seated({ kind, className }: { kind: keyof typeof SEATED; className?: string }) {
  const f = SEATED[kind];
  return <image href={f.src} className={className} x={SEAT_X - f.cx * K} y={SEAT_DESK - f.h * K} width={f.w * K} height={f.h * K} />;
}

function minutesAgo(value?: string) {
  if (!value) return Infinity;
  const t = new Date(value).getTime();
  return Number.isNaN(t) ? Infinity : (Date.now() - t) / 60000;
}
function clock(value?: string) { if (!value) return '—'; const d = new Date(value); return Number.isNaN(d.getTime()) ? value : d.toLocaleTimeString('th-TH', { hour: '2-digit', minute: '2-digit' }); }
function shortName(name: string) {
  const short = name.replace(/^STR\s+/, '').replace(/\s+SX$/, '');
  return short.length > 7 ? short.slice(0, 6) : short;
}

export function AgentPage({ onNotice }: { onNotice: (text: string, error?: boolean) => void }) {
  const [status, setStatus] = useState<AgentStatus | null>(null);
  const [overview, setOverview] = useState<IsoOverview | null>(null);
  const [problem, setProblem] = useState('');
  const [, tick] = useState(0);

  useEffect(() => {
    let alive = true;
    const poll = async () => {
      try { const s = await api.isoAgentStatus(); if (alive) { setStatus(s); setProblem(''); } }
      catch (e) { if (alive) setProblem(String(e)); }
    };
    const loadOverview = () => api.isoOverview().then(o => alive && setOverview(o)).catch(() => undefined);
    poll(); loadOverview();
    const a = window.setInterval(poll, 3000), b = window.setInterval(loadOverview, 30000), c = window.setInterval(() => tick(t => t + 1), 15000);
    return () => { alive = false; window.clearInterval(a); window.clearInterval(b); window.clearInterval(c); };
  }, []);

  async function relinkConfig() {
    const path = await open({ multiple: false, title: 'เลือกไฟล์ตั้งค่าแผนก (iso-review/config/departments.json)', filters: [{ name: 'JSON', extensions: ['json'] }] });
    if (typeof path !== 'string') return;
    try { setOverview(await api.isoImportConfig(path)); setStatus(await api.isoAgentStatus()); setProblem(''); onNotice('เชื่อมโฟลเดอร์ iso-review แล้ว'); }
    catch (e) { onNotice(String(e), true); }
  }

  const departments = useMemo(() => (overview?.departments || []).filter(d => d.group >= 1 && d.group <= 3).slice(0, SHELVES - 1), [overview]);
  const finished = useMemo(() => new Map((status?.finished || []).map(f => [f.department, f])), [status]);
  const active = status && status.stage !== 'done' && status.stage !== 'error';
  const stale = Boolean(active && minutesAgo(status?.updatedAt) > 10);
  const currentIndex = departments.findIndex(d => d.id === status?.department);
  const shelfX = currentIndex >= 0 ? plateX(currentIndex) : plateX(4);
  const outward = shelfX >= STAND.x ? 1 : -1;
  const mode: 'walk' | 'think' | 'happy' | 'sad' | 'sleep' = !status || stale ? 'sleep' : status.stage === 'reading' || status.stage === 'starting' ? 'walk' : status.stage === 'judging' ? 'think' : status.stage === 'error' ? 'sad' : 'happy';
  // Fetch a binder only when the script really starts a new batch; the rest of the time the agent reads at the table.
  const batchKey = status && mode === 'walk' ? `${status.department}#${status.batch}` : '';
  const lastBatch = useRef('');
  const [trip, setTrip] = useState(0);
  const [tripping, setTripping] = useState(false);
  useEffect(() => {
    if (!batchKey) { lastBatch.current = ''; setTripping(false); return; }
    const seen = lastBatch.current;
    lastBatch.current = batchKey;
    if (!seen || seen === batchKey) return;   // first look at the page: the agent is already reading this batch
    setTrip(t => t + 1);
    setTripping(true);
    const done = window.setTimeout(() => setTripping(false), TRIP_MS);
    return () => window.clearTimeout(done);
  }, [batchKey]);
  const progress = status?.batches ? Math.min(1, (status.stage === 'judging' || status.stage === 'done' ? status.batches : Math.max(0, status.batch - 1)) / status.batches) : 0;
  const finishedList = [...finished.values()];
  const totals = finishedList.reduce((a, f) => ({ before: a.before + f.before, after: a.after + f.after }), { before: 0, after: 0 });
  const queue = departments.filter(d => !finished.has(d.id) && d.id !== status?.department);
  const approved = (overview?.departments || []).reduce((n, d) => n + d.approved, 0);
  const stage = status?.stage;
  const bubble = !status ? 'ยังไม่มีงาน… ขอพักจิบกาแฟก่อนนะ' : stale ? `เงียบไป ${Math.round(minutesAgo(status.updatedAt))} นาทีแล้ว ช่วยดูหน้าต่างสคริปต์หน่อย` :
    stage === 'reading' ? `${status.message || 'กำลังอ่าน'} · แผนก ${status.department}` : stage === 'judging' ? `อ่านครบ ${status.docsTotal} ฉบับแล้ว กำลังคิดว่าแฟ้มไหนรวมกันได้…` :
    stage === 'done' ? `เสร็จแผนก ${status.department} แล้ว! ${status.message}` : stage === 'error' ? `ติดปัญหา: ${status.message}` : 'กำลังเตรียมตัว…';
  const shelfState = (id: string): 'done' | 'current' | 'waiting' => id === status?.department && active ? 'current' : finished.has(id) ? 'done' : 'waiting';
  const checklist: [string, boolean][] = [
    ['หยิบแฟ้มจากตู้', Boolean(status && status.batch > 0)],
    [`อ่านครบ ${status ? `${status.docsDone}/${status.docsTotal}` : ''} ฉบับ`, stage === 'judging' || stage === 'done'],
    ['ตัดสินการรวมเอกสาร', stage === 'done'],
    ['สรุปผล ส่งเจ้าของอนุมัติ', stage === 'done'],
  ];
  const trays: [string, number, number][] = [['รอคิว', queue.length, 1104], ['กำลังอ่าน', active ? 1 : 0, 1186], ['อ่านเสร็จ', finishedList.length, 1267], ['อนุมัติแล้ว', approved, 1348]];
  const walkerStyle = { '--seat-x': `${STAND.x}px`, '--seat-y': `${STAND.y}px`, '--aisle-y': `${AISLE_Y}px`, '--shelf-x': `${shelfX}px`, '--out': outward, '--back': -outward } as CSSProperties;

  return <div className="agent-page">
    <div className="page-heading"><div><div className="eyebrow">AI AT WORK</div><h1>AI กำลังทำงาน</h1><p>เจ้าหน้าที่ AI เดินไปหยิบแฟ้มจากชั้นของแต่ละแผนกมาอ่านที่โต๊ะ · อัปเดตทุก 3 วินาทีจากสคริปต์ review_department.py</p></div>
      {problem && <button className="secondary" onClick={relinkConfig}><Settings2 size={16} /> เชื่อมโฟลเดอร์ iso-review</button>}</div>
    {problem && <div className="notice error agent-problem"><span><CircleAlert size={16} /> {problem}</span></div>}

    <section className="agent-stage">
      <div className={`px-bubble ${mode}`}>{bubble}</div>
      <svg viewBox={`0 0 ${IMG_W} ${IMG_H}`} role="img" aria-label={bubble} className="px-scene">
        <image href={officeBg} x={0} y={0} width={IMG_W} height={IMG_H} preserveAspectRatio="none" className="px-bg" />

        {/* shelves: name plates, progress state */}
        {Array.from({ length: SHELVES }, (_, i) => {
          const d = departments[i];
          const cx = plateX(i);
          const state = d ? shelfState(d.id) : 'other';
          return <g key={i} className={`px-shelf ${state}`}>
            {state === 'waiting' && <rect x={cx - 44} y={162} width={88} height={154} fill="#2a1408" opacity={0.22} />}
            {state === 'current' && <rect className="px-shelf-glow" x={cx - 46} y={118} width={92} height={200} fill="none" stroke="#ffd34d" strokeWidth={5} />}
            {state === 'current' && <rect x={cx - PLATE_W / 2} y={PLATE_Y - 11} width={PLATE_W} height={22} fill="#ffd34d" />}
            <text x={cx} y={PLATE_Y + 6} textAnchor="middle" className="px-plate">{d ? shortName(d.id) : 'ส่วนกลาง'}{d && <title>{d.id}</title>}</text>
            {state === 'done' && <g transform={`translate(${cx + 22} ${PLATE_Y - 26}) scale(4)`} shapeRendering="crispEdges"><Pixels rows={['.gggg.', 'gggwgg', 'ggwggg', 'wwgggg', '.gggg.']} palette={{ g: '#2f9a4a', w: '#ffffff' }} /></g>}
            {state === 'current' && <g className="px-arrow" transform={`translate(${cx - 16} 98) scale(4)`} shapeRendering="crispEdges"><Pixels rows={['yyyyyyyy', '.yyyyyy.', '..yyyy..', '...yy...']} palette={{ y: '#ffb02e' }} /></g>}
          </g>;
        })}

        {/* whiteboard checklist */}
        <text x={995} y={392} textAnchor="middle" className="px-board-title">ISO · {status?.department || '—'}</text>
        {checklist.map(([label, done], i) => <g key={label}>
          <rect x={892} y={402 + i * 17} width={12} height={12} fill="#fbf6ef" stroke="#3b4252" strokeWidth={2} />
          {done && <path d={`M894 ${409 + i * 17} l4 4 l8 -10`} fill="none" stroke="#2f9a4a" strokeWidth={3} />}
          <text x={912} y={413 + i * 17} className={`px-board-item${done ? ' done' : ''}`}>{label}</text>
        </g>)}

        {/* bulletin board: latest results */}
        {finishedList.slice(-3).reverse().map((f, i) => <g key={f.department} transform={`rotate(${[-3, 2, -1][i]} ${1196 + i * 66} 400)`}>
          <rect x={1168 + i * 66} y={372} width={58} height={56} fill={['#fff4b8', '#d8f0ff', '#ffe0ea'][i]} stroke="#00000022" />
          <circle cx={1197 + i * 66} cy={376} r={4} fill="#c8453d" />
          <text x={1197 + i * 66} y={398} textAnchor="middle" className="px-note">{shortName(f.department)}</text>
          <text x={1197 + i * 66} y={418} textAnchor="middle" className="px-note b">{f.before}→{f.after}</text>
        </g>)}

        {/* workflow trays */}
        {trays.map(([label, count, cx]) => <g key={label}>
          <rect x={cx - 38} y={524} width={76} height={22} fill="#5b3a24" /><rect x={cx - 36} y={526} width={72} height={18} fill="#f3e3c0" />
          <text x={cx} y={540} textAnchor="middle" className="px-tray">{label}</text>
          <rect x={cx - 20} y={652} width={40} height={30} rx={4} fill="#2a2e38" opacity={0.85} />
          <text x={cx} y={676} textAnchor="middle" className="px-count">{count}</text>
        </g>)}

        {/* the agent: seated frames sit on the reading-table chair; walking frames are anchored at the feet */}
        {mode === 'walk' && !tripping && <g className="px-reading"><Seated kind="read" /></g>}
        {mode === 'think' && <Seated kind="think" />}
        {mode === 'sad' && <Seated kind="think" />}
        {mode === 'sleep' && <Seated kind="sleep" />}
        {mode === 'happy' && <g className="px-hop"><Seated kind="cheer" /></g>}
        {mode === 'walk' && tripping && <g key={trip} className="px-walker walk" style={walkerStyle}>
          <ellipse cx={0} cy={0} rx={20} ry={5} fill="#00000038" />
          <g className="f-up"><Frame className="sub-a" src={backA} /><Frame className="sub-b" src={backB} /></g>
          <g className="f-out"><g className="px-flip-out"><Frame className="sub-a" src={sideA} /><Frame className="sub-b" src={sideB} /></g></g>
          <g className="f-reach"><g className="px-reach"><Frame src={backStand} /></g></g>
          <g className="f-back"><g className="px-flip-back"><g className="px-bob"><Frame src={sideCarry} /></g></g></g>
          <g className="f-down"><Frame className="sub-a" src={frontA} /><Frame className="sub-b" src={frontB} /></g>
        </g>}
        {mode === 'think' && <g className="px-think" transform={`translate(${SEAT_X + 26} 318) scale(4)`} shapeRendering="crispEdges"><Pixels rows={['..wwwwww..', '.wwwwwwww.', 'wwwkkwwwww', 'wwwwwkwwww', 'wwwwkwwwww', '.wwwwwwww.', '..wwkwww..', '..ww......', '.w........']} palette={{ w: '#fffdf6', k: '#3b2a20' }} /></g>}
        {mode === 'sad' && <g transform={`translate(${SEAT_X + 30} 336) scale(4)`} shapeRendering="crispEdges"><Pixels rows={['.rr.', '.rr.', '.rr.', '....', '.rr.']} palette={{ r: '#e2483d' }} /></g>}
      </svg>
    </section>

    <div className="agent-grid">
      <section className="px-card">
        <div className="px-card-head"><h2>งานตอนนี้</h2><BookOpen size={18} /></div>
        <p className="px-card-sub">{status ? `${stageLabel[status.stage] || status.stage} · ${status.engine} (${status.model})` : 'ยังไม่มีการรันสคริปต์'}</p>
        {status && <div className="agent-now">
          <div className="agent-now-dept"><strong>{status.department}</strong><small>กลุ่ม {status.group}</small></div>
          <div className="px-progress" aria-label={`ความคืบหน้า ${Math.round(progress * 100)}%`}><span style={{ width: `${progress * 100}%` }} /></div>
          <div className="agent-now-stats"><span>ชุดที่ <b>{Math.min(status.batch, status.batches)}/{status.batches}</b></span><span>อ่านแล้ว <b>{status.docsDone}/{status.docsTotal}</b> ฉบับ</span><span><Clock size={12} /> เริ่ม {clock(status.startedAt)} · อัปเดต {clock(status.updatedAt)}</span></div>
          {stale && <p className="agent-stale">ไม่มีความเคลื่อนไหวเกิน 10 นาที สคริปต์อาจหยุดหรือรอโควตา</p>}
          {status.currentDocs.length > 0 && <div className="agent-docs">{status.currentDocs.map(d => <span key={d.code} title={d.title}>{d.code}</span>)}</div>}
        </div>}
      </section>
      <section className="px-card">
        <div className="px-card-head"><h2>อ่านจบแล้ว</h2></div>
        <p className="px-card-sub">{finished.size ? `${finished.size} แผนก · รวม ${totals.before} → ${totals.after} ฉบับ (${totals.before ? Math.round(totals.after / totals.before * 100) : 0}%)` : 'ยังไม่มีแผนกที่เสร็จ'}</p>
        <div className="agent-list">{[...finishedList].reverse().map(f => <div key={f.department} className="agent-row done">
          <strong>{f.department}</strong><span>{f.before} → <b>{f.after}</b> ฉบับ</span><small>{f.errors ? `จุดผิด ${f.errors} · ` : ''}{f.minutes} นาที</small>
        </div>)}</div>
      </section>
      <section className="px-card">
        <div className="px-card-head"><h2>รอคิว</h2></div>
        <p className="px-card-sub">{queue.length} แผนกในกลุ่ม 1–3 ที่ยังไม่ได้อ่าน</p>
        <div className="agent-list">{queue.map(d => <div key={d.id} className="agent-row"><strong>{d.id}</strong><span>{d.found} ฉบับ</span><small>กลุ่ม {d.group}</small></div>)}</div>
      </section>
    </div>
  </div>;
}
