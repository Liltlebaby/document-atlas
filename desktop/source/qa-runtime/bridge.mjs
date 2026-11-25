import { buildAgentCommand } from './node_modules/@roberttlange/headless/dist/agents.js';
import { extractFinalMessage, extractAgentError } from './node_modules/@roberttlange/headless/dist/output.js';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const runtime = dirname(fileURLToPath(import.meta.url));
export const model = 'gpt-6-luna';

export function childEnvironment(input = process.env) {
  const env = { ...input };
  for (const key of ['OPENAI_API_KEY', 'CODEX_API_KEY', 'OPENAI_BASE_URL', 'CODEX_MODEL', 'HEADLESS_BILLING', 'NODE_OPTIONS']) delete env[key];
  return env;
}

export function restrictedArgs(prompt) {
  // Headless 0.6.1 enables web search in read-only mode. This adapter removes it
  // and constrains the native harness to an ephemeral, document-only answer.
  const built = buildAgentCommand('codex', { prompt, model, allow:'read-only', fast:false, reasoningEffort:'low' }, {});
  const args = built.args.filter(arg => arg !== '--search');
  args.pop(); // Headless's final '-' stdin argument; append it after our options.
  args.push('--ignore-user-config', '--ignore-rules', '--ephemeral', '--output-schema', join(runtime,'answer.schema.json'));
  for (const config of [
    'forced_login_method="chatgpt"', 'project_doc_max_bytes=0',
    'web_search="disabled"', 'features.shell_tool=false', 'features.apps=false',
    'features.plugins=false', 'features.memories=false', 'features.request_permissions_tool=false',
    'skills.include_instructions=false', 'skills.bundled.enabled=false',
    'mcp_servers={}', `model_instructions_file=${JSON.stringify(join(runtime,'qa-instructions.md'))}`,
  ]) args.push('-c', config);
  args.push('-');
  return { args, stdinText: built.stdinText };
}

function codexEntry() {
  const candidates = [join(process.env.APPDATA || '', 'npm','node_modules','@openai','codex','bin','codex.js')];
  for (const dir of (process.env.PATH || '').split(';')) candidates.push(join(dir,'node_modules','@openai','codex','bin','codex.js'));
  const entry = candidates.find(path => existsSync(path));
  if (!entry) throw Error('ไม่พบ Codex CLI ในเครื่อง กรุณาติดตั้งและล็อกอินด้วย ChatGPT ก่อน');
  return entry;
}

export async function answer(prompt) {
  const entry = codexEntry();
  const env = childEnvironment();
  const auth = spawnSync(process.execPath, [entry, 'login', 'status'], {env,encoding:'utf8',windowsHide:true,timeout:15000,maxBuffer:65536});
  if (auth.status !== 0 || !`${auth.stdout}${auth.stderr}`.includes('ChatGPT')) throw Error('กรุณาล็อกอิน Codex CLI ด้วย ChatGPT ก่อนใช้งานถามตอบ ระบบนี้ไม่ใช้ API key');
  const command = restrictedArgs(prompt);
  return await new Promise((resolveReply, reject) => {
    const child = spawn(process.execPath, [entry, ...command.args], {env,cwd:runtime,windowsHide:true,stdio:['pipe','pipe','pipe']});
    child.stdout.setEncoding('utf8');child.stderr.setEncoding('utf8');
    let stdout='', stderr='', failure='';
    const stop = () => {
      if (child.pid && process.platform === 'win32') spawnSync('taskkill.exe',['/PID',String(child.pid),'/T','/F'],{windowsHide:true,stdio:'ignore',timeout:5000});
      else child.kill('SIGTERM');
    };
    const timer = setTimeout(() => { failure='ใช้เวลาตอบเกิน 120 วินาที กรุณาลองคำถามที่เจาะจงขึ้น'; stop(); },120000);
    child.stdout.on('data', data => { stdout += data.toString('utf8'); if(stdout.length>2000000){failure='คำตอบมีขนาดเกินกำหนด';stop();} });
    child.stderr.on('data', data => { if(stderr.length<32000) stderr+=data.toString('utf8'); });
    child.on('error', error => {clearTimeout(timer);reject(error);});
    child.stdin.on('error',()=>{});
    child.on('close', code => {
      clearTimeout(timer);
      if (failure) return reject(Error(failure));
      if (code !== 0) {
        const detail=extractAgentError('codex',stdout)||stderr;
        if (/usage.limit|rate.limit|quota|capacity/i.test(detail)) return reject(Error('โควต้า ChatGPT หรือความพร้อมของโมเดลไม่เพียงพอ กรุณาลองใหม่ภายหลัง ระบบไม่ได้สลับไปใช้ API'));
        if (/not.support|not.available|unsupported|model.*not found/i.test(detail)) return reject(Error('บัญชีนี้ยังเรียก GPT-6 Luna ไม่ได้ กรุณาตรวจสิทธิ์โมเดลใน Codex'));
        return reject(Error('เรียก GPT-6 Luna ไม่สำเร็จ กรุณาตรวจการล็อกอิน Codex และการเชื่อมต่ออินเทอร์เน็ต'));
      }
      const final=extractFinalMessage('codex',stdout);
      if(!final.trim())return reject(Error('โมเดลไม่ได้ส่งคำตอบกลับ กรุณาลองใหม่'));
      resolveReply(final);
    });
    child.stdin.end(command.stdinText);
  });
}

if(process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.stdin.setEncoding('utf8');let input='';for await(const part of process.stdin){input+=part;if(input.length>150000)throw Error('คำขอมีขนาดเกินกำหนด');}
    const request=JSON.parse(input);
    if(typeof request.prompt!=='string'||request.prompt.length>100000)throw Error('คำขอไม่ถูกต้อง');
    process.stdout.write(await answer(request.prompt));
  } catch(error) { process.stderr.write(error instanceof Error?error.message:'เรียกโมเดลไม่สำเร็จ');process.exitCode=1; }
}
