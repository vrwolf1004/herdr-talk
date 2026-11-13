// Node test (node --test). The same file runs in both room folders. The vectors were made by the Python reference
// (python/src/relay_rest.py room_message) with the seed bytes 0..31; the Rust side uses the same values.
import assert from "node:assert/strict";
import { test } from "node:test";
import { checkBeforeNonce, checkFormat, roomMessage, signatureOk, validName, withinWindow, WINDOW_SECONDS } from "./checks.ts";

const PK = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const NONCE = "00112233445566778899aabbccddeeff";
const TS = 1700000000;
const CREATE_SIG = "f26ec1d02def0df18204cf6507e74e121432cf29ff999b89f30b3137f6084fd2e10b3b6a5d1982a13a531547bbb23370fe821a91a27af0b616b3a05c9b718b0c";
const JOIN_SIG = "4128444185a03e137deca1e066f73d7bf2121162a1f620c01b75f0e217511189388e4da58a7774bb5b18f0e2263a77ff65136eeab8b6a765ac53ca8a15f33c08";

const create = { peer_pk: PK, display_name: "Alice", ts: TS, nonce: NONCE, sig: CREATE_SIG };
const join = { peer_pk: PK, room_code: "ABCD1234", display_name: "Bob", ts: TS, nonce: NONCE, sig: JOIN_SIG };

test("the signed bytes are the fixed string (create has an empty room line)", () => {
  const text = new TextDecoder().decode(roomMessage("create", "", PK, "Alice", TS, NONCE));
  assert.equal(text, ["herdr-talk-room-v1", "create", "", PK, "Alice", "1700000000", NONCE].join(String.fromCharCode(10)));
});

test("the fixed vectors verify, for create and for join", async () => {
  assert.equal(await signatureOk(create, "create"), true);
  assert.equal(await signatureOk(join, "join"), true);
});

test("the action and the room code are part of the signature", async () => {
  assert.equal(await signatureOk(create, "join"), false, "a create signature is not a join");
  assert.equal(await signatureOk({ ...join, room_code: "ABCD1235" }, "join"), false, "another room");
  assert.equal(await signatureOk({ ...create, display_name: "Mallory" }, "create"), false, "another name");
  assert.equal(await signatureOk({ ...create, ts: TS + 1 }, "create"), false, "another ts");
});

test("format: pk, nonce, sig, name and (for join) the room code have fixed shapes", () => {
  assert.equal(checkFormat(create, "create"), true);
  assert.equal(checkFormat(join, "join"), true);
  assert.equal(checkFormat({ ...create, peer_pk: PK.toUpperCase() }, "create"), false);
  assert.equal(checkFormat({ ...create, nonce: "ab" }, "create"), false);
  assert.equal(checkFormat({ ...create, sig: "00" }, "create"), false);
  assert.equal(checkFormat({ ...create, ts: 1.5 }, "create"), false);
  assert.equal(checkFormat({ ...join, room_code: undefined }, "join"), false, "join needs a room code");
  assert.equal(checkFormat({ ...join, room_code: "abcd1234" }, "join"), false);
  assert.equal(checkFormat("not an object", "create"), false);
});

test("names: 1 to 32 characters without control characters", () => {
  assert.equal(validName("A"), true);
  assert.equal(validName("가".repeat(32)), true);
  assert.equal(validName(""), false);
  assert.equal(validName("가".repeat(33)), false);
  assert.equal(validName("a" + String.fromCharCode(7) + "b"), false);
});

test("time window: |ts - now| <= 300 seconds", () => {
  assert.equal(withinWindow(TS, TS), true);
  assert.equal(withinWindow(TS - WINDOW_SECONDS, TS), true);
  assert.equal(withinWindow(TS - WINDOW_SECONDS - 1, TS), false);
  assert.equal(withinWindow(TS + WINDOW_SECONDS + 1, TS), false);
});

test("checks run in order: format, then time, then signature", async () => {
  assert.equal(await checkBeforeNonce({ ...create, nonce: "x" }, "create", TS), "format");
  assert.equal(await checkBeforeNonce(create, "create", TS + 1000), "time");
  assert.equal(await checkBeforeNonce({ ...create, display_name: "Mallory" }, "create", TS), "signature");
  assert.equal(await checkBeforeNonce(create, "create", TS), "ok");
  assert.equal(await checkBeforeNonce(join, "join", TS), "ok");
});
