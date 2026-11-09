// Pure checks for the delete-peer function. No network, no environment, no secret: everything here can be tested alone.
// Only erasable TypeScript syntax, so the same file runs under Deno (the function) and under Node's type stripping (tests).

export const DEL_TAG = "herdr-talk-del-v1";
export const WINDOW_SECONDS = 300;

export type Verdict = "ok" | "format" | "time" | "signature" | "nonce";

export interface DeleteBody {
  pk: string;
  short_id: string;
  ts: number;
  nonce: string;
  sig: string;
}

const PK_PATTERN = /^[0-9a-f]{64}$/;
const SHORT_ID_PATTERN = /^[a-z0-9]{8}$/;
const NONCE_PATTERN = /^[0-9a-f]{32}$/;
const SIG_PATTERN = /^[0-9a-f]{128}$/;

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// Shape only. Anything that fails here is a 400 and never reaches the signature check.
export function checkFormat(body: unknown): body is DeleteBody {
  if (!isPlainObject(body)) return false;
  const { pk, short_id, ts, nonce, sig } = body;
  if (typeof pk !== "string" || !PK_PATTERN.test(pk)) return false;
  if (typeof short_id !== "string" || !SHORT_ID_PATTERN.test(short_id)) return false;
  if (typeof ts !== "number" || !Number.isSafeInteger(ts)) return false;
  if (typeof nonce !== "string" || !NONCE_PATTERN.test(nonce)) return false;
  if (typeof sig !== "string" || !SIG_PATTERN.test(sig)) return false;
  return true;
}

// The signed bytes: DEL_TAG, pk, short_id, ts, nonce, one per line, no newline at the end, UTF-8.
export function signedMessage(pk: string, shortId: string, ts: number, nonce: string): Uint8Array {
  return new TextEncoder().encode([DEL_TAG, pk, shortId, String(ts), nonce].join("\n"));
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
export async function signatureOk(body: DeleteBody): Promise<boolean> {
  try {
    const key = await crypto.subtle.importKey("raw", hexToBytes(body.pk), { name: "Ed25519" }, false, ["verify"]);
    const message = signedMessage(body.pk, body.short_id, body.ts, body.nonce);
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
