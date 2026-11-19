"""Supabase REST side of the relay: settings, the signed row write and delete, the room requests, and checks of the answers.

Same rules as rust/crates/talk-core/src/relay_rest.rs. Standard library only (urllib for sending).
The key is the publishable key; the secret key is never read or stored here.

Row writes are signed (format v3). The signed text carries the short id (8 characters, [A-Z0-9]) of this PC. The short id is unique on the server; a write that takes an id owned by another key gets 409.
"""

import hashlib
import json
import os
import re
import secrets
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request

from core import ed25519

# The project's public URL. A setup code gets the publishable key from it, so the app needs the URL before any code.
# HERDR_TALK_RELAY_URL still wins when set (tests and other projects).
DEFAULT_URL = "https://coekrspnpcjzcbydarco.supabase.co"
URL_VARIABLE = "HERDR_TALK_RELAY_URL"
KEY_VARIABLE = "HERDR_TALK_RELAY_KEY"
PUT_TAG = "herdr-talk-put-v3"
DELETE_TAG = "herdr-talk-del-v1"
POST_TAG = "herdr-talk-msg-v1"
FETCH_TAG = "herdr-talk-fetch-v1"
MAX_DELIVERIES = 8
MAX_CIPHERTEXT_HEX = 32768
TIMEOUT_SECONDS = 5
TRUST_WRONG = "trust 코드가 맞지 않습니다"  # join-room answers 403 when the trust code is not the room's

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
    """1 to 32 characters, all printable: no control, format (bidi, zero-width), private-use or space-separator characters
    except the plain space. The same rule as talk-core's is_printable, so both editions accept the same names.
    shortcut: unassigned and noncharacter code points are not refused here (Rust refuses some of them); add them if it matters."""
    if not isinstance(text, str) or not 1 <= len(text) <= 32:
        return False
    return all(ch == " " or unicodedata.category(ch) not in NAME_BLOCKED for ch in text)


NAME_BLOCKED = {"Cc", "Cf", "Co", "Cs", "Zl", "Zp", "Zs"}


def _common_headers(config):
    return {"apikey": config.key, "Authorization": f"Bearer {config.key}"}


def _valid_ciphertext(text):
    return isinstance(text, str) and 0 < len(text) <= MAX_CIPHERTEXT_HEX and len(text) % 2 == 0 and re.fullmatch(r"[0-9a-f]+", text) is not None


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
    if status == 403:
        raise ValueError(TRUST_WRONG)
    if status == 409:
        raise ValueError("방 인원이 가득 찼습니다 (10명)")  # join-room: the room has 10 members
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


def setup_url():
    """The URL the setup code is redeemed at: HERDR_TALK_RELAY_URL when set, else DEFAULT_URL."""
    return os.environ.get(URL_VARIABLE, "").strip() or DEFAULT_URL


def normalize_setup_code(text):
    """What the user typed as a setup code: spaces and dashes dropped, letters uppercased. None when the rest is not 8
    characters of A-Z or 0-9 (the same rule as the redeem-setup-code function). Non-ASCII input is refused: some
    letters uppercase to several characters."""
    if not isinstance(text, str) or not text.isascii():
        return None
    code = "".join(ch for ch in text if not ch.isspace() and ch != "-").upper()
    return code if re.fullmatch(r"[0-9A-Z]{8}", code) else None


def read_setup_answer(status, body):
    """(url, key) from a redeem-setup-code answer. Anything else raises ValueError with the reason."""
    if status == 404:
        raise ValueError("the setup code is wrong, used or expired")
    if status == 429:
        raise ValueError("too many wrong codes; try again later")
    if status != 200:
        raise ValueError(f"status {status}")
    try:
        value = json.loads(body)
    except (ValueError, TypeError):
        raise ValueError("answer is not JSON") from None
    if not isinstance(value, dict) or not isinstance(value.get("url"), str) or not isinstance(value.get("key"), str):
        raise ValueError("answer has no url and key")
    checked = config(value["url"], value["key"])  # https only, as for the saved settings
    if checked is None:
        raise ValueError("answer has no url and key")
    return checked.url, checked.key


def redeem_setup_code(code, url=None, timeout=TIMEOUT_SECONDS):
    """The (url, key) of a one-time setup code, redeemed at `url` (default setup_url()). No apikey header: the code is the
    only credential. Raises ValueError for a bad code or answer, OSError for a network error."""
    normalized = normalize_setup_code(code)
    if normalized is None:
        raise ValueError("setup code must be 8 letters or digits")
    base = (url or setup_url()).rstrip("/")
    request = {"method": "POST", "url": f"{base}/functions/v1/redeem-setup-code",
               "headers": {"Content-Type": "application/json"}, "body": json.dumps({"code": normalized})}
    return read_setup_answer(*_send(request, timeout))


def issue_setup_code_request(passphrase, max_uses=20, days=7, url=None):
    """The request that asks the operator function for a new setup code. The passphrase is checked on the server."""
    base = (url or setup_url()).rstrip("/")
    body = json.dumps({"passphrase": passphrase, "max_uses": max_uses, "days": days}, ensure_ascii=False)
    return {"method": "POST", "url": f"{base}/functions/v1/issue-setup-code", "headers": {"Content-Type": "application/json"}, "body": body}


def read_issue_answer(status, body):
    """The new code from an issue-setup-code answer. Anything else raises ValueError with the reason."""
    if status == 200:
        try:
            value = json.loads(body)
        except (ValueError, TypeError):
            raise ValueError("answer is not JSON") from None
        if not isinstance(value, dict) or not isinstance(value.get("code"), str):
            raise ValueError("answer has no code")
        return value["code"]
    if status == 401:
        raise ValueError("the operator passphrase is wrong")
    if status == 429:
        raise ValueError("too many wrong tries; try later")
    if status == 503:
        raise ValueError("the operator secrets are not set on the server")
    raise ValueError(f"status {status}")


def issue_setup_code(passphrase, max_uses=20, days=7, url=None):
    """A new setup code from the server (the operator passphrase is checked there). Raises ValueError or OSError."""
    return read_issue_answer(*_send(issue_setup_code_request(passphrase, max_uses, days, url)))


def valid_trust(text):
    """A trust code: exactly 8 ASCII digits (the owner sets it at create, each participant types it at join)."""
    return isinstance(text, str) and len(text) == 8 and text.isascii() and text.isdigit()


def create_room_request(config, seed, name, trust, ts=None, nonce=None):
    """Ask the create-room function for a new room, signed with this PC's key. Raises ValueError for bad input.
    The trust code is sent with the request, not signed: the function stores only its salted hash."""
    if not valid_trust(trust):
        raise ValueError("trust code must be 8 digits")
    headers = _common_headers(config)
    headers["Content-Type"] = "application/json"
    body = {**_signed_room_body(seed, "create", "", name, ts, nonce), "trust": trust}
    return {"method": "POST", "url": f"{config.url}/functions/v1/create-room", "headers": headers,
            "body": json.dumps(body, ensure_ascii=False)}


def join_room_request(config, seed, room_code, name, trust, ts=None, nonce=None):
    """Ask the join-room function to add this PC to a room, signed with this PC's key. Raises ValueError for bad input."""
    if not valid_room_code(room_code):
        raise ValueError("room code must be 8 uppercase letters or digits")
    if not valid_trust(trust):
        raise ValueError("trust code must be 8 digits")
    headers = _common_headers(config)
    headers["Content-Type"] = "application/json"
    body = {"room_code": room_code, **_signed_room_body(seed, "join", room_code, name, ts, nonce), "trust": trust}
    return {"method": "POST", "url": f"{config.url}/functions/v1/join-room", "headers": headers,
            "body": json.dumps(body, ensure_ascii=False)}


def create_room(config, seed, name, trust, timeout=TIMEOUT_SECONDS):
    """The code of the new room this PC created. Network errors raise OSError, bad answers ValueError."""
    return read_room_code(*_send(create_room_request(config, seed, name, trust), timeout))


def join_room(config, seed, room_code, name, trust, timeout=TIMEOUT_SECONDS):
    """Join a room by code and trust code; returns the room info from read_join_room."""
    return read_join_room(*_send(join_room_request(config, seed, room_code, name, trust), timeout))


# Room messages (Rust relay_rest.rs post_message_body, fetch_message_body, read_fetch_response, members_noise_keys).
def deliveries_digest(deliveries):
    """SHA-256 (hex) over the deliveries in the order given: one "recipient ciphertext" line each."""
    text = chr(10).join(f"{row['recipient_pk']} {row['ciphertext']}" for row in deliveries)
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def post_message_body(seed, room_code, message_id, ts, nonce, deliveries):
    """The JSON body of a post-message request. Recipients must be unique and ascending (the signed digest depends on
    it). Raises ValueError for bad input before any signature is made."""
    if not valid_room_code(room_code) or not re.fullmatch(r"[0-9a-f]{32}", message_id or "") or not _NONCE_PATTERN.match(nonce or ""):
        raise ValueError("room code, message id and nonce must have their fixed shapes")
    if not 1 <= len(deliveries) <= MAX_DELIVERIES:
        raise ValueError("a message goes to 1 to 8 recipients")
    for index, row in enumerate(deliveries):
        if not valid_pk(row["recipient_pk"]) or not _valid_ciphertext(row["ciphertext"]):
            raise ValueError("recipient key or ciphertext is not valid")
        if index > 0 and row["recipient_pk"] <= deliveries[index - 1]["recipient_pk"]:
            raise ValueError("recipients must be unique and in ascending order")
    sender_pk = ed25519.public_key(seed).hex()
    lines = [POST_TAG, "post", room_code, sender_pk, message_id, str(ts), nonce, deliveries_digest(deliveries)]
    sig = ed25519.sign(seed, chr(10).join(lines).encode("utf-8")).hex()
    return json.dumps({"room_code": room_code, "sender_pk": sender_pk, "message_id": message_id, "ts": ts, "nonce": nonce,
                       "deliveries": deliveries, "sig": sig}, ensure_ascii=False)


def fetch_message_body(seed, room_code, since, ts, nonce):
    """The JSON body of a fetch-messages request: this recipient's inbox for the room, after `since` (a filter only)."""
    if not valid_room_code(room_code) or not _NONCE_PATTERN.match(nonce or ""):
        raise ValueError("room code and nonce must have their fixed shapes")
    recipient_pk = ed25519.public_key(seed).hex()
    lines = [FETCH_TAG, "fetch", room_code, recipient_pk, str(ts), nonce]
    body = {"room_code": room_code, "recipient_pk": recipient_pk, "ts": ts, "nonce": nonce,
            "sig": ed25519.sign(seed, chr(10).join(lines).encode("utf-8")).hex()}
    if since is not None:
        body["since"] = since
    return json.dumps(body, ensure_ascii=False)


def _function_request(config, name, body):
    headers = _common_headers(config)
    headers["Content-Type"] = "application/json"
    return {"method": "POST", "url": f"{config.url}/functions/v1/{name}", "headers": headers, "body": body}


def post_message_request(config, body):
    return _function_request(config, "post-message", body)


def fetch_messages_request(config, body):
    return _function_request(config, "fetch-messages", body)


def read_fetch_response(status, body):
    """The message rows for this recipient. Anything malformed raises ValueError (the whole answer is dropped)."""
    if status != 200:
        raise ValueError(f"status {status}")
    try:
        value = json.loads(body)
    except ValueError as error:
        raise ValueError("answer is not JSON") from error
    rows = value.get("messages") if isinstance(value, dict) else None
    if not isinstance(rows, list):
        raise ValueError("answer has no messages list")
    out = []
    for row in rows:
        if not re.fullmatch(r"[0-9a-f]{32}", str(row.get("message_id", ""))):
            raise ValueError("message id is not valid")
        if not valid_pk(row.get("sender_pk")):
            raise ValueError("sender key is not valid")
        if not isinstance(row.get("ts"), int) or isinstance(row.get("ts"), bool):
            raise ValueError("message ts is not a number")
        if not _NONCE_PATTERN.match(str(row.get("nonce", ""))):
            raise ValueError("message nonce is not valid")
        if not _valid_ciphertext(row.get("ciphertext")):
            raise ValueError("ciphertext is not valid")
        if not isinstance(row.get("created_at"), str):
            raise ValueError("message time is missing")
        out.append({"message_id": row["message_id"], "sender_pk": row["sender_pk"], "ts": row["ts"], "nonce": row["nonce"],
                    "ciphertext": row["ciphertext"], "created_at": row["created_at"]})
    return out


def members_noise_request(config, member_pks):
    """GET the Noise keys and names of the members (peers table): pk=in.(...)&select=pk,noise_pk,name. Empty list: no request."""
    if any(not valid_pk(pk) for pk in member_pks):
        raise ValueError("member key is not valid")
    headers = _common_headers(config)
    headers["Accept"] = "application/json"
    return {"method": "GET", "url": f"{config.url}/rest/v1/peers?pk=in.({','.join(member_pks)})&select=pk,noise_pk,name",
            "headers": headers, "body": None}


def read_members_noise(status, body):
    """(pk, noise_pk, name) triples from the peers answer; a name that is not valid is "". ValueError when the answer is not a list of valid rows."""
    if status != 200:
        raise ValueError(f"status {status}")
    try:
        rows = json.loads(body)
    except ValueError as error:
        raise ValueError("answer is not JSON") from error
    if not isinstance(rows, list):
        raise ValueError("answer is not a list")
    out = []
    for row in rows:
        if not valid_pk(row.get("pk")) or not valid_pk(row.get("noise_pk")):
            raise ValueError("row key is not valid")
        name = row.get("name")
        out.append((row["pk"], row["noise_pk"], name if valid_name(name) else ""))
    return out
