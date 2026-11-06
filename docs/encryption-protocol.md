# Encryption protocol

Both editions (Rust `net.rs`, Python `noise.py` + `talk.py`) speak this. It was read from the Rust edition and the Python edition was checked against it.

## Keys and ids

- Key pair: X25519, 32 bytes each. Stored in `identity.json` in the state folder as `{"public": "<64 hex>", "secret": "<64 hex>"}`. The Rust and Python editions read each other's file, so they can share one state folder (same key, so they trust each other).
- `peer_id(key)` = SHA-256 of the 32-byte public key, 64 lowercase hex digits. An old keyless Python id is 32 hex digits, so a 64-digit id without a key is a key being stripped and is refused.
- Code to compare, `fingerprint(own, peer)`: SHA-256 over the two public keys, the smaller (byte order) first, take the first 16 bytes as 32 hex digits, write them as eight groups of four joined by `-` (`1a2b-3c4d-...`). `/trust` ignores case, spaces and dashes.
- `trusted.json`: `[{"name": str, "key": "<64 hex>", "ip": str}]`.

## Discovery (UDP 47850, broadcast every 2 s)

One JSON object: `{"app": "herdr-talk", "id": <peer_id>, "name": str, "port": <TCP port>, "key": "<64 hex>"}`. A receiver drops it when `key` does not hash to `id`, when `id` has 64 digits and no `key`, or when `key` is its own key and the sender address is not an address of this PC (several chats on one PC share the key and differ by port). A key it has not verified is shown with its code, or with a "key changed" warning when a trusted key from that address is no longer announcing.

## Message (one TCP connection, one message)

Noise `Noise_IK_25519_ChaChaPoly_BLAKE2s`, empty prologue, the receiver's static key known from its announcement.

1. Initiator sends frame 1 (`e, es, s, ss`, empty payload, 96 bytes). A frame is a 2-byte big-endian length, then the bytes.
2. The responder reads the sender key from frame 1 and answers only when that key is trusted (or its own). Otherwise it closes without a reply.
3. Responder sends frame 2 (`e, ee, se`, empty payload, 48 bytes).
4. Initiator sends frame 3: the transport message, ChaCha20-Poly1305 under the first key, nonce 0. It holds the JSON `{"app", "id", "from", "text"}`, at most 65519 bytes. Then it closes its write side.

The receiver checks `id == peer_id(handshake key)` and shows the sender name the user verified.

A plain message (only with `"plaintext": true`, for an old keyless Python) is the same JSON with no framing. Its first byte is `{`, which is how a receiver tells it from a handshake (the first byte of a frame 1 length is 0). A peer with a key is never sent plain text, and a plain message with a key-derived id is dropped. A plain message is shown as `name (plain text): ...` on every row.

## Python edition

`noise.py` is pure standard library: X25519 (RFC 7748), ChaCha20-Poly1305 (RFC 8439), BLAKE2s and HMAC from `hashlib`/`hmac`, and the Noise IK handshake. Tests: RFC 7748 and RFC 8439 vectors, and the Cacophony vector for this exact pattern (`python/tests/noise_ik_vector.json`). The code is not constant time (big-integer math), so it protects against network attackers, not against another process on the same PC timing it.
