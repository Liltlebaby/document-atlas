import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { request } from 'node:http';
import { createServer } from 'node:https';
import { BlockList, isIPv4 } from 'node:net';
import { isAbsolute, join } from 'node:path';
import { createInterface } from 'node:readline';

const passphrase = process.env.ATLAS_TLS_PASSPHRASE;
delete process.env.ATLAS_TLS_PASSPHRASE;

const hopByHop = [
  'connection', 'keep-alive', 'proxy-authenticate', 'proxy-authorization',
  'proxy-connection', 'te', 'trailer', 'transfer-encoding', 'upgrade',
];

function endToEnd(headers) {
  const forwarded = { ...headers };
  const nominated = (headers.connection ?? '').split(',').map(value => value.trim().toLowerCase());
  for (const name of [...hopByHop, ...nominated]) delete forwarded[name];
  return forwarded;
}

function reply(response, code, message) {
  response.writeHead(code, { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'no-store' });
  response.end(message);
}

function forward(req, res, backendPort, allowedHosts) {
  if (!allowedHosts.has(req.headers.host?.toLowerCase()) || !req.url?.startsWith('/') || req.url.startsWith('//')) {
    reply(res, 400, 'Open the configured Document Atlas HTTPS URL.');
    return;
  }
  const headers = endToEnd(req.headers);
  headers.host = `127.0.0.1:${backendPort}`;
  const upstream = request({
    hostname: '127.0.0.1', port: backendPort, method: req.method, path: req.url, headers,
  }, incoming => {
    res.writeHead(incoming.statusCode, endToEnd(incoming.headers));
    incoming.on('error', () => res.destroy());
    incoming.pipe(res);
  });
  upstream.setTimeout(180_000, () => upstream.destroy(new Error('Atlas response timed out')));
  upstream.on('error', () => {
    if (!res.headersSent) reply(res, 502, 'Document Atlas backend unavailable.');
    else res.destroy();
  });
  res.on('close', () => {
    if (!res.writableFinished) upstream.destroy();
  });
  req.on('error', () => upstream.destroy());
  req.pipe(upstream);
}

function start() {
  const { Lan: lan } = JSON.parse(readFileSync(process.argv[2], 'utf8').replace(/^\uFEFF/, ''));
  if (!isIPv4(lan.Address) || lan.Address === '0.0.0.0') {
    throw new Error('Set Lan.Address to an IPv4 address assigned to this server.');
  }
  const hostname = lan.Hostname?.toLowerCase();
  const dnsLabel = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/;
  if (!hostname || hostname.length > 253 || !hostname.split('.').every(label => dnsLabel.test(label))) {
    throw new Error('Set Lan.Hostname to the DNS name on the TLS certificate.');
  }
  const backendPort = Number(process.env.ATLAS_PORT);
  if (!Number.isInteger(lan.Port) || lan.Port < 1 || lan.Port > 65535 || lan.Port === backendPort) {
    throw new Error('Set Lan.Port to a valid HTTPS port different from Port.');
  }
  if (typeof lan.PfxPath !== 'string' || !isAbsolute(lan.PfxPath) || passphrase === undefined) {
    throw new Error('Set Lan.PfxPath to an absolute PFX path and provide its protected passphrase.');
  }
  const allowedHosts = new Set([`${hostname}:${lan.Port}`]);
  if (lan.Port === 443) allowedHosts.add(hostname);
  const url = lan.Port === 443 ? `https://${hostname}/` : `https://${hostname}:${lan.Port}/`;
  let allowedClients;
  if (lan.AllowedSubnet !== undefined) {
    const [address, prefix, extra] = String(lan.AllowedSubnet).split('/');
    if (!isIPv4(address) || !/^\d+$/.test(prefix ?? '') || Number(prefix) > 32 || extra !== undefined) {
      throw new Error('Set Lan.AllowedSubnet to an IPv4 CIDR.');
    }
    allowedClients = new BlockList();
    allowedClients.addSubnet(address, Number(prefix), 'ipv4');
  }
  const gateway = createServer({
    pfx: readFileSync(lan.PfxPath),
    passphrase,
    minVersion: 'TLSv1.2',
    headersTimeout: 15_000,
    requestTimeout: 30_000,
  }, (req, res) => {
    if (allowedClients && !allowedClients.check(req.socket.remoteAddress, 'ipv4')) {
      reply(res, 403, 'This Document Atlas test is limited to the local network.');
      return;
    }
    forward(req, res, backendPort, allowedHosts);
  });

  const backend = spawn(join(process.env.ATLAS_ROOT, 'document-atlas-web.exe'), [], {
    stdio: ['ignore', 'pipe', 'inherit'],
    windowsHide: true,
  });
  let stopping = false;
  function stop(exitCode) {
    if (stopping) return;
    stopping = true;
    clearTimeout(startupTimeout);
    clearTimeout(sessionTimeout);
    gateway.close();
    gateway.closeAllConnections();
    if (backend.exitCode === null && backend.signalCode === null) backend.kill();
    process.exitCode = exitCode;
  }

  const startupTimeout = setTimeout(() => {
    console.error('Document Atlas backend did not start in 30 seconds.');
    stop(1);
  }, 30_000);
  const sessionTimeout = Number.isInteger(lan.SessionMinutes) && lan.SessionMinutes > 0 && lan.SessionMinutes <= 1440
    ? setTimeout(() => stop(0), lan.SessionMinutes * 60_000)
    : undefined;
  const stdout = createInterface({ input: backend.stdout });
  stdout.on('line', line => {
    console.log(line);
    if (!stopping && line === `Document Atlas Web is listening at http://127.0.0.1:${backendPort}`) {
      clearTimeout(startupTimeout);
      gateway.listen(lan.Port, lan.Address, () => console.log(`Document Atlas LAN is ready at ${url}`));
    }
  });
  gateway.on('error', error => {
    console.error(`Unable to open HTTPS on ${lan.Address}:${lan.Port} (${error.code}).`);
    stop(1);
  });
  backend.on('error', error => {
    console.error(`Unable to start Document Atlas backend (${error.code}).`);
    stop(1);
  });
  backend.on('exit', code => stop(code ?? 1));
  process.on('SIGINT', () => stop(0));
  process.on('SIGTERM', () => stop(0));
}

try {
  start();
} catch (error) {
  console.error(`Document Atlas LAN could not start. ${error.message}`);
  process.exitCode = 1;
}
