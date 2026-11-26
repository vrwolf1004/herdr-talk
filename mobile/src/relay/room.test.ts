// Room protocol tests. The signatures were made by the Python edition (python/src/core/relay_rest.py) with the same fixed inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import { bytesToHex, hexToBytes } from "./bytes.ts";
import { publicKeyOf } from "./noise.ts";
import { deleteBody, NonceCache } from "./protocol.ts";
import { checkRow, createRoomBody, deliveriesDigest, displayName, fetchMessageBody, joinRoomBody, loadMembers, parseCreated, parseFetch, parseJoin, postMessageBody, roomPayload, sealDeliveries, type Row } from "./room.ts";
import { emptySettings } from "./settings.ts";

import { computeVectors, INPUTS } from "./vectors.ts";


const SEED = new Uint8Array(32).fill(1);
const NONCE = "ab".repeat(16);
const MESSAGE_ID = "cd".repeat(16);
const DELIVERIES = [
  { recipient_pk: "11".repeat(32), ciphertext: "aabb" },
  { recipient_pk: "22".repeat(32), ciphertext: "ccdd" },
];

test("join, post and fetch signatures equal the Python edition's", () => {
  const join = JSON.parse(joinRoomBody(SEED, "ABCD2345", "Tester", "12345678", 1700000000, NONCE));
  assert.equal(join.peer_pk, "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c");
  assert.equal(join.trust, "12345678", "the trust code is sent, and the signature does not change");
  assert.equal(join.sig, "b789bab2a831bb6460c80a8acf5820df3cb8f2bb93dd3031aabbabecf178c7195aa608f26d1a22d33afe4b04243c9484d32365ee20a402f14bcffcf1cb7ec00a");
  const post = JSON.parse(postMessageBody(SEED, "ABCD2345", MESSAGE_ID, 1700000000, NONCE, DELIVERIES));
  assert.equal(post.sig, "b2fcd6781c9ca1ac85c89b9fe33e163bd08655aec59976e5ee5d2f2836ce60b0f0a0617acccea32854b281f8494615f988ff6c117b96cc32c88457a7c5e4cb05");
  const fetch = JSON.parse(fetchMessageBody(SEED, "ABCD2345", null, 1700000000, NONCE));
  assert.equal(fetch.sig, "06a9ab56820fb1b013913de4c147eb5ce4b70cf4e07a7c47fd788ecd01440ebb5c2e290c97cf83c6aa19006e9a76bf590f7f7c67eb7a52c6109b8a66de911b0f");
  assert.equal(deliveriesDigest(DELIVERIES), "6a3f33702b5f9ddabd5f8ecb37c0f4af2a9d55cb6c959abc64af525fe67a8e10");
});

test("post refuses unordered recipients, too many recipients and bad shapes", () => {
  assert.throws(() => postMessageBody(SEED, "ABCD2345", MESSAGE_ID, 1, NONCE, [DELIVERIES[1], DELIVERIES[0]]));
  assert.throws(() => postMessageBody(SEED, "ABCD2345", MESSAGE_ID, 1, NONCE, []));
  assert.throws(() => postMessageBody(SEED, "abcd2345", MESSAGE_ID, 1, NONCE, DELIVERIES));
  assert.throws(() => joinRoomBody(SEED, "ABCD2345", "", "12345678", 1, NONCE));
  assert.throws(() => joinRoomBody(SEED, "ABCD2345", "Tester", "1234", 1, NONCE), "trust code must be 8 digits");
});

test("answers: join (410 is expiry), fetch rows are checked", () => {
  const pk = "aa".repeat(32);
  assert.deepEqual(parseJoin(200, JSON.stringify({ owner_pk: pk, members: [pk] })).members, [pk]);
  assert.throws(() => parseJoin(410, ""), /expired/);
  assert.throws(() => parseJoin(403, ""), /trust 코드가 맞지 않습니다/);
  assert.throws(() => parseJoin(409, ""), /방 인원이 가득 찼습니다 \(10명\)/);
  assert.throws(() => parseJoin(200, JSON.stringify({ owner_pk: pk, members: ["x"] })));
  assert.throws(() => parseFetch(500, ""));
  assert.throws(() => parseFetch(200, JSON.stringify({ messages: [{ message_id: "x" }] })));
  assert.deepEqual(parseFetch(200, JSON.stringify({ messages: [] })), []);
});

// Two phones: a sender and a receiver with their own Noise and signing keys.
const senderNoise = new Uint8Array(32).fill(7);
const receiverNoise = new Uint8Array(32).fill(9);
const senderPk = "bb".repeat(32);
const senderNoiseHex = bytesToHex(publicKeyOf(senderNoise));
const members = new Map([[senderPk, senderNoiseHex]]);
let counter = 1;
const ephemeral = () => new Uint8Array(32).fill(counter++);

function rowFrom(name: string, text: string, ts: number, nonce: string): Row {
  const [delivery] = sealDeliveries(senderNoise, [{ pk: "cc".repeat(32), noise: bytesToHex(publicKeyOf(receiverNoise)) }], roomPayload(name, text, ts, nonce), ephemeral);
  return { message_id: MESSAGE_ID, sender_pk: senderPk, ts, nonce, ciphertext: delivery.ciphertext, created_at: "2026-11-18T00:00:00Z" };
}

test("a trusted member's message is shown once; replay, untrusted, wrong sender and old rows are not", () => {
  const trusted = (noise: string) => noise === senderNoiseHex;
  const row = rowFrom("지호", "안녕", 1000, NONCE);
  const cache = new NonceCache();
  assert.deepEqual(checkRow(row, members, receiverNoise, trusted, cache, 1000), { name: "지호", text: "안녕" });
  assert.equal(checkRow(row, members, receiverNoise, trusted, cache, 1000), null); // replay
  const fresh = rowFrom("지호", "again", 1000, "ef".repeat(16));
  assert.equal(checkRow(fresh, members, receiverNoise, () => false, new NonceCache(), 1000), null); // not trusted
  assert.equal(checkRow({ ...fresh, sender_pk: "dd".repeat(32) }, members, receiverNoise, trusted, new NonceCache(), 1000), null); // not a member
  assert.equal(checkRow(fresh, new Map([[senderPk, bytesToHex(publicKeyOf(receiverNoise))]]), receiverNoise, () => true, new NonceCache(), 1000), null); // key differs from the member's
  assert.equal(checkRow(fresh, members, receiverNoise, trusted, new NonceCache(), 1000, 1001), null); // before this phone started
  assert.equal(checkRow({ ...fresh, nonce: "00".repeat(16) }, members, receiverNoise, trusted, new NonceCache(), 1000), null); // payload nonce differs
  assert.equal(checkRow(rowFrom("x", "late", 1000, "12".repeat(16)), members, receiverNoise, trusted, new NonceCache(), 2000), null); // outside the window
  assert.equal(checkRow(rowFrom("x", "   ", 1000, "34".repeat(16)), members, receiverNoise, trusted, new NonceCache(), 1000), null); // empty text
});

test("a message for another receiver does not open", () => {
  const row = rowFrom("x", "hi", 1000, NONCE);
  assert.equal(checkRow(row, members, new Uint8Array(32).fill(5), () => true, new NonceCache(), 1000), null);
});

test("same name from two keys gets the key's first 4 characters", () => {
  const names = new Map([["aaaa1", "Jiho"], ["bbbb2", "Jiho"]]);
  assert.equal(displayName(names, "aaaa1", "Jiho"), "Jiho (aaaa)");
  assert.equal(displayName(new Map([["aaaa1", "Jiho"]]), "aaaa1", "Jiho"), "Jiho");
});

test("loadMembers joins, then reads the Noise keys of the other members only", async () => {
  const calls: string[] = [];
  const other = "ee".repeat(32);
  const settings = { ...emptySettings(), url: "https://x.supabase.co/", key: "k", name: "Tester", roomCode: "ABCD2345", trust: "12345678" };
  const ownPk = "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c";
  const fetcher = async (url: string) => {
    calls.push(url);
    const body = url.includes("join-room") ? { owner_pk: ownPk, members: [ownPk, other] } : [{ pk: other, noise_pk: "ff".repeat(32) }];
    return { status: 200, text: async () => JSON.stringify(body) };
  };
  const result = await loadMembers(settings, SEED, 1700000000, NONCE, fetcher);
  assert.equal(result.ownerPk, ownPk);
  assert.deepEqual([...result.members], [[other, "ff".repeat(32)]]);
  assert.match(calls[1], new RegExp(`pk=in\\.\\(${other}\\)`));
  assert.equal(calls[0], "https://x.supabase.co/functions/v1/join-room");
});

test("delete-peer body signs like the shared delete vector", () => {
  const body = JSON.parse(deleteBody(hexToBytes(INPUTS.signingSeed), INPUTS.deleteShortId, INPUTS.deleteTs, INPUTS.deleteNonce));
  assert.equal(body.sig, computeVectors().deleteSignature);
  assert.deepEqual(Object.keys(body), ["pk", "short_id", "ts", "nonce", "sig"]);
});

test("create-room body signs like the Python edition and the answer must carry a valid code", () => {
  const body = JSON.parse(createRoomBody(SEED, "Tester", "12345678", 1700000000, NONCE));
  assert.deepEqual(Object.keys(body), ["peer_pk", "display_name", "ts", "nonce", "trust", "sig"]);
  assert.equal(body.sig, "9177c9d12026f452226a534dc0fdb60cfee8a0350b6c08e7093c8abe824819fbecb1ca6b0911091f8870e2af154a9efcdd765577d4dee1c960f9041d87871a08");
  assert.equal(parseCreated(200, JSON.stringify({ room_code: "ABCD2345" })), "ABCD2345");
  assert.throws(() => parseCreated(200, JSON.stringify({ room_code: "abc" })));
  assert.throws(() => parseCreated(401, ""));
});
