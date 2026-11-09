// Pure checks for the put-peer function. No network, no environment, no secret: everything here can be tested alone.
// Only erasable TypeScript syntax, so the same file runs under Deno (the function) and under Node's type stripping (tests).

export const PUT_TAG = "herdr-talk-put-v3";
export const WINDOW_SECONDS = 300;

export type Verdict = "ok" | "format" | "time" | "signature" | "nonce";

export interface PutBody {
  pk: string;
  noise_pk: string;
  short_id: string;
  name: string;
  ts: number;
  nonce: string;
  sig: string;
}

const PK_PATTERN = /^[0-9a-f]{64}$/;
const NOISE_PK_PATTERN = /^[0-9a-f]{64}$/;
const SHORT_ID_PATTERN = /^[a-z0-9]{8}$/;
const NONCE_PATTERN = /^[0-9a-f]{32}$/;
const SIG_PATTERN = /^[0-9a-f]{128}$/;
// Unicode category Cc: C0 controls, DEL, C1 controls. Same rule as the table constraint and both editions.
const CONTROL_PATTERN = /[\u0000-\u001f\u007f-\u009f]/;

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// Shape and size only. Anything that fails here is a 400 and never reaches the signature check.
export function checkFormat(body: unknown): body is PutBody {
  if (!isPlainObject(body)) return false;
  const { pk, noise_pk, short_id, name, ts, nonce, sig } = body;
  if (typeof pk !== "string" || !PK_PATTERN.test(pk)) return false;
  if (typeof noise_pk !== "string" || !NOISE_PK_PATTERN.test(noise_pk)) return false;
  if (typeof short_id !== "string" || !SHORT_ID_PATTERN.test(short_id)) return false;
  if (typeof name !== "string" || CONTROL_PATTERN.test(name)) return false;
  // Length is counted in characters (code points), the same way the database counts char_length.
  const length = [...name].length;
  if (length < 1 || length > 32) return false;
  if (typeof ts !== "number" || !Number.isSafeInteger(ts)) return false;
  if (typeof nonce !== "string" || !NONCE_PATTERN.test(nonce)) return false;
  if (typeof sig !== "string" || !SIG_PATTERN.test(sig)) return false;
  return true;
}

// The signed bytes (v3): PUT_TAG, pk, noise_pk, short_id, name, ts, nonce, one per line, no newline at the end, UTF-8.
export function signedMessage(pk: string, noisePk: string, shortId: string, name: string, ts: number, nonce: string): Uint8Array {
  return new TextEncoder().encode([PUT_TAG, pk, noisePk, shortId, name, String(ts), nonce].join("\n"));
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
export async function signatureOk(body: PutBody): Promise<boolean> {
  try {
    const key = await crypto.subtle.importKey("raw", hexToBytes(body.pk), { name: "Ed25519" }, false, ["verify"]);
    const message = signedMessage(body.pk, body.noise_pk, body.short_id, body.name, body.ts, body.nonce);
    return await crypto.subtle.verify("Ed25519", key, hexToBytes(body.sig), message);
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

// HTTP status for each verdict. The body is never a reason text.
export function statusFor(verdict: Verdict): number {
  switch (verdict) {
    case "ok":
      return 204;
    case "format":
      return 400;
    case "time":
    case "signature":
    case "nonce":
      return 401;
  }
}
