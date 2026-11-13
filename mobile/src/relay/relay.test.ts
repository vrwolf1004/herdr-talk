// Node tests for the relay code (run: node --test src/relay/relay.test.ts). No network, no device.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { base64Decode, base64Encode, bytesToHex, hexToBytes, utf8Decode, utf8Encode } from "./bytes.ts";
import { publicKeyOf, readMessage1, writeMessage1 } from "./noise.ts";
import {
  NonceCache,
  channelName,
  encodePayload,
  fingerprint,
  normalizeCode,
  openEnvelope,
  parsePayload,
  putBody,
  sealEnvelope,
  signPut,
  verifyPut,
} from "./protocol.ts";
import { computeVectors, INPUTS } from "./vectors.ts";
import { codeMatches, emptySettings, isHttpsUrl, isRoomCode, keyLabel, missingFields, normalizeRoomCode } from "./settings.ts";
import { NAMES_BY_LANGUAGE, namesFor } from "./names.ts";
import { parseNames } from "../../scripts/gen-names.mjs";

const FILE = JSON.parse(readFileSync(new URL("../../test-vectors/relay.json", import.meta.url), "utf8"));

const NOW = 1700000000;
const KEY_A = new Uint8Array(32).fill(1);
const KEY_B = new Uint8Array(32).fill(2);
const NONCE = "00112233445566778899aabbccddeeff";

test("the computed vectors equal the shared vector file", () => {
  assert.deepEqual(computeVectors(), FILE.outputs);
  assert.deepEqual(FILE.inputs, INPUTS);
});

test("known vectors: channel name", () => {
  assert.equal(channelName(KEY_A, KEY_B), "c3492c89862240d31414b19d37da93d42c204f15687632dd436238f5701eb83e");
  assert.equal(channelName(KEY_B, KEY_A), channelName(KEY_A, KEY_B));
});

test("known vector: v3 row signature and public key", () => {
  const row = signPut(hexToBytes(INPUTS.signingSeed), INPUTS.rowNoisePk, INPUTS.rowShortId, INPUTS.rowName, INPUTS.rowTs, INPUTS.rowNonce);
  assert.equal(row.pk, FILE.outputs.rowPublic);
  assert.equal(row.sig, FILE.outputs.rowSignature);
});

test("Noise message 1 round trip: the responder gets the payload and the initiator's key", () => {
  const initiator = hexToBytes(INPUTS.initiatorStaticPrivate);
  const responder = hexToBytes(INPUTS.responderStaticPrivate);
  const payload = encodePayload("hi", NOW, NONCE);
  const message = writeMessage1(initiator, publicKeyOf(responder), utf8Encode(payload), hexToBytes(INPUTS.ephemeralPrivate));
  const read = readMessage1(responder, message);
  assert.ok(read);
  assert.equal(utf8Decode(read.payload), payload);
  assert.equal(bytesToHex(read.initiatorPublic), bytesToHex(publicKeyOf(initiator)));
});

test("a message for another responder does not open", () => {
  const initiator = hexToBytes(INPUTS.initiatorStaticPrivate);
  const message = writeMessage1(initiator, publicKeyOf(hexToBytes(INPUTS.responderStaticPrivate)), utf8Encode("x"), hexToBytes(INPUTS.ephemeralPrivate));
  assert.equal(readMessage1(new Uint8Array(32).fill(9), message), null);
});

test("envelope: seal, then open from the verified peer", () => {
  const line = sealEnvelope(KEY_A, KEY_B, utf8Encode("ct"));
  const opened = openEnvelope(line, KEY_B, KEY_A);
  assert.equal(typeof opened === "object" && utf8Decode(opened.ct), "ct");
});

test("envelope refusals follow the Rust order: format, version, channel, party", () => {
  const good = sealEnvelope(KEY_A, KEY_B, utf8Encode("ct"));
  assert.equal(openEnvelope("not json", KEY_B, KEY_A), "format");
  assert.equal(openEnvelope(good.replace('"v":1', '"v":2'), KEY_B, KEY_A), "version");
  assert.equal(openEnvelope(good.replace(channelName(KEY_A, KEY_B), "0".repeat(64)), KEY_B, KEY_A), "channel");
  // Sent by someone else on the same channel: a third key cannot be the sender.
  const third = new Uint8Array(32).fill(3);
  const fromThird = sealEnvelope(third, KEY_B, utf8Encode("x"));
  assert.equal(openEnvelope(fromThird, KEY_B, KEY_A), "channel", "a different channel name is refused first");
  assert.equal(openEnvelope(good, KEY_B, third), "channel");
  // Sender in the pair but not the verified peer.
  const fromMine = sealEnvelope(KEY_B, KEY_A, utf8Encode("x"));
  assert.equal(openEnvelope(fromMine, KEY_B, KEY_A), "party");
});

test("payload v1: fixed text form, and refusals", () => {
  const text = encodePayload('hi "there"', NOW, NONCE);
  assert.equal(text, `{"v":1,"ts":${NOW},"nonce":"${NONCE}","text":"hi \\"there\\""}`);
  assert.deepEqual(parsePayload(text), { ts: NOW, nonce: NONCE, text: 'hi "there"' });
  assert.equal(parsePayload(text.replace('"v":1', '"v":2')), null);
  assert.equal(parsePayload(encodePayload("x", NOW, "ABCD")), null);
  assert.equal(parsePayload("not json"), null);
});

test("replay: a nonce is accepted once; ts 299 s off is accepted, 301 s refused", () => {
  const cache = new NonceCache();
  assert.equal(cache.accept("aa", NOW, NOW), true);
  assert.equal(cache.accept("aa", NOW, NOW), false, "second time refused");
  assert.equal(cache.accept("bb", NOW - 299, NOW), true);
  assert.equal(cache.accept("cc", NOW - 301, NOW), false);
  assert.equal(cache.accept("dd", NOW + 301, NOW), false);
});

test("replay: entries leave with the window; a full cache of live entries refuses", () => {
  const cache = new NonceCache(2);
  assert.equal(cache.accept("n1", NOW, NOW), true);
  assert.equal(cache.accept("n2", NOW, NOW), true);
  assert.equal(cache.accept("n3", NOW, NOW), false, "full of live entries");
  assert.equal(cache.accept("n4", NOW + 301, NOW + 301), true, "old entries have left the window");
  assert.equal(cache.size, 1);
});

test("signed row: verifies, and any change breaks it", () => {
  const seed = hexToBytes(INPUTS.signingSeed);
  const SHORT = "ABCD2345";
  const row = signPut(seed, INPUTS.rowNoisePk, SHORT, "Alice", NOW, NONCE);
  assert.equal(verifyPut(row.pk, INPUTS.rowNoisePk, SHORT, "Alice", NOW, NONCE, row.sig), true);
  assert.equal(verifyPut(row.pk, INPUTS.rowNoisePk, "ABCD2346", "Alice", NOW, NONCE, row.sig), false, "short_id is signed");
  assert.equal(verifyPut(row.pk, INPUTS.rowNoisePk, SHORT, "Bob", NOW, NONCE, row.sig), false);
  assert.equal(verifyPut(row.pk, INPUTS.rowNoisePk, SHORT, "Alice", NOW + 1, NONCE, row.sig), false);
  const body = JSON.parse(putBody(row.pk, INPUTS.rowNoisePk, SHORT, "Alice", NOW, NONCE, row.sig));
  assert.deepEqual(Object.keys(body), ["pk", "noise_pk", "short_id", "name", "ts", "nonce", "sig"]);
});

test("fingerprint: symmetric, XXXX-XXXX groups, and typed codes are compared without dashes or case", () => {
  const a = fingerprint(KEY_A, KEY_B);
  assert.equal(a, fingerprint(KEY_B, KEY_A));
  assert.match(a, /^[0-9A-Z]{4}-[0-9A-Z]{4}$/);
  assert.equal(codeMatches(a.toUpperCase().replace(/-/g, " "), a), true);
  assert.equal(codeMatches(normalizeCode(a).slice(1) + "0", a), false);
});

test("base64 and UTF-8 helpers", () => {
  assert.equal(base64Encode(hexToBytes("00010203ff")), "AAECA/8=");
  assert.equal(bytesToHex(base64Decode("AAECA/8=") as Uint8Array), "00010203ff");
  assert.equal(base64Decode("Zg"), null);
  assert.equal(utf8Decode(utf8Encode("가a😀")), "가a😀");
});

test("settings: room ID is required, the URL must be https, the key is shown by length", () => {
  assert.deepEqual(missingFields(emptySettings()), ["url", "key", "roomCode"]);
  const full = { url: "https://x.supabase.co", key: "k", name: "Alice", roomCode: "ABCD2345" };
  assert.deepEqual(missingFields(full), []);
  assert.deepEqual(missingFields({ ...full, url: "http://x.example" }), ["url"]);
  assert.deepEqual(missingFields({ ...full, roomCode: "abcd2345" }), ["roomCode"], "lowercase is not valid until it is normalized on save");
  assert.equal(isHttpsUrl("https://x.supabase.co/"), true);
  assert.equal(keyLabel("dummy-test-key"), "(14 chars)");
});

// ---- room ID (8 characters) ----

test("room input: 8 uppercase letters or digits; typed lowercase is normalized first", () => {
  assert.equal(isRoomCode("ABCD2345"), true);
  assert.equal(isRoomCode("ZZZZZZZZ"), true);
  for (const bad of ["A".repeat(7), "A".repeat(9), "ab".repeat(32), "abcd2345", "ABCD-234", ""]) {
    assert.equal(isRoomCode(bad), false, bad);
  }
  assert.equal(normalizeRoomCode(" abcd-2345 "), "ABCD2345");
});

test("names.ts equals the talk-core name table (regenerate with scripts/gen-names.mjs)", () => {
  const rust = readFileSync(new URL("../../../rust/crates/talk-core/src/chat.rs", import.meta.url), "utf8");
  assert.deepEqual(NAMES_BY_LANGUAGE.map(([code, names]) => [code, [...names]]), parseNames(rust));
});

test("default names follow the language; an unlisted language gets every name", () => {
  assert.equal(namesFor("ko_KR")[0], "민준");
  assert.equal(namesFor("en-US")[0], "Liam");
  assert.ok(namesFor("xx").includes("민준") && namesFor("xx").includes("Liam"));
});
