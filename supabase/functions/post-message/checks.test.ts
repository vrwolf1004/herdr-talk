// Node test (node --test). Signing uses Node's own Ed25519, so the checks are tested against an independent signer.
// The digest literal is the same value the Rust talk-core test checks (room message deliveries digest).
import assert from "node:assert/strict";
import { test } from "node:test";
import { createPrivateKey, createPublicKey, sign } from "node:crypto";
import { bytesToHex, checkBeforeNonce, checkFormat, deliveriesDigest, signedMessage, WINDOW_SECONDS } from "./checks.ts";

const NL = String.fromCharCode(10);
const TS = 1700000000;
const NONCE = "00112233445566778899aabbccddeeff";
const MESSAGE_ID = "ffeeddccbbaa99887766554433221100";
const RECIPIENT_A = "a".repeat(64);
const RECIPIENT_B = "b".repeat(64);

// PKCS#8 wrapper for a raw 32-byte Ed25519 seed.
function keyFromSeed(seedHex: string) {
  const privateKey = createPrivateKey({ key: Buffer.from("302e020100300506032b657004220420" + seedHex, "hex"), format: "der", type: "pkcs8" });
  const spki = createPublicKey(privateKey).export({ format: "der", type: "spki" });
  return { privateKey, pk: bytesToHex(new Uint8Array(spki.subarray(spki.length - 32))) };
}

async function signedBody(seedHex: string, overrides: Record<string, unknown> = {}) {
  const key = keyFromSeed(seedHex);
  const deliveries = [
    { recipient_pk: RECIPIENT_A, ciphertext: "0102" },
    { recipient_pk: RECIPIENT_B, ciphertext: "0304" },
  ];
  const body = { room_code: "ABCD1234", sender_pk: key.pk, message_id: MESSAGE_ID, ts: TS, nonce: NONCE, deliveries, sig: "", ...overrides };
  body.sig = sign(null, Buffer.from(await signedMessage(body as never)), key.privateKey).toString("hex");
  return body;
}

test("the deliveries digest is the fixed literal the Rust side checks too", async () => {
  const digest = await deliveriesDigest([
    { recipient_pk: RECIPIENT_A, ciphertext: "0102" },
    { recipient_pk: RECIPIENT_B, ciphertext: "0304" },
  ]);
  assert.equal(digest, "3db42da64fc2575ca5eaf89fe0573df1fe77200fe1873e89fa61fbbf6c367de0");
});

test("a signed post passes every check", async () => {
  const body = await signedBody("00".repeat(31) + "01");
  assert.equal(checkFormat(body), true);
  assert.equal(await checkBeforeNonce(body, TS), "ok");
});

test("format: recipients must be unique and ascending; ciphertext is hex", async () => {
  const body = await signedBody("00".repeat(31) + "01");
  assert.equal(checkFormat({ ...body, deliveries: [...body.deliveries].reverse() }), false, "descending order");
  assert.equal(checkFormat({ ...body, deliveries: [body.deliveries[0], body.deliveries[0]] }), false, "duplicate");
  assert.equal(checkFormat({ ...body, deliveries: [{ recipient_pk: RECIPIENT_A, ciphertext: "xyz" }] }), false, "not hex");
  assert.equal(checkFormat({ ...body, deliveries: [] }), false, "no recipient");
  assert.equal(checkFormat({ ...body, room_code: "abcd1234" }), false, "room code case");
});

test("the signature covers the deliveries, so a changed ciphertext is refused", async () => {
  const body = await signedBody("00".repeat(31) + "01");
  const tampered = { ...body, deliveries: [{ recipient_pk: RECIPIENT_A, ciphertext: "0105" }, body.deliveries[1]] };
  assert.equal(await checkBeforeNonce(tampered, TS), "signature");
});

test("time window: |ts - now| <= 300 seconds", async () => {
  const body = await signedBody("00".repeat(31) + "01");
  assert.equal(await checkBeforeNonce(body, TS + WINDOW_SECONDS), "ok");
  assert.equal(await checkBeforeNonce(body, TS + WINDOW_SECONDS + 1), "time");
});

test("a signature by another key is refused", async () => {
  const body = await signedBody("00".repeat(31) + "01");
  const other = keyFromSeed("00".repeat(31) + "02");
  assert.equal(await checkBeforeNonce({ ...body, sender_pk: other.pk }, TS), "signature");
});
