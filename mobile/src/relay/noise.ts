// Noise_IK_25519_ChaChaPoly_BLAKE2s, first message only, as the relay uses it: the initiator sends one message that
// carries the payload (e, es, s, ss, payload). The responder learns and checks the initiator's static key from it.
// Same protocol as the Rust talk-core relay (snow) and the Python edition; checked against shared test vectors.

import { x25519 } from "@noble/curves/ed25519.js";
import { chacha20poly1305 } from "@noble/ciphers/chacha.js";
import { blake2s } from "@noble/hashes/blake2.js";
import { hmac } from "@noble/hashes/hmac.js";
import { concatBytes, utf8Encode } from "./bytes.ts";

const PROTOCOL = utf8Encode("Noise_IK_25519_ChaChaPoly_BLAKE2s");
const TAG_BYTES = 16;

export function publicKeyOf(privateKey: Uint8Array): Uint8Array {
  return x25519.getPublicKey(privateKey);
}

function hash(data: Uint8Array): Uint8Array {
  return blake2s(data, { dkLen: 32 });
}

function nonce12(counter: number): Uint8Array {
  // Noise nonce: 4 zero bytes, then the 64-bit counter little-endian.
  const out = new Uint8Array(12);
  let value = counter;
  for (let i = 4; i < 12; i++) {
    out[i] = value % 256;
    value = Math.floor(value / 256);
  }
  return out;
}

/** HKDF with two outputs, as Noise defines it (HMAC-BLAKE2s). */
function hkdf2(chainingKey: Uint8Array, input: Uint8Array): [Uint8Array, Uint8Array] {
  const temp = hmac(blake2s, chainingKey, input);
  const out1 = hmac(blake2s, temp, Uint8Array.of(1));
  const out2 = hmac(blake2s, temp, concatBytes(out1, Uint8Array.of(2)));
  return [out1, out2];
}

class SymmetricState {
  h: Uint8Array;
  ck: Uint8Array;
  key: Uint8Array | null = null;
  n = 0;

  constructor() {
    // Protocol name is 33 bytes (longer than 32), so the state starts from its hash.
    this.h = PROTOCOL.length <= 32 ? concatBytes(PROTOCOL, new Uint8Array(32 - PROTOCOL.length)) : hash(PROTOCOL);
    this.ck = this.h;
  }

  mixHash(data: Uint8Array): void {
    this.h = hash(concatBytes(this.h, data));
  }

  mixKey(input: Uint8Array): void {
    const [ck, key] = hkdf2(this.ck, input);
    this.ck = ck;
    this.key = key;
    this.n = 0;
  }

  encryptAndHash(plaintext: Uint8Array): Uint8Array {
    if (this.key === null) {
      this.mixHash(plaintext);
      return plaintext;
    }
    const ciphertext = chacha20poly1305(this.key, nonce12(this.n), this.h).encrypt(plaintext);
    this.n += 1;
    this.mixHash(ciphertext);
    return ciphertext;
  }

  decryptAndHash(ciphertext: Uint8Array): Uint8Array {
    if (this.key === null) {
      this.mixHash(ciphertext);
      return ciphertext;
    }
    const plaintext = chacha20poly1305(this.key, nonce12(this.n), this.h).decrypt(ciphertext);
    this.n += 1;
    this.mixHash(ciphertext);
    return plaintext;
  }
}

/**
 * The initiator's single message: e || enc(s) || enc(payload). `ephemeral` is given by the caller
 * (a random key in the app, a fixed key in the test vectors).
 */
export function writeMessage1(
  initiatorStatic: Uint8Array,
  responderPublic: Uint8Array,
  payload: Uint8Array,
  ephemeral: Uint8Array,
  prologue: Uint8Array = new Uint8Array(0),
): Uint8Array {
  const state = new SymmetricState();
  state.mixHash(prologue);
  state.mixHash(responderPublic); // pre-message: the responder's static key is known
  const ephemeralPublic = publicKeyOf(ephemeral);
  state.mixHash(ephemeralPublic);
  state.mixKey(x25519.getSharedSecret(ephemeral, responderPublic)); // es
  const encryptedStatic = state.encryptAndHash(publicKeyOf(initiatorStatic));
  state.mixKey(x25519.getSharedSecret(initiatorStatic, responderPublic)); // ss
  const encryptedPayload = state.encryptAndHash(payload);
  return concatBytes(ephemeralPublic, encryptedStatic, encryptedPayload);
}

/** The responder's reading of that message. Null when it does not verify. */
export function readMessage1(
  responderStatic: Uint8Array,
  message: Uint8Array,
  prologue: Uint8Array = new Uint8Array(0),
): { payload: Uint8Array; initiatorPublic: Uint8Array } | null {
  if (message.length < 32 + 32 + TAG_BYTES + TAG_BYTES) {
    return null;
  }
  try {
    const state = new SymmetricState();
    state.mixHash(prologue);
    state.mixHash(publicKeyOf(responderStatic));
    const ephemeralPublic = message.subarray(0, 32);
    state.mixHash(ephemeralPublic);
    state.mixKey(x25519.getSharedSecret(responderStatic, ephemeralPublic)); // es
    const initiatorPublic = state.decryptAndHash(message.subarray(32, 32 + 32 + TAG_BYTES));
    state.mixKey(x25519.getSharedSecret(responderStatic, initiatorPublic)); // ss
    const payload = state.decryptAndHash(message.subarray(32 + 32 + TAG_BYTES));
    return { payload, initiatorPublic };
  } catch {
    return null;
  }
}
