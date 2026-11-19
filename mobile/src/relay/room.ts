// Room protocol on the phone, same rules as Python core/relay_rest.py + core/room.py and Rust talk-core
// (relay_rest.rs, room_relay.rs): signed join, signed post (one Noise message per trusted member), signed fetch.
// Pure functions plus thin fetch calls; the screen owns the loop and the storage.

import * as ed25519 from "@noble/ed25519";
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex, hexToBytes, utf8Decode, utf8Encode } from "./bytes.ts";
import { TIMEOUT_MS } from "./client.ts";
import { readMessage1, writeMessage1 } from "./noise.ts";
import { bytesEqual, NonceCache, signingPublicKey } from "./protocol.ts";
import { isRoomCode, isTrustCode, isValidName, type RelaySettings } from "./settings.ts";

export const ROOM_TAG = "herdr-talk-room-v1";
export const POST_TAG = "herdr-talk-msg-v1";
export const FETCH_TAG = "herdr-talk-fetch-v1";
export const MAX_DELIVERIES = 8;
export const MAX_TEXT_CHARS = 2000;

const HEX64 = /^[0-9a-f]{64}$/;
const HEX32 = /^[0-9a-f]{32}$/;

const sign = (seed: Uint8Array, lines: string[]) => bytesToHex(ed25519.sign(utf8Encode(lines.join("\n")), seed));

/** Body of join-room: room_code, peer_pk, display_name, ts, nonce, trust, sig. The trust code is sent but not signed (as in talk-core). */
export function joinRoomBody(seed: Uint8Array, roomCode: string, name: string, trust: string, ts: number, nonce: string): string {
  if (!isRoomCode(roomCode) || !isValidName(name) || !isTrustCode(trust) || !HEX32.test(nonce)) throw new Error("room code, name, trust code or nonce is not valid");
  const pk = bytesToHex(signingPublicKey(seed));
  const sig = sign(seed, [ROOM_TAG, "join", roomCode, pk, name, String(ts), nonce]);
  return JSON.stringify({ room_code: roomCode, peer_pk: pk, display_name: name, ts, nonce, trust, sig });
}

/** Body of create-room: peer_pk, display_name, ts, nonce, trust, sig (the signed room code is empty). */
export function createRoomBody(seed: Uint8Array, name: string, trust: string, ts: number, nonce: string): string {
  if (!isValidName(name) || !isTrustCode(trust) || !HEX32.test(nonce)) throw new Error("name, trust code or nonce is not valid");
  const pk = bytesToHex(signingPublicKey(seed));
  const sig = sign(seed, [ROOM_TAG, "create", "", pk, name, String(ts), nonce]);
  return JSON.stringify({ peer_pk: pk, display_name: name, ts, nonce, trust, sig });
}

/** The room code of a create-room answer (200 only). */
export function parseCreated(status: number, body: string): string {
  if (status !== 200) throw new Error(`create status ${status}`);
  const code = (JSON.parse(body) as { room_code?: unknown }).room_code;
  if (typeof code !== "string" || !isRoomCode(code)) throw new Error("create answer has no valid room_code");
  return code;
}

/** The member keys of a join-room answer (200 only). Throws with the reason otherwise. */
export function parseJoin(status: number, body: string): { ownerPk: string; members: string[] } {
  if (status === 410) throw new Error("room expired (24 hours)");
  if (status === 403) throw new Error("trust 코드가 맞지 않습니다");
  if (status === 409) throw new Error("방 인원이 가득 찼습니다 (10명)");
  if (status !== 200) throw new Error(`join status ${status}`);
  const value = JSON.parse(body) as { owner_pk?: unknown; members?: unknown };
  if (typeof value.owner_pk !== "string" || !HEX64.test(value.owner_pk)) throw new Error("join answer has no owner_pk");
  if (!Array.isArray(value.members) || !value.members.every((pk) => typeof pk === "string" && HEX64.test(pk))) {
    throw new Error("join answer has no valid members");
  }
  return { ownerPk: value.owner_pk, members: value.members as string[] };
}

/** pk -> Noise public key (hex) from the peers answer. */
export function parseMembersNoise(status: number, body: string): Map<string, string> {
  if (status !== 200) throw new Error(`members status ${status}`);
  const rows = JSON.parse(body) as unknown;
  if (!Array.isArray(rows)) throw new Error("members answer is not a list");
  const out = new Map<string, string>();
  for (const row of rows as { pk?: unknown; noise_pk?: unknown }[]) {
    if (typeof row.pk !== "string" || typeof row.noise_pk !== "string" || !HEX64.test(row.pk) || !HEX64.test(row.noise_pk)) {
      throw new Error("members row is not valid");
    }
    out.set(row.pk, row.noise_pk);
  }
  return out;
}

/** SHA-256 hex over "recipient ciphertext" lines, in the order given. */
export function deliveriesDigest(deliveries: { recipient_pk: string; ciphertext: string }[]): string {
  return bytesToHex(sha256(utf8Encode(deliveries.map((row) => `${row.recipient_pk} ${row.ciphertext}`).join("\n"))));
}

export type Delivery = { recipient_pk: string; ciphertext: string };

/** Body of post-message. Recipients must be unique and ascending (the digest depends on it). */
export function postMessageBody(seed: Uint8Array, roomCode: string, messageId: string, ts: number, nonce: string, deliveries: Delivery[]): string {
  if (!isRoomCode(roomCode) || !HEX32.test(messageId) || !HEX32.test(nonce)) throw new Error("room code, message id or nonce is not valid");
  if (deliveries.length < 1 || deliveries.length > MAX_DELIVERIES) throw new Error("a message goes to 1 to 8 recipients");
  deliveries.forEach((row, index) => {
    if (!HEX64.test(row.recipient_pk) || !/^(?:[0-9a-f]{2})+$/.test(row.ciphertext) || row.ciphertext.length > 32768) {
      throw new Error("recipient key or ciphertext is not valid");
    }
    if (index > 0 && row.recipient_pk <= deliveries[index - 1].recipient_pk) throw new Error("recipients must be unique and ascending");
  });
  const senderPk = bytesToHex(signingPublicKey(seed));
  const sig = sign(seed, [POST_TAG, "post", roomCode, senderPk, messageId, String(ts), nonce, deliveriesDigest(deliveries)]);
  return JSON.stringify({ room_code: roomCode, sender_pk: senderPk, message_id: messageId, ts, nonce, deliveries, sig });
}

/** Body of fetch-messages: this recipient's inbox, after `since` (a filter only). */
export function fetchMessageBody(seed: Uint8Array, roomCode: string, since: string | null, ts: number, nonce: string): string {
  if (!isRoomCode(roomCode) || !HEX32.test(nonce)) throw new Error("room code or nonce is not valid");
  const pk = bytesToHex(signingPublicKey(seed));
  const body: Record<string, unknown> = { room_code: roomCode, recipient_pk: pk, ts, nonce, sig: sign(seed, [FETCH_TAG, "fetch", roomCode, pk, String(ts), nonce]) };
  if (since !== null) body.since = since;
  return JSON.stringify(body);
}

export type Row = { message_id: string; sender_pk: string; ts: number; nonce: string; ciphertext: string; created_at: string };

export function parseFetch(status: number, body: string): Row[] {
  if (status !== 200) throw new Error(`fetch status ${status}`);
  const rows = (JSON.parse(body) as { messages?: unknown }).messages;
  if (!Array.isArray(rows)) throw new Error("fetch answer has no messages list");
  return rows.map((row: Record<string, unknown>) => {
    const ok =
      typeof row.message_id === "string" && HEX32.test(row.message_id) &&
      typeof row.sender_pk === "string" && HEX64.test(row.sender_pk) &&
      typeof row.ts === "number" && Number.isSafeInteger(row.ts) &&
      typeof row.nonce === "string" && HEX32.test(row.nonce) &&
      typeof row.ciphertext === "string" && /^(?:[0-9a-f]{2})+$/.test(row.ciphertext) && row.ciphertext.length <= 32768 &&
      typeof row.created_at === "string";
    if (!ok) throw new Error("fetch row is not valid");
    return row as unknown as Row;
  });
}

/** The sealed payload: JSON with v, ts, nonce, name, text, spelled as Python json.dumps does (", " and ": ") so the shared vectors match byte for byte. */
export function roomPayload(name: string, text: string, ts: number, nonce: string): Uint8Array {
  return utf8Encode(`{"v": 1, "ts": ${ts}, "nonce": ${JSON.stringify(nonce)}, "name": ${JSON.stringify(name)}, "text": ${JSON.stringify(text)}}`);
}

/** Query of the peers request for these member keys. */
export function membersQuery(pks: string[]): string {
  return `pk=in.(${pks.join(",")})&select=pk,noise_pk`;
}

/** One Noise IK message per recipient (pk, Noise public key hex), sorted by pk as the post body needs. */
export function sealDeliveries(
  noisePrivate: Uint8Array,
  recipients: { pk: string; noise: string }[],
  payload: Uint8Array,
  ephemeral: () => Uint8Array,
): Delivery[] {
  return [...recipients]
    .sort((a, b) => (a.pk < b.pk ? -1 : a.pk > b.pk ? 1 : 0))
    .map((peer) => ({ recipient_pk: peer.pk, ciphertext: bytesToHex(writeMessage1(noisePrivate, hexToBytes(peer.noise), payload, ephemeral())) }));
}

const CONTROL = /[\u0000-\u001f\u007f-\u009f]/g;

/**
 * The (name, text) to show for one fetched row, or null (every failure is silent). members: sender pk -> Noise key hex.
 * isTrusted: the user confirmed this Noise key. notBefore: rows sent before this time (seconds) are dropped.
 * shortcut: the name shown is the one the trusted sender wrote into the payload; the PC editions show the name stored at /trust.
 */
export function checkRow(
  row: Row,
  members: Map<string, string>,
  noisePrivate: Uint8Array,
  isTrusted: (noiseHex: string) => boolean,
  cache: NonceCache,
  now: number,
  notBefore = 0,
): { name: string; text: string } | null {
  if (row.ts < notBefore) return null;
  const senderNoise = members.get(row.sender_pk);
  if (senderNoise === undefined) return null;
  const opened = readMessage1(noisePrivate, hexToBytes(row.ciphertext));
  if (opened === null || !bytesEqual(opened.initiatorPublic, hexToBytes(senderNoise))) return null;
  if (!isTrusted(senderNoise)) return null;
  let value: { v?: unknown; ts?: unknown; nonce?: unknown; name?: unknown; text?: unknown };
  try {
    value = JSON.parse(utf8Decode(opened.payload));
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null || value.v !== 1 || value.ts !== row.ts || value.nonce !== row.nonce) return null;
  if (!cache.accept(row.nonce, row.ts, now)) return null;
  const text = (typeof value.text === "string" ? value.text : "").replace(CONTROL, " ").slice(0, MAX_TEXT_CHARS);
  if (text.trim() === "") return null;
  const claimed = typeof value.name === "string" ? value.name.replace(CONTROL, " ").replace(/\s+/g, " ").trim() : "";
  return { name: isValidName(claimed) ? claimed : row.sender_pk.slice(0, 4), text };
}

/** The name shown for a sender: plus the first 4 characters of the key when another known key has the same name. */
export function displayName(names: Map<string, string>, pk: string, name: string): string {
  for (const [other, otherName] of names) if (other !== pk && otherName === name) return `${name} (${pk.slice(0, 4)})`;
  return name;
}

type Fetch = (url: string, init: { method: string; headers: Record<string, string>; body?: string }) => Promise<{ status: number; text: () => Promise<string> }>;

async function call(settings: RelaySettings, path: string, method: string, body: string | undefined, fetcher: Fetch): Promise<{ status: number; text: string }> {
  const headers: Record<string, string> = { apikey: settings.key, Authorization: `Bearer ${settings.key}` };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => (timer = setTimeout(() => reject(new Error("timeout")), TIMEOUT_MS)));
  try {
    const response = await Promise.race([fetcher(`${settings.url.trim().replace(/\/+$/, "")}${path}`, { method, headers, body }), timeout]);
    return { status: response.status, text: await response.text() };
  } finally {
    clearTimeout(timer);
  }
}

/** Make a new room owned by this phone. This phone's row must be published first (rooms.owner_pk references it). */
export async function createRoom(
  settings: RelaySettings,
  seed: Uint8Array,
  now: number,
  nonce: string,
  fetcher: Fetch = fetch as unknown as Fetch,
): Promise<string> {
  const answer = await call(settings, "/functions/v1/create-room", "POST", createRoomBody(seed, settings.name, settings.trust, now, nonce), fetcher);
  return parseCreated(answer.status, answer.text);
}

/** Join the room and read the other members' Noise keys. Members without a row in `peers` are left out. */
export async function loadMembers(
  settings: RelaySettings,
  seed: Uint8Array,
  now: number,
  nonce: string,
  fetcher: Fetch = fetch as unknown as Fetch,
): Promise<Map<string, string>> {
  const join = await call(settings, "/functions/v1/join-room", "POST", joinRoomBody(seed, settings.roomCode, settings.name, settings.trust, now, nonce), fetcher);
  const own = bytesToHex(signingPublicKey(seed));
  const others = parseJoin(join.status, join.text).members.filter((pk) => pk !== own);
  if (others.length === 0) return new Map();
  const peers = await call(settings, `/rest/v1/peers?${membersQuery(others)}`, "GET", undefined, fetcher);
  return parseMembersNoise(peers.status, peers.text);
}

export async function fetchRows(
  settings: RelaySettings,
  seed: Uint8Array,
  since: string | null,
  now: number,
  nonce: string,
  fetcher: Fetch = fetch as unknown as Fetch,
): Promise<Row[]> {
  const answer = await call(settings, "/functions/v1/fetch-messages", "POST", fetchMessageBody(seed, settings.roomCode, since, now, nonce), fetcher);
  return parseFetch(answer.status, answer.text);
}

/** True when the server answered 200. Throws on a network error or bad input. */
export async function postBody(settings: RelaySettings, body: string, fetcher: Fetch = fetch as unknown as Fetch): Promise<boolean> {
  return (await call(settings, "/functions/v1/post-message", "POST", body, fetcher)).status === 200;
}
