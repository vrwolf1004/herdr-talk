// Cross-check vectors for the relay: fixed keys and fixed inputs, computed here in JavaScript.
// The file mobile/test-vectors/relay.json holds these outputs. The Rust test (talk-core relay_vectors_test)
// reads the same file and must compute the same values with its own code; the node test below checks this side.

import * as ed25519 from "@noble/ed25519";
import { hexToBytes, bytesToHex, utf8Encode } from "./bytes.ts";
import { channelName, encodePayload, fingerprint, NonceCache, sealEnvelope, signPut } from "./protocol.ts";
import { publicKeyOf, writeMessage1 } from "./noise.ts";
import { deliveriesDigest, fetchMessageBody, membersQuery, postMessageBody, roomPayload, FETCH_TAG, POST_TAG } from "./room.ts";

export const INPUTS = {
  // Noise static and ephemeral private keys (32 bytes each). Public keys are derived below.
  initiatorStaticPrivate: "11".repeat(32),
  responderStaticPrivate: "22".repeat(32),
  ephemeralPrivate: "33".repeat(32),
  payloadText: "hello relay",
  payloadTs: 1700000000,
  payloadNonce: "00112233445566778899aabbccddeeff",
  // Ed25519 seed for the signed row write (put-peer v3).
  signingSeed: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
  rowNoisePk: "ab".repeat(32),
  rowName: "Alice",
  rowShortId: "AB12CD34",
  rowTs: 1700000000,
  rowNonce: "00112233445566778899aabbccddeeff",
  // The delete-peer vector (signed by the same seed; the signed string is herdr-talk-del-v1).
  deleteShortId: "AB12CD34",
  deleteTs: 1700000000,
  deleteNonce: "00112233445566778899aabbccddeeff",
  // The two public keys of the known channel-name vector (0x01 x 32 and 0x02 x 32).
  channelKeyA: "01".repeat(32),
  channelKeyB: "02".repeat(32),
  // Room messages (the values come from the Python side, core/room.py): three phones, one fixed ephemeral key.
  roomSenderSeed: "33".repeat(32),
  roomSenderNoisePrivate: "44".repeat(32),
  roomRecipientSeed: "77".repeat(32),
  roomRecipientNoisePrivate: "55".repeat(32),
  roomEphemeralPrivate: "66".repeat(32),
  roomThirdSeed: "88".repeat(32),
  roomThirdNoisePrivate: "99".repeat(32),
  roomCode: "ABCD2345",
  roomTs: 1700000000,
  roomNonce: "00112233445566778899aabbccddeeff",
  roomMessageId: "ffeeddccbbaa99887766554433221100",
  roomName: "Alice",
  roomText: "hello room",
  roomSince: "2026-11-16T00:00:00Z",
} as const;

export type Vectors = {
  initiatorPublic: string;
  responderPublic: string;
  payloadJson: string;
  noiseMessage1: string;
  envelope: string;
  channelFromNoiseKeys: string;
  channelOfKnownKeys: string;
  rowPublic: string;
  rowSignature: string;
  deleteSignature: string;
  fingerprint: string;
  roomSenderPublic: string;
  roomSenderNoisePublic: string;
  roomRecipientPublic: string;
  roomRecipientNoisePublic: string;
  roomPayloadJson: string;
  roomCiphertext: string;
  roomDeliveriesDigest: string;
  roomPostSignedText: string;
  roomPostSig: string;
  roomFetchSignedText: string;
  roomFetchSig: string;
  roomMembersQuery: string;
  roomFetchAnswer: string;
  roomNonceFirstAccept: string;
  roomNonceSecondAccept: string;
};

export function computeVectors(): Vectors {
  const initiatorStatic = hexToBytes(INPUTS.initiatorStaticPrivate);
  const responderStatic = hexToBytes(INPUTS.responderStaticPrivate);
  const ephemeral = hexToBytes(INPUTS.ephemeralPrivate);
  const initiatorPublic = publicKeyOf(initiatorStatic);
  const responderPublic = publicKeyOf(responderStatic);
  const payloadJson = encodePayload(INPUTS.payloadText, INPUTS.payloadTs, INPUTS.payloadNonce);
  const noiseMessage1 = writeMessage1(initiatorStatic, responderPublic, utf8Encode(payloadJson), ephemeral);
  const envelope = sealEnvelope(initiatorPublic, responderPublic, noiseMessage1);
  const row = signPut(hexToBytes(INPUTS.signingSeed), INPUTS.rowNoisePk, INPUTS.rowShortId, INPUTS.rowName, INPUTS.rowTs, INPUTS.rowNonce);
  const deleteMessage = utf8Encode(["herdr-talk-del-v1", row.pk, INPUTS.deleteShortId, String(INPUTS.deleteTs), INPUTS.deleteNonce].join("\n"));
  const deleteSignature = bytesToHex(ed25519.sign(deleteMessage, hexToBytes(INPUTS.signingSeed)));
  return {
    initiatorPublic: bytesToHex(initiatorPublic),
    responderPublic: bytesToHex(responderPublic),
    payloadJson,
    noiseMessage1: bytesToHex(noiseMessage1),
    envelope,
    channelFromNoiseKeys: channelName(initiatorPublic, responderPublic),
    channelOfKnownKeys: channelName(hexToBytes(INPUTS.channelKeyA), hexToBytes(INPUTS.channelKeyB)),
    rowPublic: row.pk,
    rowSignature: row.sig,
    deleteSignature,
    fingerprint: fingerprint(initiatorPublic, responderPublic),
    ...computeRoomVectors(),
  };
}

function computeRoomVectors() {
  const senderSeed = hexToBytes(INPUTS.roomSenderSeed);
  const recipientSeed = hexToBytes(INPUTS.roomRecipientSeed);
  const senderPk = bytesToHex(ed25519.getPublicKey(senderSeed));
  const recipientPk = bytesToHex(ed25519.getPublicKey(recipientSeed));
  const thirdPk = bytesToHex(ed25519.getPublicKey(hexToBytes(INPUTS.roomThirdSeed)));
  const senderNoise = hexToBytes(INPUTS.roomSenderNoisePrivate);
  const recipientNoise = publicKeyOf(hexToBytes(INPUTS.roomRecipientNoisePrivate));
  const payload = roomPayload(INPUTS.roomName, INPUTS.roomText, INPUTS.roomTs, INPUTS.roomNonce);
  const ephemeral = hexToBytes(INPUTS.roomEphemeralPrivate);
  const ciphertext = bytesToHex(writeMessage1(senderNoise, recipientNoise, payload, ephemeral));
  const deliveries = [{ recipient_pk: recipientPk, ciphertext }];
  const digest = deliveriesDigest(deliveries);
  const post = JSON.parse(postMessageBody(senderSeed, INPUTS.roomCode, INPUTS.roomMessageId, INPUTS.roomTs, INPUTS.roomNonce, deliveries));
  const fetch = JSON.parse(fetchMessageBody(recipientSeed, INPUTS.roomCode, null, INPUTS.roomTs, INPUTS.roomNonce));
  const cache = new NonceCache();
  return {
    roomSenderPublic: senderPk,
    roomSenderNoisePublic: bytesToHex(publicKeyOf(senderNoise)),
    roomRecipientPublic: recipientPk,
    roomRecipientNoisePublic: bytesToHex(recipientNoise),
    roomPayloadJson: new TextDecoder().decode(payload),
    roomCiphertext: ciphertext,
    roomDeliveriesDigest: digest,
    roomPostSignedText: [POST_TAG, "post", INPUTS.roomCode, senderPk, INPUTS.roomMessageId, String(INPUTS.roomTs), INPUTS.roomNonce, digest].join("\n"),
    roomPostSig: post.sig as string,
    roomFetchSignedText: [FETCH_TAG, "fetch", INPUTS.roomCode, recipientPk, String(INPUTS.roomTs), INPUTS.roomNonce].join("\n"),
    roomFetchSig: fetch.sig as string,
    roomMembersQuery: membersQuery([thirdPk, recipientPk].sort()),
    roomFetchAnswer: JSON.stringify({ messages: [{ message_id: INPUTS.roomMessageId, sender_pk: senderPk, ts: INPUTS.roomTs, nonce: INPUTS.roomNonce, ciphertext, created_at: INPUTS.roomSince }] }),
    roomNonceFirstAccept: String(cache.accept(INPUTS.roomNonce, INPUTS.roomTs, INPUTS.roomTs)),
    roomNonceSecondAccept: String(cache.accept(INPUTS.roomNonce, INPUTS.roomTs, INPUTS.roomTs)),
  };
}
