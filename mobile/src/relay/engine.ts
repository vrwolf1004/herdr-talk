// The message rules of the phone's relay chat: what goes into an envelope, and what is shown from one.
// No network here: the caller passes the envelope line in and sends the line that comes out.

import { utf8Decode, utf8Encode } from "./bytes.ts";
import { readMessage1, writeMessage1 } from "./noise.ts";
import { NonceCache, encodePayload, openEnvelope, parsePayload, sealEnvelope } from "./protocol.ts";

export type Keys = {
  noisePrivate: Uint8Array; // X25519, stays on the phone
  noisePublic: Uint8Array;
};

export type Shown = { sender: string; text: string; mine: false };

export type ReceiveResult = { kind: "shown"; line: Shown } | { kind: "dropped" };

/**
 * Seal one text for the verified peer. `ephemeral` is a fresh random key in the app. `nonce` is 16 random bytes as hex.
 * Returns the envelope line to send on the channel.
 */
export function sealText(keys: Keys, peerNoisePublic: Uint8Array, text: string, ts: number, nonce: string, ephemeral: Uint8Array): string {
  const payload = encodePayload(text, ts, nonce);
  const ciphertext = writeMessage1(keys.noisePrivate, peerNoisePublic, utf8Encode(payload), ephemeral);
  return sealEnvelope(keys.noisePublic, peerNoisePublic, ciphertext);
}

/**
 * Receive one envelope line from the channel. Order (as in the Rust edition): envelope check, decryption and the
 * sender's static key, payload format, time window, nonce not used before, then the text is shown.
 * Every failure is silent: the result is "dropped".
 */
export function receiveLine(
  keys: Keys,
  peerNoisePublic: Uint8Array,
  peerName: string,
  line: string,
  cache: NonceCache,
  now: number,
): ReceiveResult {
  const envelope = openEnvelope(line, keys.noisePublic, peerNoisePublic);
  if (typeof envelope === "string") return { kind: "dropped" };
  const opened = readMessage1(keys.noisePrivate, envelope.ct);
  if (opened === null) return { kind: "dropped" };
  if (!sameBytes(opened.initiatorPublic, peerNoisePublic)) return { kind: "dropped" };
  const payload = parsePayload(utf8Decode(opened.payload));
  if (payload === null) return { kind: "dropped" };
  if (!cache.accept(payload.nonce, payload.ts, now)) return { kind: "dropped" };
  const text = payload.text.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, "").slice(0, 2000);
  if (text.trim().length === 0) return { kind: "dropped" };
  return { kind: "shown", line: { sender: peerName, text, mine: false } };
}

function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((byte, index) => byte === b[index]);
}
