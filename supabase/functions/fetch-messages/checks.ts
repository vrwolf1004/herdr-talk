// Pure checks for the fetch-messages function: a recipient asks for its own room messages, signed with its Ed25519 key.
// No network, no environment, no secret. Only erasable TypeScript syntax (Deno and Node's type stripping).

export const FETCH_TAG = "herdr-talk-fetch-v1";
export const WINDOW_SECONDS = 300;
const NL = String.fromCharCode(10);

export type Verdict = "ok" | "format" | "time" | "signature" | "nonce";

export interface FetchBody {
  room_code: string;
  recipient_pk: string;
  since?: string;
  ts: number;
  nonce: string;
  sig: string;
}

const ROOM_PATTERN = /^[A-Z0-9]{8}$/;
const PK_PATTERN = /^[0-9a-f]{64}$/;
const NONCE_PATTERN = /^[0-9a-f]{32}$/;
const SIG_PATTERN = /^[0-9a-f]{128}$/;

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// Shape only. `since` is optional: an ISO time, only a filter (the signature does not need it).
export function checkFormat(body: unknown): body is FetchBody {
  if (!isPlainObject(body)) return false;
  const { room_code, recipient_pk, since, ts, nonce, sig } = body;
  if (typeof room_code !== "string" || !ROOM_PATTERN.test(room_code)) return false;
  if (typeof recipient_pk !== "string" || !PK_PATTERN.test(recipient_pk)) return false;
  if (since !== undefined && (typeof since !== "string" || since.length > 40)) return false;
  if (typeof ts !== "number" || !Number.isSafeInteger(ts)) return false;
  if (typeof nonce !== "string" || !NONCE_PATTERN.test(nonce)) return false;
  if (typeof sig !== "string" || !SIG_PATTERN.test(sig)) return false;
  return true;
}

export function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

// The signed bytes: FETCH_TAG, "fetch", room code, recipient key, ts, nonce; one per line.
export function signedMessage(body: FetchBody): Uint8Array {
  return new TextEncoder().encode([FETCH_TAG, "fetch", body.room_code, body.recipient_pk, String(body.ts), body.nonce].join(NL));
}

export function withinWindow(ts: number, nowSeconds: number): boolean {
  return Math.abs(ts - nowSeconds) <= WINDOW_SECONDS;
}

export async function signatureOk(body: FetchBody): Promise<boolean> {
  try {
    const key = await crypto.subtle.importKey("raw", hexToBytes(body.recipient_pk), { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify("Ed25519", key, hexToBytes(body.sig), signedMessage(body));
  } catch {
    return false;
  }
}

export async function checkBeforeNonce(body: unknown, nowSeconds: number): Promise<Verdict> {
  if (!checkFormat(body)) return "format";
  if (!withinWindow(body.ts, nowSeconds)) return "time";
  if (!(await signatureOk(body))) return "signature";
  return "ok";
}

export function statusFor(verdict: Verdict): number {
  switch (verdict) {
    case "ok":
      return 200;
    case "format":
      return 400;
    case "time":
    case "signature":
    case "nonce":
      return 401;
  }
}

export function messageFor(verdict: Verdict): string {
  switch (verdict) {
    case "ok":
      return "";
    case "format":
      return "invalid request";
    case "time":
      return "invalid time";
    case "signature":
      return "invalid signature";
    case "nonce":
      return "nonce already used";
  }
}
