"""Supabase REST side of the relay: settings, the signed row write and delete, the room requests, and checks of the answers.

Same rules as rust/crates/talk-core/src/relay_rest.rs. Standard library only (urllib for sending).
The key is the publishable key; the secret key is never read or stored here.

Row writes are signed (format v3). The signed text carries the short id (8 characters, [A-Z0-9]) of this PC. The short id is unique on the server; a write that takes an id owned by another key gets 409.
"""

import json
import re
import secrets
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request

import ed25519

URL_VARIABLE = "HERDR_TALK_RELAY_URL"
KEY_VARIABLE = "HERDR_TALK_RELAY_KEY"
PUT_TAG = "herdr-talk-put-v3"
DELETE_TAG = "herdr-talk-del-v1"
TIMEOUT_SECONDS = 5

_PK_PATTERN = re.compile(r"^[0-9a-f]{64}$")
_NONCE_PATTERN = re.compile(r"^[0-9a-f]{32}$")
_SHORT_PATTERN = re.compile(r"^[A-Z0-9]{8}$")
_ROOM_PATTERN = re.compile(r"^[A-Z0-9]{8}$")


class Config:
    """URL and publishable key. repr() hides the key, so logs never show it."""

    def __init__(self, url, key):
        self.url = url
        self.key = key

    def __repr__(self):
        return f"Config(url={self.url!r}, key='<hidden>')"


def config(url, key):
    """Settings from two values. Either one missing or empty means the relay is off: None.
    A URL that is not https (except loopback, for tests) raises ValueError, so the key never goes over plain HTTP."""
    url = (url or "").strip()
    key = (key or "").strip()
    if not url or not key:
        return None
    url = url.rstrip("/")
    loopback = url.startswith("http://127.0.0.1") or url.startswith("http://localhost")
    if not url.startswith("https://") and not loopback:
        raise ValueError(f"{URL_VARIABLE} must start with https://")
    return Config(url, key)



def valid_pk(text):
    return isinstance(text, str) and _PK_PATTERN.match(text) is not None


def valid_short(text):
    """A short id: exactly 8 characters, uppercase letters and digits. Lowercase input is made uppercase by the caller."""
    return isinstance(text, str) and _SHORT_PATTERN.match(text) is not None


def valid_room_code(text):
    """A room code: exactly 8 characters, uppercase letters and digits (the same rule as talk-core)."""
    return isinstance(text, str) and _ROOM_PATTERN.match(text) is not None


def valid_name(text):
    """1 to 32 characters, no control characters (category Cc, the same rule the table enforces)."""
    if not isinstance(text, str) or not 1 <= len(text) <= 32:
        return False
    return not any(unicodedata.category(ch) == "Cc" for ch in text)


def _common_headers(config):
    return {"apikey": config.key, "Authorization": f"Bearer {config.key}"}


def put_message(pk, noise_pk, short_id, name, ts, nonce):
    """The bytes that are signed (v3): PUT_TAG, pk, noise_pk, short_id, name, ts, nonce, one per line, no newline at
    the end, UTF-8."""
    return "\n".join([PUT_TAG, pk, noise_pk, short_id, name, str(ts), nonce]).encode("utf-8")


def put_request(config, seed, noise_pk, short_id, name, ts=None, nonce=None):
    """Signed write of this PC's row through the put-peer function. `seed` is the 32-byte Ed25519 private key (pk is its
    public key); `noise_pk` is the X25519 key of the Noise identity. ts defaults to now, nonce to 16 random bytes.
    The body keeps the fixed order pk, noise_pk, short_id, name, ts, nonce, sig. Raises ValueError for bad input."""
    pk = ed25519.public_key(seed).hex()
    if not _PK_PATTERN.match(noise_pk or ""):
        raise ValueError("noise_pk must be 64 lowercase hex characters")
    if not valid_short(short_id):
        raise ValueError("short_id must be 8 uppercase letters or digits")
    if not valid_name(name):
        raise ValueError("name must be 1 to 32 characters without control characters")
    ts = int(time.time()) if ts is None else ts
    nonce = secrets.token_hex(16) if nonce is None else nonce
    if not isinstance(ts, int) or isinstance(ts, bool) or not _NONCE_PATTERN.match(nonce):
        raise ValueError("ts must be an integer and nonce 32 lowercase hex characters")
    sig = ed25519.sign(seed, put_message(pk, noise_pk, short_id, name, ts, nonce)).hex()
    headers = _common_headers(config)
    headers["Content-Type"] = "application/json"
    body = {"pk": pk, "noise_pk": noise_pk, "short_id": short_id, "name": name, "ts": ts, "nonce": nonce, "sig": sig}
    return {
        "method": "POST",
        "url": f"{config.url}/functions/v1/put-peer",
        "headers": headers,
        "body": json.dumps(body, separators=(",", ":"), ensure_ascii=False),
    }


def put_succeeded(status):
    return 200 <= status < 300


def _send(request, timeout=TIMEOUT_SECONDS):
    """Send one request. Returns (status, body). A network error raises OSError."""
    data = request["body"].encode("utf-8") if request["body"] is not None else None
    req = urllib.request.Request(request["url"], data=data, headers=request["headers"], method=request["method"])
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return response.status, response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as error:
        return error.code, ""


def put_status(config, seed, noise_pk, short_id, name):
    """The HTTP status of the signed write (2xx accepted, 409 short id taken by another key), or None when the request
    could not be made or sent."""
    try:
        status, _ = _send(put_request(config, seed, noise_pk, short_id, name))
        return status
    except (OSError, ValueError):
        return None




def delete_message(pk, short_id, ts, nonce):
    """The bytes that are signed for a row delete: DELETE_TAG, pk, short_id, ts, nonce, one per line, no newline at the end."""
    return chr(10).join([DELETE_TAG, pk, short_id, str(ts), nonce]).encode("utf-8")


def delete_request(config, seed, short_id, ts=None, nonce=None):
    """Signed delete of this node's own row through the delete-peer function. Raises ValueError for bad input."""
    pk = ed25519.public_key(seed).hex()
    if not valid_short(short_id):
        raise ValueError("short_id must be 8 uppercase letters or digits")
    ts = int(time.time()) if ts is None else ts
    nonce = secrets.token_hex(16) if nonce is None else nonce
    if not isinstance(ts, int) or isinstance(ts, bool) or not _NONCE_PATTERN.match(nonce):
        raise ValueError("ts must be an integer and nonce 32 lowercase hex characters")
    sig = ed25519.sign(seed, delete_message(pk, short_id, ts, nonce)).hex()
    headers = _common_headers(config)
    headers["Content-Type"] = "application/json"
    body = {"pk": pk, "short_id": short_id, "ts": ts, "nonce": nonce, "sig": sig}
    return {
        "method": "POST",
        "url": f"{config.url}/functions/v1/delete-peer",
        "headers": headers,
        "body": json.dumps(body, separators=(",", ":"), ensure_ascii=False),
    }


def delete_peer(config, seed, short_id, timeout=TIMEOUT_SECONDS):
    """True when the relay accepted the signed delete (or the row was already gone)."""
    try:
        status, _ = _send(delete_request(config, seed, short_id), timeout)
        return put_succeeded(status)
    except (OSError, ValueError):
        return False


def read_room_code(status, body):
    """The room code of a create-room answer. Anything but 200 with a valid code raises ValueError."""
    if status != 200:
        raise ValueError(f"status {status}: {body}")
    try:
        value = json.loads(body)
    except (ValueError, TypeError):
        raise ValueError("answer is not JSON")
    code = value.get("room_code") if isinstance(value, dict) else None
    if not valid_room_code(code):
        raise ValueError("answer has no valid room_code")
    return code


def read_join_room(status, body):
    """{room_code, owner_pk, members} from a join-room answer. Anything malformed raises ValueError."""
    if status == 410:
        raise ValueError("방 기한 만료")  # join-room: the room is past its 24 hours
    if status != 200:
        raise ValueError(f"status {status}: {body}")
    try:
        value = json.loads(body)
    except (ValueError, TypeError):
        raise ValueError("answer is not JSON")
    if not isinstance(value, dict) or not valid_room_code(value.get("room_code")):
        raise ValueError("answer has no valid room_code")
    if not valid_pk(value.get("owner_pk")):
        raise ValueError("answer has no valid owner_pk")
    members = value.get("members")
    if not isinstance(members, list) or not all(valid_pk(pk) for pk in members):
        raise ValueError("answer has no valid members list")
    return {"room_code": value["room_code"], "owner_pk": value["owner_pk"], "members": members}


ROOM_TAG = "herdr-talk-room-v1"


def room_message(action, room_code, peer_pk, name, ts, nonce):
    """The bytes signed for a room request (v1): ROOM_TAG, action ("create" or "join"), the room code (empty for create),
    peer pk, display name, ts, nonce, one per line, no newline at the end, UTF-8. Same as the Rust and mobile sides."""
    return "\n".join([ROOM_TAG, action, room_code, peer_pk, name, str(ts), nonce]).encode("utf-8")


def _signed_room_body(seed, action, room_code, name, ts=None, nonce=None):
    """The signed fields of a create or join request. `seed` is the 32-byte Ed25519 key (pk is its public key)."""
    if not valid_name(name):
        raise ValueError("name must be 1 to 32 characters without control characters")
    pk = ed25519.public_key(seed).hex()
    ts = int(time.time()) if ts is None else ts
    nonce = secrets.token_hex(16) if nonce is None else nonce
    if not isinstance(ts, int) or isinstance(ts, bool) or not _NONCE_PATTERN.match(nonce):
        raise ValueError("ts must be an integer and nonce 32 lowercase hex characters")
    sig = ed25519.sign(seed, room_message(action, room_code, pk, name, ts, nonce)).hex()
    return {"peer_pk": pk, "display_name": name, "ts": ts, "nonce": nonce, "sig": sig}


def create_room_request(config, seed, name, ts=None, nonce=None):
    """Ask the create-room function for a new room, signed with this PC's key. Raises ValueError for bad input."""
    headers = _common_headers(config)
    headers["Content-Type"] = "application/json"
    body = _signed_room_body(seed, "create", "", name, ts, nonce)
    return {"method": "POST", "url": f"{config.url}/functions/v1/create-room", "headers": headers,
            "body": json.dumps(body, ensure_ascii=False)}


def join_room_request(config, seed, room_code, name, ts=None, nonce=None):
    """Ask the join-room function to add this PC to a room, signed with this PC's key. Raises ValueError for bad input."""
    if not valid_room_code(room_code):
        raise ValueError("room code must be 8 uppercase letters or digits")
    headers = _common_headers(config)
    headers["Content-Type"] = "application/json"
    body = {"room_code": room_code, **_signed_room_body(seed, "join", room_code, name, ts, nonce)}
    return {"method": "POST", "url": f"{config.url}/functions/v1/join-room", "headers": headers,
            "body": json.dumps(body, ensure_ascii=False)}


def create_room(config, seed, name, timeout=TIMEOUT_SECONDS):
    """The code of the new room this PC created. Network errors raise OSError, bad answers ValueError."""
    return read_room_code(*_send(create_room_request(config, seed, name), timeout))


def join_room(config, seed, room_code, name, timeout=TIMEOUT_SECONDS):
    """Join a room by code; returns the room info from read_join_room."""
    return read_join_room(*_send(join_room_request(config, seed, room_code, name), timeout))
