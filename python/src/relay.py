"""Wire format v1 for the relay (docs/supabase-relay.md): the channel name and the one-line envelope.

Only the bytes are handled here. `ct` is whatever Noise produced; this module never encrypts.
Same rules as rust/crates/talk-core/src/relay.rs.
"""

import base64
import hashlib
import json

CHANNEL_PREFIX = b"herdr-talk-relay-v1"
KEY_LEN = 32


class Reject(Exception):
    """An envelope is dropped. `reason` is one of: format, version, party, channel."""

    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def channel_name(a, b):
    """hex(SHA-256(prefix || min || max)). The order of the two keys does not matter."""
    low, high = sorted((bytes(a), bytes(b)))
    return hashlib.sha256(CHANNEL_PREFIX + low + high).hexdigest()


def seal(from_key, to_key, ct):
    """One envelope as a JSON line, keys in the order v, channel, from, to, ct. None when a key is not 32 bytes."""
    if len(from_key) != KEY_LEN or len(to_key) != KEY_LEN:
        return None
    return json.dumps(
        {
            "v": 1,
            "channel": channel_name(from_key, to_key),
            "from": bytes(from_key).hex(),
            "to": bytes(to_key).hex(),
            "ct": base64.b64encode(bytes(ct)).decode("ascii"),
        },
        separators=(",", ":"),
    )


def _key_field(value):
    if not isinstance(value, str) or len(value) != KEY_LEN * 2:
        raise Reject("format")
    try:
        key = bytes.fromhex(value)
    except ValueError:
        raise Reject("format")
    if key.hex() != value:  # uppercase hex is not the v1 form
        raise Reject("format")
    return key


def open_envelope(line, pair):
    """Check one line against the two keys this side expects on the channel (`pair`).
    Returns {"from", "to", "ct"} (bytes). Raises Reject; nothing is trusted before the checks pass."""
    try:
        value = json.loads(line)
    except (ValueError, TypeError):
        raise Reject("format")
    if not isinstance(value, dict):
        raise Reject("format")
    version = value.get("v")
    if isinstance(version, bool) or not isinstance(version, int):
        raise Reject("format")
    if version != 1:
        raise Reject("version")
    from_key = _key_field(value.get("from"))
    to_key = _key_field(value.get("to"))
    channel = value.get("channel")
    ct_text = value.get("ct")
    if not isinstance(channel, str) or not isinstance(ct_text, str):
        raise Reject("format")
    try:
        ct = base64.b64decode(ct_text, validate=True)
    except ValueError:
        raise Reject("format")
    if channel != channel_name(pair[0], pair[1]):
        raise Reject("channel")
    if from_key not in (bytes(pair[0]), bytes(pair[1])) or to_key not in (bytes(pair[0]), bytes(pair[1])):
        raise Reject("party")
    return {"from": from_key, "to": to_key, "ct": ct}
