"""Supabase REST side of the relay: settings, the requests for the `peers` table, and checks of the answers.

Same rules as rust/crates/talk-core/src/relay_rest.rs. Standard library only (urllib for sending).
The key is the publishable key; the secret key is never read or stored here.
"""

import json
import os
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
PUT_TAG = "herdr-talk-put-v2"
TIMEOUT_SECONDS = 5

_PK_PATTERN = re.compile(r"^[0-9a-f]{64}$")
_NONCE_PATTERN = re.compile(r"^[0-9a-f]{32}$")


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


def config_from_env():
    return config(os.environ.get(URL_VARIABLE), os.environ.get(KEY_VARIABLE))


def valid_pk(text):
    return isinstance(text, str) and _PK_PATTERN.match(text) is not None


def valid_name(text):
    """1 to 32 characters, no control characters (category Cc, the same rule the table enforces)."""
    if not isinstance(text, str) or not 1 <= len(text) <= 32:
        return False
    return not any(unicodedata.category(ch) == "Cc" for ch in text)


def _common_headers(config):
    return {"apikey": config.key, "Authorization": f"Bearer {config.key}"}


def put_message(pk, noise_pk, name, ts, nonce):
    """The bytes that are signed (v2): PUT_TAG, pk, noise_pk, name, ts, nonce, one per line, no newline at the end, UTF-8."""
    return "\n".join([PUT_TAG, pk, noise_pk, name, str(ts), nonce]).encode("utf-8")


def put_request(config, seed, noise_pk, name, ts=None, nonce=None):
    """Signed write of this PC's row through the put-peer function. `seed` is the 32-byte Ed25519 private key (pk is its
    public key); `noise_pk` is the X25519 key of the Noise identity. ts defaults to now, nonce to 16 random bytes.
    The body keeps the fixed order pk, noise_pk, name, ts, nonce, sig. Raises ValueError for bad input."""
    pk = ed25519.public_key(seed).hex()
    if not _PK_PATTERN.match(noise_pk or ""):
        raise ValueError("noise_pk must be 64 lowercase hex characters")
    if not valid_name(name):
        raise ValueError("name must be 1 to 32 characters without control characters")
    ts = int(time.time()) if ts is None else ts
    nonce = secrets.token_hex(16) if nonce is None else nonce
    if not isinstance(ts, int) or isinstance(ts, bool) or not _NONCE_PATTERN.match(nonce):
        raise ValueError("ts must be an integer and nonce 32 lowercase hex characters")
    sig = ed25519.sign(seed, put_message(pk, noise_pk, name, ts, nonce)).hex()
    headers = _common_headers(config)
    headers["Content-Type"] = "application/json"
    body = {"pk": pk, "noise_pk": noise_pk, "name": name, "ts": ts, "nonce": nonce, "sig": sig}
    return {
        "method": "POST",
        "url": f"{config.url}/functions/v1/put-peer",
        "headers": headers,
        "body": json.dumps(body, separators=(",", ":"), ensure_ascii=False),
    }


def get_request(config, pk):
    """Lookup of one peer by pk. Raises ValueError for bad input."""
    if not valid_pk(pk):
        raise ValueError("pk must be 64 lowercase hex characters")
    headers = _common_headers(config)
    headers["Accept"] = "application/json"
    query = urllib.parse.urlencode({"pk": f"eq.{pk}", "select": "pk,name"}, safe=",.")
    return {"method": "GET", "url": f"{config.url}/rest/v1/peers?{query}", "headers": headers, "body": None}


def put_succeeded(status):
    return 200 <= status < 300


def read_get_response(status, body, pk):
    """The answer to a lookup: None when there is no such peer. Malformed answers raise ValueError (drop them)."""
    if status != 200:
        raise ValueError(f"status {status}")
    try:
        rows = json.loads(body)
    except (ValueError, TypeError):
        raise ValueError("answer is not JSON")
    if not isinstance(rows, list):
        raise ValueError("answer is not a list")
    if not rows:
        return None
    if len(rows) != 1 or not isinstance(rows[0], dict):
        raise ValueError("expected one row")
    row = rows[0]
    answered = row.get("pk")
    name = row.get("name")
    if not valid_pk(answered) or answered != pk:
        raise ValueError("row pk does not match the request")
    if not valid_name(name):
        raise ValueError("row name is not valid")
    return {"pk": answered, "name": name}


def _send(request):
    """Send one request. Returns (status, body). A network error raises OSError."""
    data = request["body"].encode("utf-8") if request["body"] is not None else None
    req = urllib.request.Request(request["url"], data=data, headers=request["headers"], method=request["method"])
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_SECONDS) as response:
            return response.status, response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as error:
        return error.code, ""


def put_peer(config, seed, noise_pk, name):
    """True when the relay accepted the signed write."""
    try:
        return put_succeeded(_send(put_request(config, seed, noise_pk, name))[0])
    except (OSError, ValueError):
        return False


def get_peer(config, pk):
    """The relay's record for pk, or None (no such peer, a failure, or an answer that does not check out)."""
    try:
        status, body = _send(get_request(config, pk))
        return read_get_response(status, body, pk)
    except (OSError, ValueError):
        return None
