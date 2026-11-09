// Cross-check vectors for the relay: fixed keys and fixed inputs, computed here in JavaScript.
// The file mobile/test-vectors/relay.json holds these outputs. The Rust test (talk-core relay_vectors_test)
// reads the same file and must compute the same values with its own code; the node test below checks this side.

import * as ed25519 from "@noble/ed25519";
import { hexToBytes, bytesToHex, utf8Encode } from "./bytes.ts";
import { channelName, encodePayload, fingerprint, sealEnvelope, signPut } from "./protocol.ts";
import { publicKeyOf, writeMessage1 } from "./noise.ts";

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
  rowShortId: "ab12cd34",
  rowTs: 1700000000,
  rowNonce: "00112233445566778899aabbccddeeff",
  // The delete-peer vector (signed by the same seed; the signed string is herdr-talk-del-v1).
  deleteShortId: "ab12cd34",
  deleteTs: 1700000000,
  deleteNonce: "00112233445566778899aabbccddeeff",
  // The two public keys of the known channel-name vector (0x01 x 32 and 0x02 x 32).
  channelKeyA: "01".repeat(32),
  channelKeyB: "02".repeat(32),
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
  };
}
