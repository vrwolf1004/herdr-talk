"""Minimal WebSocket client (RFC 6455) on the standard library: the Upgrade handshake, masked text frames, reading
text frames with fragments, ping/pong and close. No extensions, no binary messages.

Used by relay_realtime for the Supabase Realtime connection. wss:// uses ssl with the default certificate checks.
"""

import base64
import hashlib
import os
import socket
import ssl
import struct
from urllib.parse import urlsplit

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
MAX_MESSAGE = 1024 * 1024  # bytes; a larger message closes the connection


class WebSocketError(Exception):
    """Connection or protocol failure. The message never contains the URL or a key."""


def accept_key(client_key):
    """The Sec-WebSocket-Accept value for a Sec-WebSocket-Key (RFC 6455 section 4.2.2)."""
    digest = hashlib.sha1((client_key + GUID).encode("ascii")).digest()
    return base64.b64encode(digest).decode("ascii")


def _mask(payload, key):
    return bytes(byte ^ key[index % 4] for index, byte in enumerate(payload))


def encode_frame(opcode, payload, mask=True, fin=True):
    """One frame. Client frames are masked (mask=True)."""
    first = (0x80 if fin else 0) | opcode
    length = len(payload)
    mask_bit = 0x80 if mask else 0
    if length < 126:
        header = struct.pack("!BB", first, mask_bit | length)
    elif length < 65536:
        header = struct.pack("!BBH", first, mask_bit | 126, length)
    else:
        header = struct.pack("!BBQ", first, mask_bit | 127, length)
    if not mask:
        return header + payload
    key = os.urandom(4)
    return header + key + _mask(payload, key)


def read_frame(read):
    """Read one frame with `read(n)` (must return exactly n bytes or raise). Returns (fin, opcode, payload)."""
    first, second = read(2)
    fin = bool(first & 0x80)
    opcode = first & 0x0F
    masked = bool(second & 0x80)
    length = second & 0x7F
    if length == 126:
        (length,) = struct.unpack("!H", read(2))
    elif length == 127:
        (length,) = struct.unpack("!Q", read(8))
    if length > MAX_MESSAGE:
        raise WebSocketError("frame too large")
    key = read(4) if masked else None
    payload = read(length) if length else b""
    if key is not None:
        payload = _mask(payload, key)
    return fin, opcode, payload


class Client:
    """A connected WebSocket. Create with connect(). Not thread-safe: one reader or writer at a time."""

    def __init__(self, sock):
        self._sock = sock
        self._buffer = b""
        self.closed = False

    def _read(self, count):
        while len(self._buffer) < count:
            chunk = self._sock.recv(65536)
            if not chunk:
                raise WebSocketError("connection closed")
            self._buffer += chunk
        data, self._buffer = self._buffer[:count], self._buffer[count:]
        return data

    def send_text(self, text):
        if self.closed:
            raise WebSocketError("connection closed")
        self._sock.sendall(encode_frame(0x1, text.encode("utf-8")))

    def recv_text(self):
        """The next text message, or None when no complete message is ready within the socket timeout.
        Pings are answered; a close frame marks the connection closed and raises."""
        parts = []
        size = 0
        while True:
            try:
                fin, opcode, payload = read_frame(self._read)
            except socket.timeout:
                if parts:
                    raise WebSocketError("message cut off by timeout")
                return None
            if opcode == 0x9:  # ping
                self._sock.sendall(encode_frame(0xA, payload))
                continue
            if opcode == 0xA:  # pong
                continue
            if opcode == 0x8:  # close
                self.closed = True
                try:
                    self._sock.sendall(encode_frame(0x8, payload[:2]))
                except OSError:
                    pass
                self.close()
                raise WebSocketError("closed by server")
            if opcode in (0x1, 0x0):
                size += len(payload)
                if size > MAX_MESSAGE:
                    raise WebSocketError("message too large")
                parts.append(payload)
                if fin:
                    return b"".join(parts).decode("utf-8", errors="strict")
                continue
            raise WebSocketError("unsupported frame")

    def settimeout(self, seconds):
        self._sock.settimeout(seconds)

    def close(self):
        if not self.closed:
            self.closed = True
            try:
                self._sock.sendall(encode_frame(0x8, struct.pack("!H", 1000)))
            except OSError:
                pass
        try:
            self._sock.close()
        except OSError:
            pass


def connect(url, timeout=5.0, context=None):
    """Open a WebSocket to a ws:// or wss:// URL (query string kept). Raises WebSocketError on any failure."""
    parts = urlsplit(url)
    if parts.scheme == "wss":
        secure, default_port = True, 443
    elif parts.scheme == "ws":
        secure, default_port = False, 80
    else:
        raise WebSocketError("URL is not ws or wss")
    if not parts.hostname:
        raise WebSocketError("URL has no host")
    port = parts.port or default_port
    target = parts.path or "/"
    if parts.query:
        target += "?" + parts.query
    try:
        raw = socket.create_connection((parts.hostname, port), timeout=timeout)
    except OSError:
        raise WebSocketError("connect failed")
    try:
        if secure:
            ctx = context or ssl.create_default_context()
            raw = ctx.wrap_socket(raw, server_hostname=parts.hostname)
        client_key = base64.b64encode(os.urandom(16)).decode("ascii")
        host = parts.hostname if parts.port is None else f"{parts.hostname}:{parts.port}"
        request = (
            f"GET {target} HTTP/1.1\r\n"
            f"Host: {host}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {client_key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "\r\n"
        )
        raw.sendall(request.encode("ascii"))
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = raw.recv(1024)
            if not chunk:
                raise WebSocketError("handshake closed")
            head += chunk
            if len(head) > 16384:
                raise WebSocketError("handshake too long")
        head, rest = head.split(b"\r\n\r\n", 1)
        lines = head.decode("iso-8859-1").split("\r\n")
        if not lines[0].startswith("HTTP/1.1 101"):
            raise WebSocketError("handshake refused")
        headers = {}
        for line in lines[1:]:
            name, _, value = line.partition(":")
            headers[name.strip().lower()] = value.strip()
        if headers.get("sec-websocket-accept") != accept_key(client_key):
            raise WebSocketError("bad Sec-WebSocket-Accept")
        client = Client(raw)
        client._buffer = rest
        client.settimeout(timeout)
        return client
    except WebSocketError:
        raw.close()
        raise
    except OSError:
        raw.close()
        raise WebSocketError("handshake failed")
