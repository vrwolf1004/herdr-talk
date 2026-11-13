// Run with: node --test supabase/functions/put-peer/checks.test.ts   (Node 22.6+ strips the types itself)
import { test } from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, sign } from "node:crypto";
import { checkBeforeNonce, checkFormat, signatureOk, signedMessage, statusFor, withinWindow, type PutBody } from "./checks.ts";

const NOISE_PK = "ab".repeat(32);
const SHORT_ID = "ABCD2345";
const NONCE = "00112233445566778899aabbccddeeff";
const TS = 1700000000;

// A fresh Ed25519 key for each run (no fixed secret in the repo). The signing side is Node's crypto, the checking side is Web Crypto.
function signer() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const x = publicKey.export({ format: "jwk" }).x as string;
  return { pk: Buffer.from(x, "base64url").toString("hex"), privateKey };
}

function bodyFor(key: ReturnType<typeof signer>, overrides: Partial<PutBody> = {}): PutBody {
  const base = { pk: key.pk, noise_pk: NOISE_PK, short_id: SHORT_ID, name: "Alice", ts: TS, nonce: NONCE };
  const merged = { ...base, ...overrides };
  const message = signedMessage(merged.pk, merged.noise_pk, merged.short_id, merged.name, merged.ts, merged.nonce);
  return { ...merged, sig: sign(null, Buffer.from(message), key.privateKey).toString("hex") };
}

test("v3 signed message is the fixed order, one per line, no newline at the end", () => {
  const message = new TextDecoder().decode(signedMessage("pk", NOISE_PK, SHORT_ID, "Alice", TS, NONCE));
  assert.equal(message, ["herdr-talk-put-v3", "pk", NOISE_PK, SHORT_ID, "Alice", "1700000000", NONCE].join("\n"));
});

test("a signed v3 body passes the format check and the full check order", async () => {
  const key = signer();
  const body = bodyFor(key);
  assert.equal(checkFormat(body), true);
  assert.equal(await signatureOk(body), true);
  assert.equal(await checkBeforeNonce(body, TS), "ok");
});

test("format accepts a short_id of 8 lowercase letters or digits only", () => {
  const key = signer();
  for (const good of ["ABCD2345", "ZZZZZZZZ", "00000000"]) {
    assert.equal(checkFormat(bodyFor(key, { short_id: good })), true, good);
  }
  for (const bad of ["ABCD234", "ABCD23456", "abcd2345", "ABCD-234", "", "ABCD 234"]) {
    assert.equal(checkFormat(bodyFor(key, { short_id: bad })), false, bad);
  }
});

test("format refuses bad shapes", () => {
  const key = signer();
  const good = bodyFor(key);
  assert.equal(checkFormat({ ...good, pk: good.pk.toUpperCase() }), false);
  assert.equal(checkFormat({ ...good, noise_pk: "AB".repeat(32) }), false);
  assert.equal(checkFormat({ ...good, noise_pk: undefined }), false);
  assert.equal(checkFormat({ ...good, short_id: undefined }), false);
  assert.equal(checkFormat({ ...good, name: "" }), false);
  assert.equal(checkFormat({ ...good, name: "a\u0007b" }), false);
  assert.equal(checkFormat({ ...good, name: "x".repeat(33) }), false);
  assert.equal(checkFormat({ ...good, nonce: "ABCD" }), false);
  assert.equal(checkFormat({ ...good, sig: good.sig.slice(0, 127) }), false);
  assert.equal(checkFormat({ ...good, ts: 1.5 }), false);
  assert.equal(checkFormat({ ...good, ts: "1700000000" }), false);
  assert.equal(checkFormat(null), false);
  assert.equal(checkFormat([good]), false);
});

test("name length counts characters, not bytes", () => {
  const key = signer();
  assert.equal(checkFormat(bodyFor(key, { name: "가".repeat(32) })), true);
  assert.equal(checkFormat(bodyFor(key, { name: "가".repeat(33) })), false);
});

test("time window is 300 seconds either side", () => {
  assert.equal(withinWindow(TS, TS + 300), true);
  assert.equal(withinWindow(TS, TS - 300), true);
  assert.equal(withinWindow(TS, TS + 301), false);
  assert.equal(withinWindow(TS, TS - 301), false);
});

test("a change to any signed field breaks the signature", async () => {
  const key = signer();
  const body = bodyFor(key);
  assert.equal(await signatureOk({ ...body, noise_pk: "cd".repeat(32) }), false);
  assert.equal(await signatureOk({ ...body, short_id: "ABCD2346" }), false, "short_id is signed");
  assert.equal(await signatureOk({ ...body, name: "Bob" }), false);
  assert.equal(await signatureOk({ ...body, pk: signer().pk }), false);
});

test("the full check order: ok, then time, then signature", async () => {
  const key = signer();
  const body = bodyFor(key);
  assert.equal(await checkBeforeNonce(body, TS), "ok");
  assert.equal(await checkBeforeNonce(body, TS + 301), "time");
  assert.equal(await checkBeforeNonce({ ...body, name: "Bob" }, TS), "signature");
  assert.equal(await checkBeforeNonce({ ...body, sig: "00".repeat(64) }, TS), "signature");
  assert.equal(await checkBeforeNonce({ nope: true }, TS), "format");
});

test("status codes: 204 ok, 400 format, 401 for time, signature and nonce", () => {
  assert.equal(statusFor("ok"), 204);
  assert.equal(statusFor("format"), 400);
  assert.equal(statusFor("time"), 401);
  assert.equal(statusFor("signature"), 401);
  assert.equal(statusFor("nonce"), 401);
});

// v3 vector from mobile/test-vectors/relay.json (chef's OpenSSL signature). Same values as the Rust and mobile tests.
test("v3 vector: the signature matches the external vector", async () => {
  const body: PutBody = {
    pk: "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8",
    noise_pk: "ab".repeat(32),
    short_id: "AB12CD34",
    name: "Alice",
    ts: 1700000000,
    nonce: "00112233445566778899aabbccddeeff",
    sig: "5a5d9f0ee7490ec09cf1eb71643d08c89e840a7b0a4209d32d328a38098a43a0c2c31ddb456d5ff719d599c63162511acd51bb8c558cd065d212bf0502d77301",
  };
  assert.equal(new TextDecoder().decode(signedMessage(body.pk, body.noise_pk, body.short_id, body.name, body.ts, body.nonce)).split("\n").length, 7);
  assert.equal(await signatureOk(body), true);
  assert.equal(await signatureOk({ ...body, short_id: "AB12CD35" }), false);
});
