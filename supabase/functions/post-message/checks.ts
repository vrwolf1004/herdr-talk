// Pure checks for the post-message function: a sender posts one room message (one ciphertext per recipient).
// No network, no environment, no secret: everything here can be tested alone. Only erasable TypeScript syntax, so the
// same file runs under Deno (the function) and under Node's type stripping (tests).

export const POST_TAG = "herdr-talk-msg-v1";
export const WINDOW_SECONDS = 300;
export const MAX_DELIVERIES = 8;
const NL = String.fromCharCode(10);

export type Verdict = "ok" | "format" | "time" | "signature" | "nonce";

export interface Delivery {
  recipient_pk: string;
  ciphertext: string;
}

export interface PostBody {
  room_code: string;
  sender_pk: string;
  message_id: string;
  ts: number;
  nonce: string;
  deliveries: Delivery[];
  sig: string;
}

const ROOM_PATTERN = /^[A-Z0-9]{8}$/;
const PK_PATTERN = /^[0-9a-f]{64}$/;
const ID_PATTERN = /^[0-9a-f]{32}$/;
const NONCE_PATTERN = /^[0-9a-f]{32}$/;
const SIG_PATTERN = /^[0-9a-f]{128}$/;
const CIPHERTEXT_PATTERN = /^[0-9a-f]{2,32768}$/;

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// Shape only. Recipients must be unique and in ascending order: the signed digest depends on that order.
export function checkFormat(body: unknown): body is PostBody {
  if (!isPlainObject(body)) return false;
  const { room_code, sender_pk, message_id, ts, nonce, deliveries, sig } = body;
  if (typeof room_code !== "string" || !ROOM_PATTERN.test(room_code)) return false;
  if (typeof sender_pk !== "string" || !PK_PATTERN.test(sender_pk)) return false;
  if (typeof message_id !== "string" || !ID_PATTERN.test(message_id)) return false;
  if (typeof ts !== "number" || !Number.isSafeInteger(ts)) return false;
  if (typeof nonce !== "string" || !NONCE_PATTERN.test(nonce)) return false;
  if (typeof sig !== "string" || !SIG_PATTERN.test(sig)) return false;
  if (!Array.isArray(deliveries) || deliveries.length < 1 || deliveries.length > MAX_DELIVERIES) return false;
  let previous = "";
  for (const delivery of deliveries) {
    if (!isPlainObject(delivery)) return false;
    const { recipient_pk, ciphertext } = delivery;
    if (typeof recipient_pk !== "string" || !PK_PATTERN.test(recipient_pk)) return false;
    if (typeof ciphertext !== "string" || !CIPHERTEXT_PATTERN.test(ciphertext)) return false;
    if (recipient_pk <= previous) return false;
    previous = recipient_pk;
  }
  return true;
}

export function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

// SHA-256 (hex) over the deliveries in the order they are sent: "recipient ciphertext" lines.
export async function deliveriesDigest(deliveries: Delivery[]): Promise<string> {
  const text = deliveries.map((delivery) => delivery.recipient_pk + " " + delivery.ciphertext).join(NL);
  return bytesToHex(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text))));
}

// The signed bytes: POST_TAG, "post", room code, sender key, message id, ts, nonce, deliveries digest; one per line.
export async function signedMessage(body: PostBody): Promise<Uint8Array> {
  const digest = await deliveriesDigest(body.deliveries);
  const lines = [POST_TAG, "post", body.room_code, body.sender_pk, body.message_id, String(body.ts), body.nonce, digest];
  return new TextEncoder().encode(lines.join(NL));
}

// |ts - now| <= 300 seconds.
export function withinWindow(ts: number, nowSeconds: number): boolean {
  return Math.abs(ts - nowSeconds) <= WINDOW_SECONDS;
}

// Ed25519 check with Web Crypto, by the sender's key. Any exception counts as a failed check.
export async function signatureOk(body: PostBody): Promise<boolean> {
  try {
    const key = await crypto.subtle.importKey("raw", hexToBytes(body.sender_pk), { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify("Ed25519", key, hexToBytes(body.sig), await signedMessage(body));
  } catch {
    return false;
  }
}

// The order the function applies: format, time window, signature. The nonce check comes after these (in index.ts).
export async function checkBeforeNonce(body: unknown, nowSeconds: number): Promise<Verdict> {
  if (!checkFormat(body)) return "format";
  if (!withinWindow(body.ts, nowSeconds)) return "time";
  if (!(await signatureOk(body))) return "signature";
  return "ok";
}

// HTTP status for each verdict. The success answer (200) is written by index.ts.
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

// The {error} text for a refusal: fixed and short.
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
