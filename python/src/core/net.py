"""The LAN network of the chat: the announce, the encrypted send and the peer checks that go with them.

Moved out of talk.py unchanged. core/chat.py holds the chat state and the receiving side; talk.py re-exports both.
"""

import errno
import ipaddress
import json
import socket
import threading
import time

from core import noise, store
from core.events import Message
from core.names import clean_text, plain_sender, sender_name, squeeze_spaces
from i18n import t


APP = "herdr-talk"


ANNOUNCE_SECONDS = 2


MAX_TEXT_CHARS = 2000


SEND_TIMEOUT_SECONDS = 3


MAX_FRAME = 0xFFFF             # a Noise frame carries a 2-byte length


STOP = threading.Event()  # run() 이 끝날 때 설정한다. 네트워크 스레드는 이것을 보고 멈춘다.


def announce(udp_port, me):
    """Broadcast this instance's name, port and key every few seconds."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    try:
        while not STOP.is_set():
            fields = {"app": APP, "id": me["id"], "name": me["name"], "port": me["port"]}
            if me.get("key") is not None:
                fields["key"] = store.to_hex(me["key"])
            fields["commits"] = dict(me.get("commits") or {})  # always present, empty when there are none (Rust rejects a notice without it)
            try:
                sock.sendto(json.dumps(fields).encode("utf-8"), ("255.255.255.255", udp_port))
            except OSError:
                pass
            STOP.wait(ANNOUNCE_SECONDS)
    finally:
        sock.close()


def is_local_address(ip):
    """True for this PC's own addresses: loopback, and any address of any adapter here (LAN, VPN, VirtualBox, WSL).
    A broadcast leaves through whichever adapter the OS picks, so the sender address need not be the default-route one."""
    try:
        address = ipaddress.ip_address(ip)
    except ValueError:
        return False
    if address.is_loopback:
        return True
    probe = socket.socket(socket.AF_INET6 if address.version == 6 else socket.AF_INET, socket.SOCK_DGRAM)
    try:
        probe.bind((ip, 0))  # only an address assigned to this PC can be bound; port 0 sends nothing
        return True
    except OSError:
        return False
    finally:
        probe.close()


class _Deadline:
    """A connection that must finish within its time in total. Each read or write gets only the time left, so a peer that
    drips bytes slowly cannot hold the thread."""

    def __init__(self, conn, seconds):
        self.conn = conn
        self.until = time.monotonic() + seconds

    def _arm(self):
        left = self.until - time.monotonic()
        if left <= 0:
            raise socket.timeout("connection took too long")
        self.conn.settimeout(left)

    def recv(self, size):
        self._arm()
        return self.conn.recv(size)

    def sendall(self, data):
        self._arm()
        self.conn.sendall(data)

    def recv_exact(self, size):
        data = b""
        while len(data) < size:
            chunk = self.recv(size - len(data))
            if not chunk:
                raise ConnectionError("connection closed")
            data += chunk
        return data

    def read_frame(self):
        return self.recv_exact(int.from_bytes(self.recv_exact(2), "big"))

    def write_frame(self, data):
        if len(data) > MAX_FRAME:
            raise ValueError("frame too long")
        self.sendall(len(data).to_bytes(2, "big") + data)


def deliver(data, key, chat):
    """Show one message. With a key, the id must be that key's id. Without one, the id must not be a key-derived id.
    In Supabase mode the LAN carries no chat: the message is dropped (discovery and trust are not affected)."""
    if chat.supabase:
        return
    try:
        message = json.loads(data.decode("utf-8"))
    except ValueError:
        return
    if not isinstance(message, dict) or message.get("app") != APP:
        return
    sender, body = message.get("from"), message.get("text")
    peer_id = message.get("id")
    peer_id = peer_id if isinstance(peer_id, str) else ""
    if not (peer_id == store.peer_id(key) if key is not None else len(peer_id) != 64):
        return
    if not isinstance(sender, str) or not isinstance(body, str):
        return  # 형식이 틀린 메시지는 버린다
    # Rows are kept as sent, one per line, within the text limit. Each row is cleaned and spaces squeezed.
    rows = [squeeze_spaces(clean_text(row, MAX_TEXT_CHARS)) for row in body[:MAX_TEXT_CHARS].split("\n")]
    if all(not row.strip() for row in rows):
        return
    if key is not None:
        # Verified: the name the user verified, not the one the message claims. One line.
        name = chat.trusted_name(key)
        chat.emit(Message(name if name is not None else sender_name(sender), squeeze_spaces(' '.join(rows))))
        return
    # Plain: no key to check. Every row carries the plain sender's label, so no row can pass for another sender.
    label = f"{plain_sender(sender)} ({t('chat.plain')})"
    for row in rows:
        chat.emit(Message(label, row))


_PEER_CLOSED_ERRNOS = {getattr(errno, name) for name in ("ENOTCONN", "ECONNRESET", "EPIPE") if hasattr(errno, name)}


_PEER_CLOSED_WINERRORS = {10053, 10054}  # WSAECONNABORTED, WSAECONNRESET


def _peer_closed(error):
    """True when the error only says the other side has already closed the connection."""
    if isinstance(error, (ConnectionResetError, BrokenPipeError)):
        return True
    if getattr(error, "winerror", None) in _PEER_CLOSED_WINERRORS:
        return True
    return getattr(error, "errno", None) in _PEER_CLOSED_ERRNOS


def send_to(peer, me, text):
    """Send one message. A peer with a key only ever gets the encrypted form.
    Whether a keyless peer may get plain text is decided by the caller (Chat)."""
    message = json.dumps({"app": APP, "id": me["id"], "from": me["name"], "text": text}).encode("utf-8")
    key = peer.get("key")
    if key is not None and me.get("secret") is None:
        raise OSError("no identity to encrypt with")
    with socket.create_connection((peer["ip"], peer["port"]), timeout=SEND_TIMEOUT_SECONDS) as conn:
        if key is None:
            conn.sendall(message)
        else:
            try:
                _send_encrypted(_Deadline(conn, SEND_TIMEOUT_SECONDS), me, key, message)
            except (noise.NoiseError, ValueError) as error:
                raise OSError(str(error)) from error
        try:
            conn.shutdown(socket.SHUT_WR)
        except OSError as error:
            # The peer closed first (it dropped a plain message), so the data was already sent. Windows reports
            # WinError 10054 (ConnectionResetError); Linux reports ENOTCONN (107) after the peer's RST. Other errors stay.
            if not _peer_closed(error):
                raise


def _send_encrypted(conn, me, peer_key, message):
    """Noise IK initiator: handshake with the peer's key, then one encrypted message."""
    handshake = noise.Initiator(me["secret"], peer_key)
    conn.write_frame(handshake.write_message1())
    handshake.read_message2(conn.read_frame())
    conn.write_frame(handshake.send.encrypt(message))
