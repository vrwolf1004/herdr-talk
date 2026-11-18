"""Room messages (Rust talk-core relay_chat/room_relay.rs and relay_replay.rs, the same rules).

A typed text goes to every trusted member: one Noise message per member (the same IK message as on the LAN), all in one
post. A message for this PC is shown only when its sender is a member, the Noise key is the member's and trusted, the
payload matches the row (ts, nonce), and the nonce is new inside the window. No screen and no locale text here: the
functions return values, and the screen decides what to show.
"""

import json
import secrets
import time
from collections import deque

from core import noise, relay_rest
from core.names import clean_text, squeeze_spaces

WINDOW_SECONDS = 300      # a message is accepted only while its ts is this close to now (Rust relay_replay.rs)
CAPACITY = 10000          # nonces remembered; a full cache refuses new messages rather than forget a live nonce
MAX_TEXT_CHARS = 2000     # the same limit as the LAN text (talk.py)


class NonceCache:
    """Each nonce is accepted once, and only inside the window. Nothing is recorded for a refused message."""

    def __init__(self, capacity=CAPACITY):
        self.capacity = capacity
        self.entries = deque()  # (ts, nonce) in arrival order
        self.seen = set()

    def accept(self, nonce, ts, now):
        if abs(now - ts) > WINDOW_SECONDS:
            return False
        self._prune(now)
        if nonce in self.seen or len(self.entries) >= self.capacity:
            return False
        self.seen.add(nonce)
        self.entries.append((ts, nonce))
        return True

    def _prune(self, now):
        while self.entries and abs(now - self.entries[0][0]) > WINDOW_SECONDS:
            _, nonce = self.entries.popleft()
            self.seen.discard(nonce)


def payload_bytes(name, text, ts, nonce):
    """The sealed payload (the JSON Rust sends: v, ts, nonce, name, text)."""
    return json.dumps({"v": 1, "ts": ts, "nonce": nonce, "name": name, "text": text}, ensure_ascii=False).encode("utf-8")


def seal(secret, peer_noise, payload):
    """One Noise IK message to a member: the sender's key is proven to the receiver by the handshake."""
    return noise.Initiator(secret, peer_noise).write_message1(payload)


def open_message(secret, ciphertext):
    """(payload, sender's Noise public key) of one message to this PC. Raises noise.NoiseError for a bad one."""
    responder = noise.Responder(secret)
    payload = responder.read_message1(ciphertext)
    return payload, responder.remote_static


def check_row(row, members, secret, cache, now, not_before=0):
    """The (name, text) to show for one fetched row, or None when it must not be shown (every failure is silent).

    members: {sender_pk: noise_pk bytes} of the room. Every member of the room is accepted (no /trust step in rooms): the
    name shown is the one the sender wrote into the payload, or the first 4 characters of its key when that name is not valid.
    not_before: a row sent before this time (seconds) is dropped: the node takes only what came after it started."""
    if row["ts"] < not_before:
        return None
    sender_noise = members.get(row["sender_pk"])
    if sender_noise is None:
        return None
    try:
        payload, remote = open_message(secret, bytes.fromhex(row["ciphertext"]))
    except (noise.NoiseError, ValueError):
        return None
    if remote != sender_noise:
        return None
    try:
        value = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return None
    if not isinstance(value, dict) or value.get("v") != 1 or value.get("ts") != row["ts"] or value.get("nonce") != row["nonce"]:
        return None
    if not cache.accept(row["nonce"], row["ts"], now):
        return None
    text = clean_text(value.get("text") if isinstance(value.get("text"), str) else "", MAX_TEXT_CHARS)
    if not text.strip():
        return None
    claimed = squeeze_spaces(clean_text(value.get("name") if isinstance(value.get("name"), str) else "", 32)).strip()
    return (claimed if relay_rest.valid_name(claimed) else row["sender_pk"][:4]), text


def post_text(config, seed, room_code, secret, recipients, name, text, ts=None):
    """Send one text to the trusted recipients ((pk, noise bytes) pairs) in one post. Returns True when the server
    answered 200. Raises OSError on a network error and ValueError for bad input."""
    ts = int(time.time()) if ts is None else ts
    nonce = secrets.token_hex(16)
    message_id = secrets.token_hex(16)
    payload = payload_bytes(name, text, ts, nonce)
    deliveries = [{"recipient_pk": pk, "ciphertext": seal(secret, noise_pk, payload).hex()}
                  for pk, noise_pk in sorted(recipients, key=lambda pair: pair[0])]
    body = relay_rest.post_message_body(seed, room_code, message_id, ts, nonce, deliveries)
    status, _ = relay_rest._send(relay_rest.post_message_request(config, body))
    return status == 200


def fetch_rows(config, seed, room_code, since=None, ts=None):
    """This PC's inbox rows for the room (ValueError for a bad answer, OSError for a network error)."""
    ts = int(time.time()) if ts is None else ts
    body = relay_rest.fetch_message_body(seed, room_code, since, ts, secrets.token_hex(16))
    status, answer = relay_rest._send(relay_rest.fetch_messages_request(config, body))
    return relay_rest.read_fetch_response(status, answer)


def display_name(names, pk, name):
    """The name shown for a sender: the 대화명, plus the first 4 characters of the key when another known key has the
    same 대화명 (Rust room_relay.rs label)."""
    if any(other != pk and other_name == name for other, other_name in names.items()):
        return f"{name} ({pk[:4]})"
    return name
