"""Relay path of the chat (Python edition, same rules as rust/crates/talk-core/src/relay_chat.rs).

Sending: the peer's row is read with get_peer; its noise_pk must be a key the user verified with /trust. The text is
sealed into one Noise IK message (message 1 of IK carries the payload, so the receiver learns and checks the sender's
static key) and sent as an envelope on the channel channel_name(my noise key, peer noise key).
Receiving: relay.open_envelope and the verified-sender check, then Noise decryption, then the payload checks
(version, time window, nonce not used before). Nothing is stored. Receive failures are silent; a failed send says why.

This module does not import talk.py. The chat gives it a `hooks` object with: say(text), show(sender, text),
trusted_name(key_bytes) -> name or None, set_status(text).
"""

import json
import queue
import re
import secrets
import threading
import time

import noise
import relay
import relay_key
import relay_realtime
import relay_replay
import relay_rest
import relay_settings
import websocket

RETRY_SECONDS = 5
PEER_WAIT_SECONDS = 30
SLICE_SECONDS = 0.2
NOISE_MAX = 65535
MAX_TEXT_CHARS = 2000

_NONCE = re.compile(r"^[0-9a-f]{32}$")


class Session:
    """Keys the relay thread needs: the Ed25519 seed (row signing) and the Noise key pair (envelopes)."""

    def __init__(self, seed, noise_secret, noise_public):
        self.seed = seed
        self.noise_secret = bytes(noise_secret)
        self.noise_public = bytes(noise_public)


def encode_payload(text, ts, nonce):
    """The plaintext inside the ciphertext (format v1): one JSON line, keys in this order."""
    return json.dumps({"v": 1, "ts": ts, "nonce": nonce, "text": text}, separators=(",", ":"), ensure_ascii=False)


def parse_payload(payload):
    """(ts, nonce, text) for version 1 with an integer ts, a 32-hex nonce and a text; None for anything else."""
    try:
        value = json.loads(payload)
    except (ValueError, TypeError):
        return None
    if not isinstance(value, dict):
        return None
    version, ts, nonce, text = value.get("v"), value.get("ts"), value.get("nonce"), value.get("text")
    if isinstance(version, bool) or version != 1:
        return None
    if isinstance(ts, bool) or not isinstance(ts, int) or ts < 0:
        return None
    if not isinstance(nonce, str) or not _NONCE.match(nonce) or not isinstance(text, str):
        return None
    return ts, nonce, text


def seal_payload(secret, peer_key, text):
    """One Noise IK message carrying the text as its payload (initiator side). ValueError when it does not fit."""
    message = noise.Initiator(secret, peer_key).write_message1(text.encode("utf-8"))
    if len(message) > NOISE_MAX:
        raise ValueError("message too long")
    return message


def open_payload(secret, expected, ct):
    """The text of one IK message from `expected` (responder side). None when it does not verify."""
    try:
        handshake = noise.Responder(secret)
        payload = handshake.read_message1(ct)
        if handshake.remote_static != bytes(expected):
            return None
        return payload.decode("utf-8")
    except (noise.NoiseError, ValueError, UnicodeDecodeError):
        return None


def clean_text(value, limit=MAX_TEXT_CHARS):
    """Control characters become spaces, the text is cut to `limit`, and runs of spaces are squeezed."""
    cleaned = "".join(" " if (ord(ch) < 32 or ord(ch) == 127) else ch for ch in value[:limit])
    return re.sub(r" {2,}", " ", cleaned).strip()


def receive(hooks, line, session, theirs, cache, now):
    """One envelope line. Order: envelope check, decryption, format (v1), time window, nonce not used before, then
    the line. Every failure is silent. Returns True when a line was shown."""
    try:
        envelope = relay_realtime.accept(line, session.noise_public, theirs)
    except relay.Reject:
        return False
    text = open_payload(session.noise_secret, theirs, envelope["ct"])
    if text is None:
        return False
    parsed = parse_payload(text)
    if parsed is None:
        return False
    ts, nonce, body = parsed
    if not cache.accept(nonce, ts, now):
        return False
    body = clean_text(body)
    if not body:
        return False
    hooks.show(hooks.trusted_name(bytes(theirs)) or "?", body)
    return True


def verified_key(hooks, peer):
    """The peer's Noise key from its row, when the user verified that key with /trust."""
    try:
        key = bytes.fromhex(peer["noise_pk"])
    except (KeyError, ValueError, TypeError):
        return None
    return key if len(key) == 32 and hooks.trusted_name(key) is not None else None


def send_text(hooks, config, peer_pk, session, theirs, link, text, now=None):
    """Re-check the row right before sending, then seal and send one envelope. Returns None or the notice to show."""
    peer = relay_rest.get_peer(config, peer_pk)
    if peer is None or verified_key(hooks, peer) != bytes(theirs):
        return "relay: peer not verified, not sent"
    payload = encode_payload(text, int(time.time()) if now is None else now, secrets.token_hex(16))
    try:
        ciphertext = seal_payload(session.noise_secret, theirs, payload)
    except (ValueError, noise.NoiseError):
        return "relay: message too long or not encrypted"
    line = relay.seal(session.noise_public, theirs, ciphertext)
    if line is None:
        return "relay: bad key length"
    try:
        link.send_envelope(line)
    except websocket.WebSocketError as error:
        return f"relay: {error}"
    return None


def make_session(state, noise_secret, noise_public):
    return Session(relay_key.load_or_create(state), noise_secret, noise_public)


class Relay:
    """The relay thread of one chat. start() replaces nothing: the chat stops the old one first."""

    def __init__(self, hooks, state, config, settings, session, connect=None):
        self.hooks = hooks
        self.state = state
        self.config = config
        self.settings = settings
        self.session = session
        self._connect = connect or relay_realtime.Link.connect
        self.outbox = queue.Queue()
        self.stop_event = threading.Event()
        self.thread = None

    def start(self):
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def stop(self):
        self.stop_event.set()

    def send(self, text):
        """Queue a typed text for the relay thread."""
        self.outbox.put(text)

    def _wait(self, seconds):
        """Sleep in slices so a stop is seen. Texts typed meanwhile are not sent: a notice says so."""
        until = time.monotonic() + seconds
        while time.monotonic() < until and not self.stop_event.is_set():
            while True:
                try:
                    self.outbox.get_nowait()
                except queue.Empty:
                    break
                self.hooks.say("relay: not connected, the message was not sent over the relay")
            time.sleep(SLICE_SECONDS)

    def _run(self):
        hooks, settings, session = self.hooks, self.settings, self.session
        cache = relay_replay.NonceCache()
        if relay_rest.valid_name(settings["name"]):
            if not self._publish(settings, session):
                hooks.set_status("relay: could not publish this PC's row")
        while not self.stop_event.is_set():
            peer = relay_rest.get_peer(self.config, settings["peer"])
            if peer is None:
                hooks.set_status("relay: peer not found on the relay")
                self._wait(PEER_WAIT_SECONDS)
                continue
            theirs = verified_key(hooks, peer)
            if theirs is None:
                hooks.set_status("relay: peer not verified yet (use /trust)")
                self._wait(PEER_WAIT_SECONDS)
                continue
            channel = relay.channel_name(session.noise_public, theirs)
            try:
                link = self._connect(self.config, channel)
            except (websocket.WebSocketError, ValueError) as error:
                hooks.set_status(f"relay: {error}, retrying")
                self._wait(RETRY_SECONDS)
                continue
            hooks.set_status("relay: connected")
            try:
                self._pump(link, theirs, cache)
            finally:
                link.close()

    def _publish(self, settings, session):
        """Write this PC's row. A 409 means another key owns the short id: make a new one and try again (up to 5)."""
        for _ in range(5):
            short = relay_key.load_or_create_short_id(self.state)
            status = relay_rest.put_status(self.config, session.seed, session.noise_public.hex(), short, settings["name"])
            if status is not None and relay_rest.put_succeeded(status):
                return True
            if status != 409:
                return False
            relay_key.replace_short_id(self.state, relay_key.new_short_id())
        return False

    def _pump(self, link, theirs, cache):
        hooks, session, settings = self.hooks, self.session, self.settings
        while not self.stop_event.is_set():
            while True:
                try:
                    text = self.outbox.get_nowait()
                except queue.Empty:
                    break
                notice = send_text(hooks, self.config, settings["peer"], session, theirs, link, text)
                if notice:
                    hooks.say(notice)
            try:
                line = link.poll_envelope()
            except websocket.WebSocketError as error:
                hooks.set_status(f"relay: {error}, retrying")
                return
            if line:
                receive(hooks, line, session, theirs, cache, int(time.time()))


def start(hooks, state, settings, noise_secret, noise_public, connect=None):
    """Start the relay for `settings`. Returns (Relay or None, status text). None: the relay is off (local mode, or the
    settings are empty/incomplete). Never raises for a bad setting: the status says why."""
    if settings["mode"] != relay_settings.SUPABASE:
        return None, ""
    if not settings["url"] and not settings["key"] and not settings["peer"]:
        return None, ""
    try:
        config = relay_rest.config(settings["url"], settings["key"])
    except ValueError as error:
        return None, f"relay: {error}"
    if config is None:
        return None, "relay: settings incomplete"
    if not relay_rest.valid_short(settings["peer"]):
        return None, "relay: peer id missing"
    try:
        session = make_session(state, noise_secret, noise_public)
    except (relay_key.SigningKeyError, OSError) as error:
        return None, f"relay: {error}"
    relay_thread = Relay(hooks, state, config, settings, session, connect=connect)
    relay_thread.start()
    return relay_thread, ""
