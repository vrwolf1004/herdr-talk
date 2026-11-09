// Run with: node --test supabase/functions/put-peer/checks.test.ts   (Node 22.6+ strips the types itself)
import { test } from "node:test";
import assert from "node:assert/strict";
import { checkBeforeNonce, checkFormat, signatureOk, signedMessage, statusFor, withinWindow } from "./checks.ts";

// v2 vector, same as the Python and Rust tests: seed bytes(range(32)), noise_pk ab x32, name "Alice", ts 1700000000,
// nonce 00112233445566778899aabbccddeeff. The signature was made with OpenSSL 3.5.4 (pkeyutl -sign -rawin).
const PK = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const NOISE_PK = "ab".repeat(32);
const NONCE = "00112233445566778899aabbccddeeff";
const SIG = "afc3815f65780c85d388896dea433086219bd41938750cf27af23b77e79a01020b458f9fc33207b7cfc472f3fdb4e8eb235cd70269255854569559c3a70d4c08";
const TS = 1700000000;
const vector = { pk: PK, noise_pk: NOISE_PK, name: "Alice", ts: TS, nonce: NONCE, sig: SIG };

test("format accepts the vector body", () => {
  assert.equal(checkFormat(vector), true);
});

test("format refuses bad shapes", () => {
  assert.equal(checkFormat({ ...vector, pk: PK.toUpperCase() }), false);
  assert.equal(checkFormat({ ...vector, noise_pk: "AB".repeat(32) }), false);
  assert.equal(checkFormat({ ...vector, noise_pk: undefined }), false);
  assert.equal(checkFormat({ ...vector, name: "" }), false);
  assert.equal(checkFormat({ ...vector, name: "a\u0007b" }), false);
  assert.equal(checkFormat({ ...vector, name: "x".repeat(33) }), false);
  assert.equal(checkFormat({ ...vector, nonce: "ABCD" }), false);
  assert.equal(checkFormat({ ...vector, sig: SIG.slice(0, 127) }), false);
  assert.equal(checkFormat({ ...vector, ts: 1.5 }), false);
  assert.equal(checkFormat({ ...vector, ts: "1700000000" }), false);
  assert.equal(checkFormat(null), false);
  assert.equal(checkFormat([vector]), false);
});

test("name length counts characters, not bytes", () => {
  assert.equal(checkFormat({ ...vector, name: "가".repeat(32) }), true);
  assert.equal(checkFormat({ ...vector, name: "가".repeat(33) }), false);
});

test("time window is 300 seconds either side", () => {
  assert.equal(withinWindow(TS, TS + 300), true);
  assert.equal(withinWindow(TS, TS - 300), true);
  assert.equal(withinWindow(TS, TS + 301), false);
  assert.equal(withinWindow(TS, TS - 301), false);
});

test("v2 signed message and signature match the external vector", async () => {
  const message = new TextDecoder().decode(signedMessage(PK, NOISE_PK, "Alice", TS, NONCE));
  assert.equal(message, ["herdr-talk-put-v2", PK, NOISE_PK, "Alice", "1700000000", NONCE].join("\n"));
  assert.equal(await signatureOk(vector), true);
  assert.equal(await signatureOk({ ...vector, noise_pk: "cd".repeat(32) }), false);
});

test("the full check order: ok, then time, then signature", async () => {
  assert.equal(await checkBeforeNonce(vector, TS), "ok");
  assert.equal(await checkBeforeNonce(vector, TS + 301), "time");
  assert.equal(await checkBeforeNonce({ ...vector, name: "Bob" }, TS), "signature");
  assert.equal(await checkBeforeNonce({ ...vector, sig: "00".repeat(64) }, TS), "signature");
  assert.equal(await checkBeforeNonce({ ...vector, pk: "02".repeat(32) }, TS), "signature");
  assert.equal(await checkBeforeNonce({ nope: true }, TS), "format");
});

test("status codes: 204 ok, 400 format, 401 for time, signature and nonce", () => {
  assert.equal(statusFor("ok"), 204);
  assert.equal(statusFor("format"), 400);
  assert.equal(statusFor("time"), 401);
  assert.equal(statusFor("signature"), 401);
  assert.equal(statusFor("nonce"), 401);
});
