#!/usr/bin/env node
// herdr-talk 모바일 브리지. 폰(Expo Go)의 HTTP 요청을 로컬 herdr-talk 제어 채널로 중계한다.
// 외부 패키지 없음. 실행: node bridge.js  (herdr-talk serve 가 먼저 떠 있어야 함)
const http = require('node:http');
const net = require('node:net');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');

// 포트와 토큰: 환경변수가 먼저, 다음은 상태 폴더의 phone.json (방장 노드가 만든다), 없으면 이 브리지가 무작위로 만든다.
// phone.json 은 요청마다 다시 읽는다: 채팅창의 /phone new 로 토큰이 바뀌면 바로 반영되고 기존 폰은 새 토큰이 필요하다.
function phoneInfo() {
  try {
    const info = JSON.parse(fs.readFileSync(path.join(stateDir(), 'phone.json'), 'utf8'));
    return /^\d{6}$/.test(String(info.token)) ? info : null;
  } catch { return null; }
}
const PORT = Number(process.env.TALK_BRIDGE_PORT || phoneInfo()?.port || 47860);
const FALLBACK_TOKEN = String(crypto.randomInt(0, 1000000)).padStart(6, '0');
function currentToken() {
  return process.env.TALK_BRIDGE_TOKEN || phoneInfo()?.token || FALLBACK_TOKEN;
}
const TIMEOUT_MS = 5000;
// 6자리 토큰 무작위 대입 방지: IP 별 연속 실패 5회 → 10분 차단.
const MAX_FAILS = 5;
const LOCK_MS = 10 * 60 * 1000;
const failures = new Map(); // ip -> { count, until }
const MAX_BODY = 64 * 1024;

// herdr-talk 의 state_dir() 와 같은 순서.
function stateDir() {
  if (process.env.HERDR_PLUGIN_STATE_DIR) return process.env.HERDR_PLUGIN_STATE_DIR;
  const base = process.env.LOCALAPPDATA || path.join(process.env.HOME || os.homedir(), '.local', 'state');
  return path.join(base, 'herdr-talk');
}

function controlFiles() {
  let names = [];
  try { names = fs.readdirSync(stateDir()); } catch { return []; }
  return names.filter((n) => /^control-\d+\.json$/.test(n))
    .map((n) => { try { return JSON.parse(fs.readFileSync(path.join(stateDir(), n), 'utf8')); } catch { return null; } })
    .filter((c) => c && c.port && c.token);
}

// 제어 채널 한 연결에서: 토큰 인증 후 requests 를 순서대로 보내고 답을 모은다.
function talk(control, requests) {
  return new Promise((resolve, reject) => {
    const steps = [{ token: control.token }, ...requests];
    const answers = [];
    let buffer = '';
    let step = 0;
    let done = false;
    const socket = net.connect(control.port, '127.0.0.1');
    const finish = (error, value) => {
      if (done) return;
      done = true;
      socket.destroy();
      error ? reject(error) : resolve(value);
    };
    socket.setTimeout(TIMEOUT_MS, () => finish(new Error('control timeout')));
    socket.on('error', (error) => finish(error));
    socket.on('connect', () => socket.write(JSON.stringify(steps[0]) + '\n'));
    socket.on('data', (chunk) => {
      buffer += chunk.toString('utf8');
      let end;
      while ((end = buffer.indexOf('\n')) >= 0) {
        const answer = JSON.parse(buffer.slice(0, end));
        buffer = buffer.slice(end + 1);
        if (step === 0 && answer.ok !== true) return finish(new Error('token refused'));
        if (step > 0) answers.push(answer);
        step += 1;
        if (step === steps.length) return finish(null, answers);
        socket.write(JSON.stringify(steps[step]) + '\n');
      }
    });
    socket.on('close', () => finish(new Error('control closed')));
  });
}

// 살아 있는 herdr-talk 제어 채널을 찾아 requests 를 보낸다.
async function withChat(requests) {
  for (const control of controlFiles()) {
    try {
      return await talk(control, requests);
    } catch {
      // 죽은 pid 의 파일일 수 있다. 다음 파일로.
    }
  }
  throw Object.assign(new Error('herdr-talk 실행 중 아님 (herdr-talk serve 필요)'), { status: 503 });
}

function send(res, status, body) {
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' });
  res.end(JSON.stringify(body));
}

function authorized(req) {
  const given = Buffer.from(String(req.headers.authorization || ''));
  const expected = Buffer.from('Bearer ' + currentToken());
  return given.length === expected.length && crypto.timingSafeEqual(given, expected);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let size = 0;
    const chunks = [];
    req.on('data', (chunk) => {
      size += chunk.length;
      if (size > MAX_BODY) reject(Object.assign(new Error('body too large'), { status: 413 }));
      else chunks.push(chunk);
    });
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

const server = http.createServer(async (req, res) => {
  try {
    const ip = req.socket.remoteAddress;
    const record = failures.get(ip);
    if (record && record.until > Date.now()) return send(res, 429, { ok: false, error: 'too many attempts' });
    if (!authorized(req)) {
      const count = (record?.count ?? 0) + 1;
      failures.set(ip, count >= MAX_FAILS ? { count: 0, until: Date.now() + LOCK_MS } : { count, until: 0 });
      return send(res, 401, { ok: false, error: 'bad token' });
    }
    failures.delete(ip);

    if (req.method === 'GET' && req.url === '/state') {
      const [lines, peers] = await withChat([{ cmd: 'lines' }, { cmd: 'peers' }]);
      return send(res, 200, {
        ok: true,
        lines: lines.lines || [],
        peers: peers.peers || [],
        me: peers.me || null,
      });
    }

    if (req.method === 'POST' && req.url === '/send') {
      const { text } = JSON.parse((await readBody(req)) || '{}');
      const message = String(text || '').trim();
      if (!message) return send(res, 400, { ok: false, error: 'empty message' });
      const [answer] = await withChat([{ cmd: 'send', text: message }]);
      return send(res, answer.ok ? 200 : 502, answer);
    }

    return send(res, 404, { ok: false, error: 'not found' });
  } catch (error) {
    return send(res, error.status || 500, { ok: false, error: error.message });
  }
});

server.listen(PORT, '0.0.0.0', () => {
  const ips = Object.values(os.networkInterfaces()).flat()
    .filter((i) => i && i.family === 'IPv4' && !i.internal).map((i) => i.address);
  console.log(`herdr-talk bridge: port ${PORT}`);
  // phone.json 의 토큰은 채팅창 (Ctrl+P) 에서만 보여 주므로 여기에 찍지 않는다.
  if (process.env.TALK_BRIDGE_TOKEN || !phoneInfo()) console.log(`token: ${currentToken()}`);
  else console.log('token: 채팅창에서 Ctrl+P (또는 /phone) 로 확인');
  for (const ip of ips) console.log(`app address: http://${ip}:${PORT}`);
  console.log('같은 Wi-Fi 에서만 쓰세요. 토큰은 이 창(또는 채팅창)에만 보입니다.');
});
