// Pure checks for the create-room and join-room functions: a room request is signed with the requester's Ed25519 key.
// No network, no environment, no secret: everything here can be tested alone. Only erasable TypeScript syntax, so the
// same file runs under Deno (the function) and under Node's type stripping (tests). Keep the two copies (create-room and
// join-room) identical.

export const ROOM_TAG = "herdr-talk-room-v1";
export const WINDOW_SECONDS = 300;
export const ROOM_MEMBER_LIMIT = 10; // people in a room, the owner included
const NL = String.fromCharCode(10);

export type Action = "create" | "join";
export type Verdict = "ok" | "format" | "time" | "signature" | "nonce";

export interface RoomBody {
  peer_pk: string;
  room_code?: string;
  display_name: string;
  ts: number;
  nonce: string;
  sig: string;
  trust: string;
}

// The room's trust code: 8 digits, set by the owner at create, typed by each participant at join. Not in the signed bytes.
export const TRUST_PATTERN = /^[0-9]{8}$/;
export const TRUST_FAILS_LIMIT = 10; // wrong codes in a row before the room is locked
export const TRUST_LOCK_SECONDS = 3600;
const TRUST_ITERATIONS = 100000;

const PK_PATTERN = /^[0-9a-f]{64}$/;
const ROOM_PATTERN = /^[A-Z0-9]{8}$/;
const NONCE_PATTERN = /^[0-9a-f]{32}$/;
const SIG_PATTERN = /^[0-9a-f]{128}$/;

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// 1 to 32 characters, no control characters: the same rule as the rows and the Python and Rust sides.
export function validName(name: string): boolean {
  const chars = [...name];
  if (chars.length < 1 || chars.length > 32) return false;
  return !chars.some((ch) => {
    const code = ch.codePointAt(0) ?? 0;
    return code < 32 || (code >= 127 && code <= 159);
  });
}

// Shape only. Anything that fails here is a 400 and never reaches the signature check. A join needs a room code.
export function checkFormat(body: unknown, action: Action): body is RoomBody {
  if (!isPlainObject(body)) return false;
  const { peer_pk, room_code, display_name, ts, nonce, sig, trust } = body;
  if (typeof peer_pk !== "string" || !PK_PATTERN.test(peer_pk)) return false;
  if (typeof trust !== "string" || !TRUST_PATTERN.test(trust)) return false;
  if (action === "join" && (typeof room_code !== "string" || !ROOM_PATTERN.test(room_code))) return false;
  if (typeof display_name !== "string" || !validName(display_name)) return false;
  if (typeof ts !== "number" || !Number.isSafeInteger(ts)) return false;
  if (typeof nonce !== "string" || !NONCE_PATTERN.test(nonce)) return false;
  if (typeof sig !== "string" || !SIG_PATTERN.test(sig)) return false;
  return true;
}

// The signed bytes: ROOM_TAG, action, room code (an empty line for create), peer pk, display name, ts, nonce; one per
// line, no newline at the end, UTF-8. The Python (relay_rest.room_message) and Rust sides build the same string.
export function roomMessage(action: Action, roomCode: string, peerPk: string, displayName: string, ts: number, nonce: string): Uint8Array {
  const code = action === "create" ? "" : roomCode;
  return new TextEncoder().encode([ROOM_TAG, action, code, peerPk, displayName, String(ts), nonce].join(NL));
}

export function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

// |ts - now| <= 300 seconds.
export function withinWindow(ts: number, nowSeconds: number): boolean {
  return Math.abs(ts - nowSeconds) <= WINDOW_SECONDS;
}

// Ed25519 check with Web Crypto. Any exception (bad key, bad length) counts as a failed check.
export async function signatureOk(body: RoomBody, action: Action): Promise<boolean> {
  try {
    const key = await crypto.subtle.importKey("raw", hexToBytes(body.peer_pk), { name: "Ed25519" }, false, ["verify"]);
    const message = roomMessage(action, body.room_code ?? "", body.peer_pk, body.display_name, body.ts, body.nonce);
    return await crypto.subtle.verify("Ed25519", key, hexToBytes(body.sig), message);
  } catch {
    return false;
  }
}

// The stored form of a trust code: PBKDF2-SHA256 over the code with the room's salt (hex), 32 bytes, as hex.
export async function trustHash(trust: string, saltHex: string): Promise<string> {
  const key = await crypto.subtle.importKey("raw", new TextEncoder().encode(trust), "PBKDF2", false, ["deriveBits"]);
  const salt = hexToBytes(saltHex);
  const bits = await crypto.subtle.deriveBits({ name: "PBKDF2", hash: "SHA-256", salt, iterations: TRUST_ITERATIONS }, key, 256);
  return [...new Uint8Array(bits)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

// A new random salt for a room (16 bytes, hex).
export function newSalt(): string {
  return [...crypto.getRandomValues(new Uint8Array(16))].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

// Equal strings compared over every character, so the time does not show where they differ.
export function sameHex(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

// The order the function applies: format, time window, signature. The nonce check comes after these (in index.ts).
export async function checkBeforeNonce(body: unknown, action: Action, nowSeconds: number): Promise<Verdict> {
  if (!checkFormat(body, action)) return "format";
  if (!withinWindow(body.ts, nowSeconds)) return "time";
  if (!(await signatureOk(body, action))) return "signature";
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

// The {error} text for a refusal. Short and fixed: it never says which check failed in more detail than this.
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
