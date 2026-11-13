// Network side of the phone relay: the REST calls (signed row write, row lookup) and the Realtime websocket.
// Only fetch and the platform WebSocket are used (Expo Go has both). Nothing here is printed with the key.

import { isKeyHex, putBody, signPut } from "./protocol.ts";
import { isPeerInput, isValidName, type RelaySettings } from "./settings.ts";

export const TIMEOUT_MS = 5000;
const JOIN_TIMEOUT_MS = 10000;
const HEARTBEAT_MS = 25000;
const RETRY_MS = 5000;
const EVENT = "envelope";

type Fetch = (url: string, init: { method: string; headers: Record<string, string>; body?: string }) => Promise<{ status: number; text: () => Promise<string> }>;

/** The websocket URL, from the https project URL and the publishable key. */
export function socketUrl(url: string, key: string): string {
  const base = url.trim().replace(/\/+$/, "").replace(/^https:\/\//, "wss://");
  return `${base}/realtime/v1/websocket?apikey=${encodeURIComponent(key)}&vsn=1.0.0`;
}

export function channelTopic(channel: string): string {
  return `realtime:${channel}`;
}

/** How many times a new short ID is tried when the relay says another key has it. */
export const PUT_TRIES = 5;

export type PutOutcome = "ok" | "taken" | "failed";

/** Signed row write (put-peer v3). "taken" (409): the short ID belongs to another key. Any other failure is "failed". */
export async function putRow(
  settings: RelaySettings,
  signingSeed: Uint8Array,
  noisePublicHex: string,
  shortId: string,
  now: number,
  nonce: string,
  fetcher: Fetch = fetch as unknown as Fetch,
): Promise<PutOutcome> {
  if (!isValidName(settings.name) || !isKeyHex(noisePublicHex) || !isPeerInput(shortId)) return "failed";
  const row = signPut(signingSeed, noisePublicHex, shortId, settings.name, now, nonce);
  try {
    const response = await withTimeout(
      fetcher(`${settings.url.replace(/\/+$/, "")}/functions/v1/put-peer`, {
        method: "POST",
        headers: { apikey: settings.key, Authorization: `Bearer ${settings.key}`, "Content-Type": "application/json" },
        body: putBody(row.pk, noisePublicHex, shortId, settings.name, now, nonce, row.sig),
      }),
    );
    if (response.status === 409) return "taken";
    return response.status >= 200 && response.status < 300 ? "ok" : "failed";
  } catch {
    return "failed";
  }
}

function withTimeout<T>(promise: Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("timeout")), TIMEOUT_MS);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

export type LinkEvents = {
  onStatus: (text: string) => void;
  onEnvelope: (line: string) => void;
};

/**
 * One channel on the websocket: join, heartbeat, receive and send envelopes. Reconnects after a failure, every 5 s,
 * until close() is called. The WebSocket class is injected so the logic can be tested without a device.
 */
export class RelayLink {
  private socket: WebSocket | null = null;
  private joined = false;
  private closed = false;
  private ref = 0;
  private heartbeat: ReturnType<typeof setInterval> | null = null;
  private joinTimer: ReturnType<typeof setTimeout> | null = null;
  private retry: ReturnType<typeof setTimeout> | null = null;

  private readonly url: string;
  private readonly topic: string;
  private readonly events: LinkEvents;
  private readonly makeSocket: (url: string) => WebSocket;

  constructor(url: string, topic: string, events: LinkEvents, makeSocket: (url: string) => WebSocket = (u) => new WebSocket(u)) {
    this.url = url;
    this.topic = topic;
    this.events = events;
    this.makeSocket = makeSocket;
  }

  open(): void {
    this.closed = false;
    this.events.onStatus("connecting");
    const socket = this.makeSocket(this.url);
    this.socket = socket;
    socket.onopen = () => {
      this.send({ topic: this.topic, event: "phx_join", payload: { config: { broadcast: { ack: false, self: false }, presence: { key: "" }, private: false } }, ref: this.nextRef() });
      this.joinTimer = setTimeout(() => this.fail("join not acknowledged"), JOIN_TIMEOUT_MS);
    };
    socket.onmessage = (message: { data: unknown }) => this.onFrame(String(message.data));
    socket.onerror = () => this.fail("connection error");
    socket.onclose = () => this.fail("connection closed");
  }

  /** Send one envelope line on the channel. False when the socket is not joined. */
  sendEnvelope(line: string): boolean {
    if (!this.socket || !this.joined) return false;
    let envelope: unknown;
    try {
      envelope = JSON.parse(line);
    } catch {
      return false;
    }
    this.send({ topic: this.topic, event: "broadcast", payload: { type: "broadcast", event: EVENT, payload: envelope }, ref: this.nextRef() });
    return true;
  }

  close(): void {
    this.closed = true;
    this.clearTimers();
    this.socket?.close();
    this.socket = null;
    this.joined = false;
  }

  private onFrame(text: string): void {
    let frame: { topic?: unknown; event?: unknown; payload?: unknown; ref?: unknown };
    try {
      frame = JSON.parse(text);
    } catch {
      return;
    }
    if (frame.event === "phx_reply" && frame.topic === this.topic) {
      const status = (frame.payload as { status?: unknown } | undefined)?.status;
      if (status === "ok") {
        this.joined = true;
        if (this.joinTimer) clearTimeout(this.joinTimer);
        this.joinTimer = null;
        this.heartbeat = setInterval(() => this.send({ topic: "phoenix", event: "heartbeat", payload: {}, ref: this.nextRef() }), HEARTBEAT_MS);
        this.events.onStatus("connected");
      } else {
        this.fail("join refused");
      }
      return;
    }
    if (frame.event === "broadcast" && frame.topic === this.topic) {
      const payload = frame.payload as { type?: unknown; event?: unknown; payload?: unknown } | undefined;
      if (payload?.type === "broadcast" && payload.event === EVENT && typeof payload.payload === "object" && payload.payload !== null) {
        this.events.onEnvelope(JSON.stringify(payload.payload));
      }
    }
  }

  private send(frame: { topic: string; event: string; payload: unknown; ref: string }): void {
    this.socket?.send(JSON.stringify(frame));
  }

  private nextRef(): string {
    this.ref += 1;
    return String(this.ref);
  }

  private fail(reason: string): void {
    this.clearTimers();
    this.joined = false;
    this.socket = null;
    if (this.closed) return;
    this.events.onStatus(`${reason}, retrying`);
    this.retry = setTimeout(() => this.open(), RETRY_MS);
  }

  private clearTimers(): void {
    if (this.heartbeat) clearInterval(this.heartbeat);
    if (this.joinTimer) clearTimeout(this.joinTimer);
    if (this.retry) clearTimeout(this.retry);
    this.heartbeat = null;
    this.joinTimer = null;
    this.retry = null;
  }
}
