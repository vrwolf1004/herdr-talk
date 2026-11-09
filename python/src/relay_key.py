"""The Ed25519 signing key for relay row writes. Kept apart from the Noise key (identity.json) on purpose.

Stored as 64 hex characters in relay-signing.key in the state folder. The file is created new, never overwritten:
a damaged file is an error, not a reason to make a new key (the peer id would change). On Unix the file is 0600;
on Windows the profile folder's ACL applies (same as identity.json).
"""

import os
import re
import secrets

import store

KEY_FILE = "relay-signing.key"
_HEX = re.compile(r"^[0-9a-f]{64}$")


class SigningKeyError(ValueError):
    pass


def load_or_create(state):
    """The 32-byte seed. Makes a new key only when there is no file yet."""
    path = os.path.join(state, KEY_FILE)
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read().strip()
    except FileNotFoundError:
        return _create(state, path)
    except OSError as error:
        raise SigningKeyError(f"cannot read {KEY_FILE}: {error}")
    if not _HEX.match(text):
        raise SigningKeyError(f"{KEY_FILE} is damaged; it was not replaced")
    return bytes.fromhex(text)


def _create(state, path):
    os.makedirs(state, exist_ok=True)
    seed = secrets.token_bytes(32)
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        return load_or_create(state)  # another process made it first: use that one
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(seed.hex() + "\n")
    return seed


def noise_public_hex(state):
    """The X25519 public key of this PC's Noise identity (identity.json), as 64 lowercase hex characters.
    Read only: a missing identity is an error here (start a chat first), never a new key."""
    found = store.read_identity(state)
    if found is None:
        raise SigningKeyError("no identity yet: start a chat first")
    public, _secret = found
    return public.hex()
