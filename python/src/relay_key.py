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
SHORT_FILE = "short-id.txt"
SHORT_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
_HEX = re.compile(r"^[0-9a-f]{64}$")
_SHORT = re.compile(r"^[A-Z0-9]{8}$")


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


def new_short_id():
    """A random short id: 8 characters from [A-Z0-9]."""
    return "".join(secrets.choice(SHORT_ALPHABET) for _ in range(8))


def _write_new(path, text):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(text + chr(10))


def load_or_create_short_id(state):
    """This PC's short id. Made once and kept; a damaged file is an error, not a reason to make a new one."""
    path = os.path.join(state, SHORT_FILE)
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read().strip()
    except FileNotFoundError:
        text = None
    except OSError as error:
        raise SigningKeyError(f"cannot read {SHORT_FILE}: {error}")
    if text is not None:
        # A file from an older version may hold lowercase: it is read as uppercase (the input rule).
        text = text.upper()
        if not _SHORT.match(text):
            raise SigningKeyError(f"{SHORT_FILE} is damaged; it was not replaced")
        return text
    os.makedirs(state, exist_ok=True)
    short = new_short_id()
    try:
        _write_new(path, short)
    except FileExistsError:
        return load_or_create_short_id(state)
    return short


def replace_short_id(state, short):
    """Keep a new short id after the server said the old one is taken (the only time it changes)."""
    if not _SHORT.match(short):
        raise SigningKeyError("short id must be 8 uppercase letters or digits")
    path = os.path.join(state, SHORT_FILE)
    temp = path + ".tmp"
    with open(temp, "w", encoding="utf-8") as f:
        f.write(short + chr(10))
    os.replace(temp, path)
