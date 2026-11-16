// Node test (node --test). Signing uses Node's own Ed25519, an independent signer.
import assert from "node:assert/strict";
import { test } from "node:test";
import { createPrivateKey, createPublicKey, sign } from "node:crypto";
import { checkBeforeNonce, checkFormat, signedMessage, WINDOW_SECONDS, type FetchBody } from "./checks.ts";

const TS = 1700000000;
const NONCE = "00112233445566778899aabbccddeeff";

function keyFromSeed(seedHex: string) {
  const privateKey = createPrivateKey({ key: Buffer.from("302e020100300506032b657004220420" + seedHex, "hex"), format: "der", type: "pkcs8" });
  const spki = createPublicKey(privateKey).export({ format: "der", type: "spki" });
  return { privateKey, pk: Buffer.from(spki.subarray(spki.length - 32)).toString("hex") };
}

function signedBody(seedHex: string, overrides: Record<string, unknown> = {}): FetchBody {
  const key = keyFromSeed(seedHex);
  const body = { room_code: "ABCD1234", recipient_pk: key.pk, ts: TS, nonce: NONCE, sig: "", ...overrides } as FetchBody;
  body.sig = sign(null, Buffer.from(signedMessage(body)), key.privateKey).toString("hex");
  return body;
}

test("a signed fetch passes; the optional since is only a filter", async () => {
  const body = signedBody("00".repeat(31) + "01");
  assert.equal(await checkBeforeNonce(body, TS), "ok");
  assert.equal(checkFormat({ ...body, since: "2026-11-16T10:00:00Z" }), true);
  assert.equal(await checkBeforeNonce({ ...body, since: "2026-11-16T10:00:00Z" }, TS), "ok", "since does not change the signature");
});

test("format: room code, recipient key, nonce and sig have fixed shapes; since has a length limit", () => {
  const body = signedBody("00".repeat(31) + "01");
  assert.equal(checkFormat({ ...body, room_code: "abcd1234" }), false);
  assert.equal(checkFormat({ ...body, recipient_pk: "AB".repeat(32) }), false);
  assert.equal(checkFormat({ ...body, nonce: "ab" }), false);
  assert.equal(checkFormat({ ...body, since: "x".repeat(41) }), false);
  assert.equal(checkFormat("not an object"), false);
});

test("time window and signature", async () => {
  const body = signedBody("00".repeat(31) + "01");
  assert.equal(await checkBeforeNonce(body, TS + WINDOW_SECONDS + 1), "time");
  const other = keyFromSeed("00".repeat(31) + "02");
  assert.equal(await checkBeforeNonce({ ...body, recipient_pk: other.pk }, TS), "signature", "another key cannot fetch this inbox");
  assert.equal(await checkBeforeNonce({ ...body, room_code: "ABCD1235" }, TS), "signature", "the room is signed");
});
