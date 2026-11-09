"""Files that outlive one chat: this PC's identity key and the peers the user has verified.
Same files and format as the Rust edition (store.rs), so both editions can share one state folder.
  identity.json  {"public": hex, "secret": hex}                 readable by the user only
  trusted.json   [{"name": str, "key": hex, "ip": str}, ...]
"""

import hashlib
import json
import os
import re

import noise

IDENTITY_FILE = "identity.json"
TRUSTED_FILE = "trusted.json"
_HEX = re.compile(r"^(?:[0-9a-fA-F]{2})*$")


def to_hex(data):
    return data.hex()


def from_hex(text):
    """bytes, or None when text is not an even number of hex digits."""
    return bytes.fromhex(text) if isinstance(text, str) and _HEX.match(text) else None


def peer_id(key):
    """A peer's id is the SHA-256 of its public key, in hex. Nobody can claim another peer's id without its key."""
    return hashlib.sha256(key).hexdigest()


def key_label(key):
    """The first 8 hex digits of the peer id: enough to tell peers apart on screen, never the full id."""
    return peer_id(key)[:8]


def fingerprint(own, peer):
    """Code both users compare: 128 bits of SHA-256 over the two public keys, sorted, in groups of four hex digits."""
    first, second = sorted((own, peer))
    digits = hashlib.sha256(first + second).hexdigest()[:32]
    return "-".join(digits[i:i + 4] for i in range(0, 32, 4))


def normalize_code(text):
    """What the user types for a code: no dashes, spaces or capitals."""
    return "".join(ch for ch in text if not ch.isspace() and ch != "-").lower()


def _damaged():
    return ValueError("herdr-talk state file is damaged")


def _read_json(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return None
    except ValueError as error:
        raise _damaged() from error


def _write_private(path, text, exclusive=False):
    """Owner read/write only on Unix. On Windows the file keeps the default permissions of the user's profile folder."""
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | (os.O_EXCL if exclusive else os.O_TRUNC)
    with os.fdopen(os.open(path, flags, 0o600), "w", encoding="utf-8") as out:
        out.write(text)


def read_identity(folder):
    """(public, secret), or None when there is none yet. A damaged file is an error, never a reason to make a new key:
    a new key would make every peer distrust this PC."""
    value = _read_json(os.path.join(folder, IDENTITY_FILE))
    if value is None:
        return None
    try:
        public, secret = from_hex(value["public"]), from_hex(value["secret"])
    except (KeyError, TypeError) as error:
        raise _damaged() from error
    if public is None or secret is None or len(public) != 32 or len(secret) != 32 or noise.public_key(secret) != public:
        raise _damaged()
    return public, secret


def load_or_create_identity(folder):
    """The stored identity, made on first use. Two chats starting together in one folder end up with the same key."""
    found = read_identity(folder)
    if found:
        return found
    public, secret = noise.generate_keypair()
    text = json.dumps({"public": to_hex(public), "secret": to_hex(secret)})
    try:
        _write_private(os.path.join(folder, IDENTITY_FILE), text, exclusive=True)
    except FileExistsError:
        return read_identity(folder)  # another chat made it first
    return public, secret


def read_trusted(folder):
    """[{"name", "key" (bytes), "ip"}]"""
    value = _read_json(os.path.join(folder, TRUSTED_FILE))
    if value is None:
        return []
    if not isinstance(value, list):
        raise _damaged()
    trusted = []
    for entry in value:
        key = from_hex(entry.get("key")) if isinstance(entry, dict) else None
        if key is None:
            raise _damaged()
        name, ip = entry.get("name", ""), entry.get("ip", "")
        trusted.append({"name": name if isinstance(name, str) else "", "key": key, "ip": ip if isinstance(ip, str) else ""})
    return trusted


def write_trusted(folder, trusted):
    rows = [{"name": peer["name"], "key": to_hex(peer["key"]), "ip": peer["ip"]} for peer in trusted]
    _write_private(os.path.join(folder, TRUSTED_FILE), json.dumps(rows, ensure_ascii=False))


def plaintext_allowed():
    """Plain messages to peers without a key are off unless config.json sets "plaintext": true."""
    folder = os.environ.get("HERDR_PLUGIN_CONFIG_DIR")
    if not folder:
        return False
    try:
        with open(os.path.join(folder, "config.json"), encoding="utf-8-sig") as f:
            return json.load(f).get("plaintext") is True
    except (OSError, ValueError, AttributeError):
        return False
