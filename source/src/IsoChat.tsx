import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { BookOpenText, Bot, ChevronRight, CircleAlert, FileText, LoaderCircle, MessageSquareText, RotateCw, Send, Square } from 'lucide-react';
import { api, type IsoChatAnswer, type IsoChatCitation, type IsoChatProgress, type IsoChatStatus } from './api';
import { RagInspector } from './RagInspector';
import './iso-chat.css';

const PdfReader = lazy(() => import('./PdfReader'));

type Message = { id: string; question: string; response?: IsoChatAnswer; error?: string; cancelled?: boolean };
type Stage = '' | 'retrieving' | 'answering' | 'cancelling';
const stages: Record<Exclude<Stage, ''>, string> = { retrieving:'กำลังค้นข้อความจากเอกสาร RAG…', answering:'GPT-6 Luna กำลังเรียบเรียงคำตอบพร้อมอ้างอิง…', cancelling:'กำลังยกเลิกคำถาม…' };
const examples = ['การควบคุมเอกสารและการแก้ไข Revision มีขั้นตอนอย่างไร', 'การคัดแยกขยะและของเสียต้องทำอย่างไร', 'การเก็บรักษาบันทึกคุณภาพต้องปฏิบัติอย่างไร'];

function createRequestId() {
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  // Web Crypto exposes random bytes on HTTP, where randomUUID is unavailable.
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  return [hex.slice(0, 8), hex.slice(8, 12), hex.slice(12, 16), hex.slice(16, 20), hex.slice(20)].join('-');
}

function AnswerText({ response, messageId }: { response: IsoChatAnswer; messageId: string }) {
  return <div className="iso-chat-answer-text">{response.answer.split(/(\[\d+\])/g).map((part, index) => {
    const match = /^\[(\d+)\]$/.exec(part);
    const reference = match && response.citations.find(source => source.id === Number(match[1]));
    return reference ? <a key={index} className="iso-chat-inline-ref" href={`#citation-${messageId}-${reference.id}`} aria-label={`ดูอ้างอิง ${reference.id} ${reference.name} หน้า ${reference.page}`}>{part}</a> : part;
  })}</div>;
}

export function IsoChatPage({ workspaceId, compact = false }: { workspaceId: string; compact?: boolean }) {
  const [status, setStatus] = useState<IsoChatStatus | null>(null);
  const [statusError, setStatusError] = useState('');
  const [question, setQuestion] = useState('');
  const [messages, setMessages] = useState<Message[]>([]);
  const [stage, setStage] = useState<Stage>('');
  const [inspect, setInspect] = useState<IsoChatCitation | null>(null);
  const [reading, setReading] = useState<IsoChatCitation | null>(null);
  const closeReader = useCallback(() => setReading(null), []);
  const [pdfError, setPdfError] = useState('');
  const request = useRef('');
  const mounted = useRef(true);
  const cancelled = useRef(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const end = useRef<HTMLDivElement>(null);
  const closeInspector = useCallback(() => setInspect(null), []);

  const refresh = useCallback(async () => {
    setStatusError('');
    try { const value=await api.isoChatStatus(); if(mounted.current)setStatus(value); }
    catch(error) { if(mounted.current)setStatusError(String(error)); }
  }, []);
  useEffect(() => { mounted.current=true; refresh(); return () => { mounted.current=false; if(request.current)void api.cancelIsoChat(request.current).catch(()=>{}); }; }, [refresh]);
  useEffect(() => {
    if(!('__TAURI_INTERNALS__' in window))return;
    let disposed=false;let off:(()=>void)|undefined;
    listen<IsoChatProgress>('iso-chat-progress', event => { if(event.payload.requestId===request.current && event.payload.workspaceId===workspaceId && !cancelled.current)setStage(event.payload.stage); })
      .then(unlisten => { if(disposed)unlisten();else off=unlisten; }).catch(()=>{});
    return()=>{disposed=true;off?.();};
  },[workspaceId]);
  useEffect(()=>{if(messages.length)end.current?.scrollIntoView({behavior:'smooth',block:'nearest'});},[messages.length]);

  async function send() {
    const text=question.trim();
    if(!text || text.length>2000 || request.current || !status?.ready || !status.indexedDocuments)return;
    const id = createRequestId();
    request.current = id;
    cancelled.current = false;
    setStage('retrieving');
    setQuestion('');
    setMessages(previous=>[...previous,{id,question:text}]);
    try {
      const response=await api.askIsoChat(text,workspaceId,id);
      if(mounted.current && request.current===id)setMessages(previous=>previous.map(message=>message.id===id?{...message,response}:message));
    } catch(error) {
      if(mounted.current && request.current===id)setMessages(previous=>previous.map(message=>message.id===id?{...message,error:String(error),cancelled:cancelled.current}:message));
    } finally {
      if (request.current === id) {
        request.current = '';
        if (mounted.current) {
          setStage('');
          if (!compact) requestAnimationFrame(() => { if (mounted.current) input.current?.focus(); });
        }
      }
    }
  }
  async function cancel() {
    if(!request.current)return;cancelled.current=true;setStage('cancelling');
    try { await api.cancelIsoChat(request.current); } catch(error) { if(mounted.current){cancelled.current=false;setPdfError(String(error));setStage('answering');} }
  }
  const busy=Boolean(stage);
  const conversation = <section key="conversation" className="panel iso-chat-conversation" aria-label="คำถามและคำตอบ">
      {messages.length===0 ? <div className="iso-chat-welcome">{!compact && <span className="iso-chat-welcome-icon"><MessageSquareText size={32}/></span>}<h2>{compact ? "ตัวอย่างคำถาม" : "อยากทราบเรื่องใดในเอกสาร ISO?"}</h2>{!compact && <p>ระบุหัวข้อ รหัสเอกสาร หรือชื่อแผนก เพื่อช่วยค้นหาเนื้อหาที่ตรงคำถาม</p>}<div className="iso-chat-examples">{examples.map(example=><button key={example} className="secondary" onClick={()=>{setQuestion(example);input.current?.focus();}}><FileText size={compact ? 19 : 16} aria-hidden="true"/><span>{example}</span>{compact && <ChevronRight size={16} aria-hidden="true"/>}</button>)}</div></div> : <div className="iso-chat-messages">{messages.map(message=><article key={message.id} className="iso-chat-turn"><div className="iso-chat-question"><small>คำถามของคุณ</small><p>{message.question}</p></div>{message.response&&<div className="iso-chat-answer"><div className="iso-chat-answer-label"><Bot size={18}/><strong>คำตอบจากเอกสาร</strong>{message.response.insufficientEvidence&&<span>ข้อมูลยังไม่เพียงพอ</span>}</div><AnswerText response={message.response} messageId={message.id}/>{message.response.citations.length>0&&<section className="iso-chat-sources" aria-label="เอกสารอ้างอิง"><h3><BookOpenText size={17}/> เอกสารอ้างอิง · {message.response.citations.length} ช่วงข้อความ</h3>{message.response.citations.map(source=><article key={source.id} id={`citation-${message.id}-${source.id}`} className="iso-chat-source"><div className="iso-chat-source-title"><span>[{source.id}]</span><div><strong>{source.name}</strong><small>หน้า {source.page} · {source.path}</small></div></div><details><summary>อ่านข้อความที่ใช้อ้างอิง</summary><p>{source.text}</p></details><div className="iso-chat-source-actions"><button className="text-button" onClick={()=>setInspect(source)}><BookOpenText size={14}/> ดูข้อมูล RAG หน้า {source.page}</button><button className="secondary" onClick={()=>setReading(source)}><BookOpenText size={14}/> อ่าน PDF</button></div></article>)}</section>}</div>}{message.error&&<div className={message.cancelled?'iso-chat-cancelled':'iso-chat-error'} role={message.cancelled?'status':'alert'}><CircleAlert size={17}/><div><p>{message.cancelled?'ยกเลิกคำถามแล้ว':message.error}</p><button className="text-button" disabled={busy} onClick={()=>{setQuestion(message.question);input.current?.focus();}}>นำคำถามกลับมาแก้ไข</button></div></div>}</article>)}<div ref={end}/></div>}
      {busy&&<div className="iso-chat-progress" role="status"><LoaderCircle size={18} className="spin"/>{stages[stage as Exclude<Stage,''>]}</div>}
    </section>;
  const composer = <form key="composer" className="panel iso-chat-composer" onSubmit={event=>{event.preventDefault();void send();}}><label htmlFor="iso-chat-question">พิมพ์คำถามเกี่ยวกับเอกสาร</label><textarea id="iso-chat-question" ref={input} rows={3} maxLength={2000} value={question} disabled={busy} onChange={event=>setQuestion(event.target.value)} onKeyDown={event=>{if(!compact&&event.key==='Enter'&&!event.shiftKey&&!event.nativeEvent.isComposing){event.preventDefault();void send();}}} placeholder="เช่น การคัดแยกของเสียต้องปฏิบัติอย่างไร และอ้างอิงเอกสารใด"/><div className="iso-chat-composer-actions"><small>{!compact && 'Enter เพื่อส่ง · Shift+Enter ขึ้นบรรทัดใหม่ · '}{question.length}/2,000</small>{busy?<button type="button" className="secondary" disabled={stage==='cancelling'} onClick={cancel}><Square size={15}/> ยกเลิก</button>:<button type="submit" className="primary" disabled={!question.trim()||!status?.ready||!status.indexedDocuments}><Send size={16}/> ส่งคำถาม</button>}</div>{!compact && <p>แต่ละคำถามค้นเอกสารใหม่ คำถามและช่วงข้อความอ้างอิงจะส่งไปยัง GPT-6 Luna ผ่านบัญชีที่ IT ตั้งไว้บนเซิร์ฟเวอร์ กรุณาตรวจ PDF ก่อนใช้คำตอบตัดสินใจ</p>}</form>;

  return <div className="iso-chat-page">
    <div className="page-heading"><div><div className="eyebrow">ISO KNOWLEDGE ASSISTANT</div><h1>ถาม–ตอบเอกสาร ISO</h1><p>{compact ? 'ค้นคำตอบจากเอกสาร พร้อมเลขหน้าอ้างอิง' : 'ถามจากเอกสารที่ทำดัชนีแล้ว พร้อมข้อความอ้างอิงและเลขหน้าที่ตรวจสอบได้'}</p></div>{!compact && <span className="iso-chat-model"><Bot size={16}/> GPT-6 Luna <small>ทดลองใช้</small></span>}</div>
    <div className="iso-chat-status"><span className={status?.ready?'iso-chat-dot ready':'iso-chat-dot'}/><span>{status ? compact ? status.ready ? 'พร้อมใช้งาน' : status.message : `${status.message} · ${status.indexedDocuments.toLocaleString()} เอกสารในพื้นที่นี้` : 'กำลังตรวจความพร้อม…'}</span>{compact && <span className="iso-chat-model">GPT-6 Luna</span>}<button className="text-button" disabled={busy} onClick={refresh} aria-label="ตรวจความพร้อมใหม่"><RotateCw size={14}/></button></div>
    {statusError&&<p className="notice notice-inline error" role="alert">{statusError}</p>}
    {status&&status.indexedDocuments===0&&<p className="notice notice-inline error">พื้นที่นี้ยังไม่มีเอกสาร RAG ที่พร้อมค้นหา กรุณาทำดัชนีที่เมนู Document Index ก่อน</p>}
    {compact && !messages.length ? [composer, conversation] : [conversation, composer]}
    {compact && <p className="iso-chat-mobile-note">คำถามและข้อความอ้างอิงจะส่งไปยัง GPT-6 Luna กรุณาตรวจ PDF ก่อนใช้คำตอบตัดสินใจ</p>}
    {pdfError&&<div className="notice notice-inline error" role="alert">{pdfError}</div>}
    {reading&&<Suspense fallback={<div className="pdf-reader-backdrop" role="status">กำลังเปิดตัวอ่านเอกสาร…</div>}><PdfReader key={reading.nodeId} nodeId={reading.nodeId} name={reading.name} initialPage={reading.page} onClose={closeReader}/></Suspense>}
    {inspect&&<RagInspector key={`${inspect.nodeId}:${inspect.page}`} nodeId={inspect.nodeId} initialPage={inspect.page} onClose={closeInspector}/>}
  </div>;
}
