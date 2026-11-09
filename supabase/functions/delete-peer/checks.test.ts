// Run with: node --test supabase/functions/delete-peer/checks.test.ts   (Node 22.6+ strips the types itself)
import { test } from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, sign } from "node:crypto";
import { checkBeforeNonce, checkFormat, signatureOk, signedMessage, statusFor, withinWindow, type DeleteBody } from "./checks.ts";

const SHORT_ID = "abcd2345";
const NONCE = "00112233445566778899aabbccddeeff";
const TS = 1700000000;

// A fresh Ed25519 key for each run (no fixed secret in the repo). The signing side is Node's crypto, the checking side is Web Crypto.
function signer() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const x = publicKey.export({ format: "jwk" }).x as string;
  return { pk: Buffer.from(x, "base64url").toString("hex"), privateKey };
}

function bodyFor(key: ReturnType<typeof signer>, overrides: Partial<DeleteBody> = {}): DeleteBody {
  const base = { pk: key.pk, short_id: SHORT_ID, ts: TS, nonce: NONCE };
  const merged = { ...base, ...overrides };
  const message = signedMessage(merged.pk, merged.short_id, merged.ts, merged.nonce);
  return { ...merged, sig: sign(null, Buffer.from(message), key.privateKey).toString("hex") };
}

test("v1 signed message is the fixed order, one per line, no newline at the end", () => {
  const message = new TextDecoder().decode(signedMessage("pk", SHORT_ID, TS, NONCE));
  assert.equal(message, ["herdr-talk-del-v1", "pk", SHORT_ID, "1700000000", NONCE].join("\n"));
});

test("a signed delete body passes the format check and the full check order", async () => {
  const key = signer();
  const body = bodyFor(key);
  assert.equal(checkFormat(body), true);
  assert.equal(await signatureOk(body), true);
  assert.equal(await checkBeforeNonce(body, TS), "ok");
});

test("format accepts a short_id of 8 lowercase letters or digits only", () => {
  const key = signer();
  for (const good of ["abcd2345", "zzzzzzzz", "00000000"]) {
    assert.equal(checkFormat(bodyFor(key, { short_id: good })), true, good);
  }
  for (const bad of ["abcd234", "abcd23456", "ABCD2345", "abcd-234", ""]) {
    assert.equal(checkFormat(bodyFor(key, { short_id: bad })), false, bad);
  }
});

test("format refuses bad shapes", () => {
  const key = signer();
  const good = bodyFor(key);
  assert.equal(checkFormat({ ...good, pk: good.pk.toUpperCase() }), false);
  assert.equal(checkFormat({ ...good, short_id: undefined }), false);
  assert.equal(checkFormat({ ...good, nonce: "ABCD" }), false);
  assert.equal(checkFormat({ ...good, sig: good.sig.slice(0, 127) }), false);
  assert.equal(checkFormat({ ...good, ts: 1.5 }), false);
  assert.equal(checkFormat({ ...good, ts: "1700000000" }), false);
  assert.equal(checkFormat({ pk: good.pk, short_id: SHORT_ID, ts: TS, nonce: NONCE, sig: good.sig, name: "x" }), true, "extra keys are not part of the check");
  assert.equal(checkFormat(null), false);
  assert.equal(checkFormat([good]), false);
});

test("time window is 300 seconds either side", () => {
  assert.equal(withinWindow(TS, TS + 300), true);
  assert.equal(withinWindow(TS, TS - 300), true);
  assert.equal(withinWindow(TS, TS + 301), false);
});

test("a change to pk or short_id breaks the signature (both are signed)", async () => {
  const key = signer();
  const body = bodyFor(key);
  assert.equal(await signatureOk({ ...body, short_id: "abcd2346" }), false);
  assert.equal(await signatureOk({ ...body, pk: signer().pk }), false);
  assert.equal(await signatureOk({ ...body, ts: TS + 1 }), false);
});

test("the put-peer tag does not verify here: the two signed strings differ", async () => {
  const key = signer();
  const putMessage = new TextEncoder().encode(["herdr-talk-put-v3", key.pk, "ab".repeat(32), SHORT_ID, "Alice", String(TS), NONCE].join("\n"));
  const body = { ...bodyFor(key), sig: sign(null, Buffer.from(putMessage), key.privateKey).toString("hex") };
  assert.equal(await signatureOk(body), false);
});

test("the full check order: ok, then time, then signature", async () => {
  const key = signer();
  const body = bodyFor(key);
  assert.equal(await checkBeforeNonce(body, TS), "ok");
  assert.equal(await checkBeforeNonce(body, TS + 301), "time");
  assert.equal(await checkBeforeNonce({ ...body, short_id: "abcd2346" }, TS), "signature");
  assert.equal(await checkBeforeNonce({ nope: true }, TS), "format");
});

test("status codes: 204 ok, 400 format, 401 for time, signature and nonce", () => {
  assert.equal(statusFor("ok"), 204);
  assert.equal(statusFor("format"), 400);
  assert.equal(statusFor("time"), 401);
  assert.equal(statusFor("signature"), 401);
  assert.equal(statusFor("nonce"), 401);
});

// Vector from mobile/test-vectors/relay.json (chef's OpenSSL signature, seed bytes(range(32))). Same values as the Rust test.
test("delete vector: the signature matches the external vector", async () => {
  const body: DeleteBody = {
    pk: "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8",
    short_id: "ab12cd34",
    ts: 1700000000,
    nonce: "00112233445566778899aabbccddeeff",
    sig: "8ef091fb1ca2c37949db8bbb5071dbcb7e2b493ba41728898ea4a2356f5e6086c7f1f0630bc5341f2f046626c4aeda109aaf24170100fc7bae89cdf4a3836b08",
  };
  assert.equal(signedMessage(body.pk, body.short_id, body.ts, body.nonce).length, 135);
  assert.equal(await signatureOk(body), true);
  assert.equal(await signatureOk({ ...body, short_id: "ab12cd35" }), false);
});
