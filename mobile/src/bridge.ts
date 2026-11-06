// herdr-talk 모바일 브리지 HTTP 클라이언트. 서버 쪽은 ../../mobile-bridge/bridge.js
export type Config = { url: string; token: string };
export type Line = { text: string; mine: boolean };
export type Peer = { name: string; ip: string; port: number };
export type ChatState = { lines: Line[]; peers: Peer[]; me: { name: string; port: number } | null };

const TIMEOUT_MS = 5000;

async function call(config: Config, path: string, init?: RequestInit): Promise<any> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  try {
    const res = await fetch(`${config.url.replace(/\/+$/, '')}${path}`, {
      ...init,
      signal: controller.signal,
      headers: { Authorization: `Bearer ${config.token}`, 'Content-Type': 'application/json', ...init?.headers },
    });
    const body = await res.json();
    if (!res.ok || body.ok !== true) throw new Error(body.error ?? `HTTP ${res.status}`);
    return body;
  } finally {
    clearTimeout(timer);
  }
}

export async function fetchState(config: Config): Promise<ChatState> {
  const body = await call(config, '/state');
  return { lines: body.lines, peers: body.peers, me: body.me };
}

export async function sendText(config: Config, text: string): Promise<void> {
  await call(config, '/send', { method: 'POST', body: JSON.stringify({ text }) });
}
