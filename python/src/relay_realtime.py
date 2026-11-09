"""Supabase Realtime (Phoenix channels v1.0.0) for the relay: frames, topic, URL, and an open channel (Link).
Same rules as rust/crates/talk-core/src/relay_realtime.rs. Public channel: nobody signs in, the publishable key is the
apikey in the URL, and the channel name (a hash of two keys) is the only thing that picks the room."""

import json
import time

import relay
import websocket

EVENT = "envelope"
VERSION = "1.0.0"
HEARTBEAT_SECONDS = 25
POLL_SECONDS = 0.5
JOIN_WAIT_SECONDS = 10
CONNECT_WAIT_SECONDS = 5

_KEY_CHARS = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-")


def topic_for(channel):
    """The channel topic on the wire: "realtime:" followed by the channel name."""
    return f"realtime:{channel}"


def socket_url(config):
    """wss for https (ws for loopback http, tests only), the publishable key in the query. ValueError never carries
    the URL or the key."""
    if config.url.startswith("https://"):
        base = "wss://" + config.url[len("https://"):]
    elif config.url.startswith("http://"):
        base = "ws://" + config.url[len("http://"):]
    else:
        raise ValueError("relay URL has no scheme")
    if not config.key or any(ch not in _KEY_CHARS for ch in config.key):
        raise ValueError("relay key has characters that are not allowed in the URL")
    return f"{base}/realtime/v1/websocket?apikey={config.key}&vsn={VERSION}"


def encode(frame):
    return json.dumps(
        {"topic": frame["topic"], "event": frame["event"], "payload": frame["payload"], "ref": frame.get("ref")},
        separators=(",", ":"),
        ensure_ascii=False,
    )


def decode(text):
    """A frame dict, or None for anything that is not a Phoenix message."""
    try:
        value = json.loads(text)
    except (ValueError, TypeError):
        return None
    if not isinstance(value, dict) or not isinstance(value.get("topic"), str) or not isinstance(value.get("event"), str):
        return None
    ref = value.get("ref")
    if ref is not None and not isinstance(ref, str):
        return None
    return {"topic": value["topic"], "event": value["event"], "payload": value.get("payload"), "ref": ref}


def join_frame(topic, ref):
    return {
        "topic": topic,
        "event": "phx_join",
        "payload": {"config": {"broadcast": {"ack": False, "self": False}, "presence": {"key": ""}, "private": False}},
        "ref": ref,
    }


def heartbeat_frame(ref):
    return {"topic": "phoenix", "event": "heartbeat", "payload": {}, "ref": ref}


def broadcast_frame(topic, envelope_line, ref):
    """A broadcast carrying one envelope line, embedded as JSON (not as a string). ValueError when it is not JSON."""
    try:
        envelope = json.loads(envelope_line)
    except (ValueError, TypeError):
        raise ValueError("envelope is not JSON")
    return {"topic": topic, "event": "broadcast", "payload": {"type": "broadcast", "event": EVENT, "payload": envelope}, "ref": ref}


def envelope_of(frame, topic):
    """The envelope line carried by a received frame on `topic`, or None (any other frame gives None)."""
    if frame["topic"] != topic or frame["event"] != "broadcast":
        return None
    payload = frame["payload"]
    if not isinstance(payload, dict) or payload.get("type") != "broadcast" or payload.get("event") != EVENT:
        return None
    envelope = payload.get("payload")
    if not isinstance(envelope, dict):
        return None
    return json.dumps(envelope, separators=(",", ":"), ensure_ascii=False)


def may_send(peer, trusted_noise_pk):
    """May a message go to this peer? Only when the row's noise_pk is the key the user verified with /trust."""
    if not peer or not trusted_noise_pk:
        return False
    noise_pk = peer.get("noise_pk")
    return isinstance(noise_pk, str) and noise_pk == bytes(trusted_noise_pk).hex()


def accept(line, mine, theirs):
    """Accept a received envelope line only from the verified peer. `mine` and `theirs` are X25519 keys (bytes).
    Raises relay.Reject."""
    envelope = relay.open_envelope(line, (mine, theirs))
    if envelope["from"] != bytes(theirs):
        raise relay.Reject("party")
    return envelope


class Link:
    """An open channel: the websocket, the joined topic and the heartbeat timer."""

    def __init__(self, client, topic):
        self._client = client
        self.topic = topic
        self._next_ref = 1
        self._last_beat = time.monotonic()

    @classmethod
    def connect(cls, config, channel, clock=time.monotonic):
        """Connect, join the channel and wait for the join to be acknowledged. Raises websocket.WebSocketError."""
        client = websocket.connect(socket_url(config), timeout=CONNECT_WAIT_SECONDS)
        try:
            link = cls(client, topic_for(channel))
            client.settimeout(POLL_SECONDS)
            ref = link._reference()
            link._send(join_frame(link.topic, ref))
            deadline = clock() + JOIN_WAIT_SECONDS
            while clock() < deadline:
                frame = link._read_frame()
                if frame and frame["event"] == "phx_reply" and frame["ref"] == ref:
                    if isinstance(frame["payload"], dict) and frame["payload"].get("status") == "ok":
                        return link
                    raise websocket.WebSocketError("join refused")
            raise websocket.WebSocketError("join not acknowledged")
        except Exception:
            client.close()
            raise

    def _reference(self):
        ref = str(self._next_ref)
        self._next_ref += 1
        return ref

    def _send(self, frame):
        self._client.send_text(encode(frame))

    def _read_frame(self):
        text = self._client.recv_text()
        return decode(text) if text is not None else None

    def send_envelope(self, envelope_line):
        self._send(broadcast_frame(self.topic, envelope_line, self._reference()))

    def poll_envelope(self):
        """The next envelope line from the channel, or None. Sends a heartbeat when one is due."""
        if time.monotonic() - self._last_beat >= HEARTBEAT_SECONDS:
            self._send(heartbeat_frame(self._reference()))
            self._last_beat = time.monotonic()
        frame = self._read_frame()
        return envelope_of(frame, self.topic) if frame else None

    def close(self):
        self._client.close()
