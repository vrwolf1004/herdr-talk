// The relay protocol, same rules as the Rust talk-core (relay.rs, relay_rest.rs, relay_chat.rs, relay_replay.rs):
// channel name, envelope, payload (v1), signed row write (put-peer v2), key fingerprint, nonce cache.

import { sha256, sha512 } from "@noble/hashes/sha2.js";
import * as ed25519 from "@noble/ed25519";
import { concatBytes, base64Decode, base64Encode, bytesToHex, hexToBytes, utf8Encode } from "./bytes.ts";

// @noble/ed25519 (v3) signs synchronously once it has a SHA-512.
ed25519.hashes.sha512 = sha512;

export const CHANNEL_PREFIX = "herdr-talk-relay-v1";
export const PUT_TAG = "herdr-talk-put-v3";
export const WINDOW_SECONDS = 300;
export const NONCE_CAPACITY = 10000;

const HEX64 = /^[0-9a-f]{64}$/;
const HEX32 = /^[0-9a-f]{32}$/;

export function isKeyHex(text: string): boolean {
  return HEX64.test(text);
}

/** hex(SHA-256("herdr-talk-relay-v1" || min(a,b) || max(a,b))). Order of the two keys does not matter. */
export function channelName(a: Uint8Array, b: Uint8Array): string {
  const [low, high] = compareBytes(a, b) <= 0 ? [a, b] : [b, a];
  return bytesToHex(sha256(concatBytes(utf8Encode(CHANNEL_PREFIX), low, high)));
}

function compareBytes(a: Uint8Array, b: Uint8Array): number {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

/** Envelope as one JSON line, keys in the order v, channel, from, to, ct. */
export function sealEnvelope(fromNoise: Uint8Array, toNoise: Uint8Array, ciphertext: Uint8Array): string {
  return JSON.stringify({
    v: 1,
    channel: channelName(fromNoise, toNoise),
    from: bytesToHex(fromNoise),
    to: bytesToHex(toNoise),
    ct: base64Encode(ciphertext),
  });
}

export type Reject = "format" | "version" | "party" | "channel";

export type Envelope = { from: Uint8Array; to: Uint8Array; ct: Uint8Array };

/** Check one line against the two keys this side expects (the same order of checks as relay.rs). */
export function openEnvelope(line: string, mine: Uint8Array, theirs: Uint8Array): Envelope | Reject {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return "format";
  }
  if (typeof value !== "object" || value === null) return "format";
  const fields = value as Record<string, unknown>;
  if (typeof fields.v !== "number") return "format";
  if (fields.v !== 1) return "version";
  const from = keyField(fields.from);
  const to = keyField(fields.to);
  if (from === null || to === null) return "format";
  if (typeof fields.channel !== "string" || typeof fields.ct !== "string") return "format";
  const ct = base64Decode(fields.ct);
  if (ct === null) return "format";
  if (fields.channel !== channelName(mine, theirs)) return "channel";
  const inPair = (key: Uint8Array) => bytesEqual(key, mine) || bytesEqual(key, theirs);
  if (!inPair(from) || !inPair(to)) return "party";
  // Accept only a message from the verified peer.
  if (!bytesEqual(from, theirs)) return "party";
  return { from, to, ct };
}

function keyField(value: unknown): Uint8Array | null {
  if (typeof value !== "string" || !isKeyHex(value)) return null;
  return hexToBytes(value);
}

export function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((byte, index) => byte === b[index]);
}

/** Plaintext payload (format v1): one JSON line with the keys v, ts, nonce, text in this order. */
export function encodePayload(text: string, ts: number, nonce: string): string {
  return `{"v":1,"ts":${ts},"nonce":"${nonce}","text":${JSON.stringify(text)}}`;
}

export type Payload = { ts: number; nonce: string; text: string };

export function parsePayload(payload: string): Payload | null {
  let value: unknown;
  try {
    value = JSON.parse(payload);
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null) return null;
  const fields = value as Record<string, unknown>;
  if (fields.v !== 1) return null;
  if (typeof fields.ts !== "number" || !Number.isSafeInteger(fields.ts)) return null;
  if (typeof fields.nonce !== "string" || !HEX32.test(fields.nonce)) return null;
  if (typeof fields.text !== "string") return null;
  return { ts: fields.ts, nonce: fields.nonce, text: fields.text };
}

/** The signed string (v3): PUT_TAG, pk, noise_pk, short_id, name, ts, nonce, one per line, no newline at the end. */
export function putMessage(pk: string, noisePk: string, shortId: string, name: string, ts: number, nonce: string): Uint8Array {
  return utf8Encode([PUT_TAG, pk, noisePk, shortId, name, String(ts), nonce].join("\n"));
}

/** The Ed25519 public key (32 bytes) of a signing seed. */
export function signingPublicKey(seed: Uint8Array): Uint8Array {
  return ed25519.getPublicKey(seed);
}

export function signPut(seed: Uint8Array, noisePk: string, shortId: string, name: string, ts: number, nonce: string): { pk: string; sig: string } {
  const pk = bytesToHex(signingPublicKey(seed));
  const sig = bytesToHex(ed25519.sign(putMessage(pk, noisePk, shortId, name, ts, nonce), seed));
  return { pk, sig };
}

export function verifyPut(pk: string, noisePk: string, shortId: string, name: string, ts: number, nonce: string, sig: string): boolean {
  try {
    return ed25519.verify(hexToBytes(sig), putMessage(pk, noisePk, shortId, name, ts, nonce), hexToBytes(pk));
  } catch {
    return false;
  }
}

/** The request body of put-peer, in the fixed order pk, noise_pk, short_id, name, ts, nonce, sig. */
export function putBody(pk: string, noisePk: string, shortId: string, name: string, ts: number, nonce: string, sig: string): string {
  return JSON.stringify({ pk, noise_pk: noisePk, short_id: shortId, name, ts, nonce, sig });
}

/** Same as the Rust store::fingerprint: the first 128 bits of SHA-256 over the two keys, sorted, in groups of four. */
export function fingerprint(own: Uint8Array, peer: Uint8Array): string {
  const [first, second] = compareBytes(own, peer) <= 0 ? [own, peer] : [peer, own];
  const hex = bytesToHex(sha256(concatBytes(first, second))).slice(0, 32);
  const groups: string[] = [];
  for (let i = 0; i < 32; i += 4) {
    groups.push(hex.slice(i, i + 4));
  }
  return groups.join("-");
}

/** What the user types for a code: no dashes, spaces or capitals. */
export function normalizeCode(text: string): string {
  return text.replace(/[\s-]/g, "").toLowerCase();
}

/** Replay protection: a nonce is accepted once, only while its ts is within the window. Same rules as relay_replay.rs. */
export class NonceCache {
  private readonly entries: { ts: number; nonce: string }[] = [];
  private readonly seen = new Set<string>();

  private readonly capacity: number;

  constructor(capacity: number = NONCE_CAPACITY) {
    this.capacity = capacity;
  }

  accept(nonce: string, ts: number, now: number): boolean {
    if (Math.abs(now - ts) > WINDOW_SECONDS) return false;
    this.prune(now);
    if (this.seen.has(nonce) || this.entries.length >= this.capacity) return false;
    this.seen.add(nonce);
    this.entries.push({ ts, nonce });
    return true;
  }

  private prune(now: number): void {
    while (this.entries.length > 0 && Math.abs(now - this.entries[0].ts) > WINDOW_SECONDS) {
      const removed = this.entries.shift();
      if (removed) this.seen.delete(removed.nonce);
    }
  }

  get size(): number {
    return this.entries.length;
  }
}
