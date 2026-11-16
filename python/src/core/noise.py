"""Noise_IK_25519_ChaChaPoly_BLAKE2s with the standard library only, the same protocol as the Rust edition (snow).

  X25519           RFC 7748
  ChaCha20-Poly1305 RFC 8439
  BLAKE2s, HMAC    hashlib, hmac
  Handshake        Noise protocol framework rev 34, pattern IK

shortcut: pure Python big-integer math is not constant time, so a process that can time this one on the same PC may learn
bits of a key; upgrade to the `cryptography` package if that matters. Keys never leave the PC, so a network peer cannot time it.
"""

import hashlib
import hmac
import os
import struct

PROTOCOL = b"Noise_IK_25519_ChaChaPoly_BLAKE2s"
KEY_BYTES = 32
TAG_BYTES = 16

_P = 2**255 - 19
_A24 = 121665


class NoiseError(Exception):
    """A handshake or message that does not verify."""


# ---------------------------------------------------------------- X25519

def x25519(scalar, point):
    """RFC 7748 section 5. scalar and point are 32 bytes; returns 32 bytes."""
    if len(scalar) != 32 or len(point) != 32:
        raise NoiseError("bad key length")
    k = bytearray(scalar)
    k[0] &= 248
    k[31] &= 127
    k[31] |= 64
    k = int.from_bytes(k, "little")
    x1 = int.from_bytes(point, "little") & ((1 << 255) - 1)
    x2, z2, x3, z3, swap = 1, 0, x1, 1, 0
    for t in range(254, -1, -1):
        bit = (k >> t) & 1
        swap ^= bit
        if swap:
            x2, x3, z2, z3 = x3, x2, z3, z2
        swap = bit
        a, b = (x2 + z2) % _P, (x2 - z2) % _P
        aa, bb = a * a % _P, b * b % _P
        e = (aa - bb) % _P
        c, d = (x3 + z3) % _P, (x3 - z3) % _P
        da, cb = d * a % _P, c * b % _P
        x3, z3 = (da + cb) ** 2 % _P, x1 * (da - cb) ** 2 % _P
        x2, z2 = aa * bb % _P, e * (aa + _A24 * e) % _P
    if swap:
        x2, x3, z2, z3 = x3, x2, z3, z2
    return (x2 * pow(z2, _P - 2, _P) % _P).to_bytes(32, "little")


def public_key(secret):
    return x25519(secret, b"\x09" + b"\x00" * 31)


def generate_keypair():
    """(public, secret), both 32 bytes."""
    secret = os.urandom(32)
    return public_key(secret), secret


def _dh(secret, public):
    shared = x25519(secret, public)
    if shared == b"\x00" * 32:
        raise NoiseError("low-order public key")  # a key that makes the shared secret known to anyone
    return shared


# ---------------------------------------------------------------- ChaCha20-Poly1305

_MASK = 0xFFFFFFFF


def _chacha_block(key, counter, nonce):
    state = list(struct.unpack("<16I", b"expand 32-byte k" + key + struct.pack("<I", counter) + nonce))
    work = state[:]

    def quarter(a, b, c, d):
        work[a] = (work[a] + work[b]) & _MASK
        work[d] ^= work[a]
        work[d] = ((work[d] << 16) | (work[d] >> 16)) & _MASK
        work[c] = (work[c] + work[d]) & _MASK
        work[b] ^= work[c]
        work[b] = ((work[b] << 12) | (work[b] >> 20)) & _MASK
        work[a] = (work[a] + work[b]) & _MASK
        work[d] ^= work[a]
        work[d] = ((work[d] << 8) | (work[d] >> 24)) & _MASK
        work[c] = (work[c] + work[d]) & _MASK
        work[b] ^= work[c]
        work[b] = ((work[b] << 7) | (work[b] >> 25)) & _MASK

    for _ in range(10):
        quarter(0, 4, 8, 12)
        quarter(1, 5, 9, 13)
        quarter(2, 6, 10, 14)
        quarter(3, 7, 11, 15)
        quarter(0, 5, 10, 15)
        quarter(1, 6, 11, 12)
        quarter(2, 7, 8, 13)
        quarter(3, 4, 9, 14)
    return struct.pack("<16I", *[(w + s) & _MASK for w, s in zip(work, state)])


def _chacha_xor(key, counter, nonce, data):
    out = bytearray()
    for offset in range(0, len(data), 64):
        block = _chacha_block(key, counter + offset // 64, nonce)
        chunk = data[offset:offset + 64]
        out += bytes(a ^ b for a, b in zip(chunk, block))
    return bytes(out)


def poly1305(key, message):
    """RFC 8439 section 2.5: the 16-byte tag of message under a 32-byte one-time key."""
    r = int.from_bytes(key[:16], "little") & 0x0FFFFFFC0FFFFFFC0FFFFFFC0FFFFFFF
    s = int.from_bytes(key[16:], "little")
    p = (1 << 130) - 5
    acc = 0
    for offset in range(0, len(message), 16):
        block = message[offset:offset + 16]
        acc = (acc + int.from_bytes(block + b"\x01", "little")) * r % p
    return ((acc + s) & ((1 << 128) - 1)).to_bytes(16, "little")


def _pad16(data):
    return b"\x00" * (-len(data) % 16)


def _mac_data(aad, ciphertext):
    return aad + _pad16(aad) + ciphertext + _pad16(ciphertext) + struct.pack("<QQ", len(aad), len(ciphertext))


def aead_encrypt(key, nonce, aad, plaintext):
    """RFC 8439 section 2.8: ciphertext followed by the 16-byte tag."""
    one_time = _chacha_block(key, 0, nonce)[:32]
    ciphertext = _chacha_xor(key, 1, nonce, plaintext)
    return ciphertext + poly1305(one_time, _mac_data(aad, ciphertext))


def aead_decrypt(key, nonce, aad, data):
    if len(data) < TAG_BYTES:
        raise NoiseError("message too short")
    ciphertext, tag = data[:-TAG_BYTES], data[-TAG_BYTES:]
    one_time = _chacha_block(key, 0, nonce)[:32]
    if not hmac.compare_digest(tag, poly1305(one_time, _mac_data(aad, ciphertext))):
        raise NoiseError("bad tag")
    return _chacha_xor(key, 1, nonce, ciphertext)


# ---------------------------------------------------------------- Noise

def _hash(data):
    return hashlib.blake2s(data).digest()


def _hmac(key, data):
    return hmac.new(key, data, hashlib.blake2s).digest()


def _hkdf2(chaining_key, material):
    temp = _hmac(chaining_key, material)
    first = _hmac(temp, b"\x01")
    return first, _hmac(temp, first + b"\x02")


def _nonce(counter):
    return b"\x00" * 4 + struct.pack("<Q", counter)


class CipherState:
    """One direction of the transport: a key and a message counter."""

    def __init__(self, key):
        self.key = key
        self.counter = 0

    def encrypt(self, plaintext, aad=b""):
        out = aead_encrypt(self.key, _nonce(self.counter), aad, plaintext)
        self.counter += 1
        return out

    def decrypt(self, data, aad=b""):
        out = aead_decrypt(self.key, _nonce(self.counter), aad, data)
        self.counter += 1
        return out


class _Handshake:
    def __init__(self, static_secret, prologue, ephemeral_secret):
        self.s_secret = static_secret
        self.s_public = public_key(static_secret)
        self.e_secret = ephemeral_secret  # fixed only by tests that replay vectors
        self.h = _hash(PROTOCOL)  # the name is longer than 32 bytes
        self.ck = self.h
        self.k = None
        self.n = 0
        self.rs = None
        self.re = None
        self.send = self.recv = None
        self._mix_hash(prologue)

    def _mix_hash(self, data):
        self.h = _hash(self.h + data)

    def _mix_key(self, material):
        self.ck, self.k = _hkdf2(self.ck, material)
        self.n = 0

    def _encrypt_and_hash(self, plaintext):
        if self.k is None:
            ciphertext = plaintext
        else:
            ciphertext = aead_encrypt(self.k, _nonce(self.n), self.h, plaintext)
            self.n += 1
        self._mix_hash(ciphertext)
        return ciphertext

    def _decrypt_and_hash(self, ciphertext):
        if self.k is None:
            plaintext = ciphertext
        else:
            plaintext = aead_decrypt(self.k, _nonce(self.n), self.h, ciphertext)
            self.n += 1
        self._mix_hash(ciphertext)
        return plaintext

    def _new_ephemeral(self):
        if self.e_secret is None:
            self.e_secret = os.urandom(32)
        return public_key(self.e_secret)

    def _split(self, initiator):
        first, second = _hkdf2(self.ck, b"")
        mine, theirs = (first, second) if initiator else (second, first)
        self.send, self.recv = CipherState(mine), CipherState(theirs)

    @property
    def handshake_hash(self):
        return self.h


class Initiator(_Handshake):
    """-> e, es, s, ss      <- e, ee, se.  Needs the responder's static public key up front."""

    def __init__(self, static_secret, remote_static, prologue=b"", ephemeral_secret=None):
        super().__init__(static_secret, prologue, ephemeral_secret)
        self.rs = remote_static
        self._mix_hash(remote_static)

    def write_message1(self, payload=b""):
        e_public = self._new_ephemeral()
        self._mix_hash(e_public)
        out = e_public
        self._mix_key(_dh(self.e_secret, self.rs))
        out += self._encrypt_and_hash(self.s_public)
        self._mix_key(_dh(self.s_secret, self.rs))
        return out + self._encrypt_and_hash(payload)

    def read_message2(self, message):
        if len(message) < KEY_BYTES + TAG_BYTES:
            raise NoiseError("message too short")
        self.re = message[:KEY_BYTES]
        self._mix_hash(self.re)
        self._mix_key(_dh(self.e_secret, self.re))
        self._mix_key(_dh(self.s_secret, self.re))
        payload = self._decrypt_and_hash(message[KEY_BYTES:])
        self._split(True)
        return payload


class Responder(_Handshake):
    """The other side. After read_message1, `remote_static` is the sender's key, proven by the handshake.
    Check it before write_message2: an unverified sender gets no answer."""

    def __init__(self, static_secret, prologue=b"", ephemeral_secret=None):
        super().__init__(static_secret, prologue, ephemeral_secret)
        self._mix_hash(self.s_public)

    @property
    def remote_static(self):
        return self.rs

    def read_message1(self, message):
        if len(message) < KEY_BYTES + KEY_BYTES + TAG_BYTES + TAG_BYTES:
            raise NoiseError("message too short")
        self.re = message[:KEY_BYTES]
        self._mix_hash(self.re)
        self._mix_key(_dh(self.s_secret, self.re))
        end = KEY_BYTES + KEY_BYTES + TAG_BYTES
        self.rs = self._decrypt_and_hash(message[KEY_BYTES:end])
        self._mix_key(_dh(self.s_secret, self.rs))
        return self._decrypt_and_hash(message[end:])

    def write_message2(self, payload=b""):
        e_public = self._new_ephemeral()
        self._mix_hash(e_public)
        self._mix_key(_dh(self.e_secret, self.re))
        self._mix_key(_dh(self.e_secret, self.rs))
        out = e_public + self._encrypt_and_hash(payload)
        self._split(False)
        return out
