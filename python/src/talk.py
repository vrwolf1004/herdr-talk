"""herdr-talk: simple chat between PCs on the same network (phase 1, no server).

How it works
  - Every instance broadcasts its name and TCP port over UDP every few seconds.
  - Other instances on the same network hear the broadcast and remember the peer.
  - A message is sent straight to each peer over TCP. Nothing is stored.

Screen
  - One screen on every console, the same layout as the Rust edition: a status row, messages, a divider, and the input row at the bottom.

Commands (typed in the input line)
  <text>     send to every peer
  /peers     list peers seen recently
  /help      show commands
  /quit      leave the chat
"""

import argparse
import itertools
import hmac
import ipaddress
import json
import locale
import os
import random
import re
import secrets
import shutil
import socket
import subprocess
import sys
import threading
import time
import unicodedata
import uuid
from concurrent.futures import ThreadPoolExecutor

import herdr_api
import noise
import phone
import store


def load_dock_layout():
    """설치된 herdr-dock-grid 가 있으면 그 원본을, 없으면 이 plugin 의 복사본(src/dock_layout.py)을 쓴다."""
    grid = herdr_api.installed_plugin_path("herdr-dock-grid")
    if grid:
        sys.path.insert(0, os.path.join(grid, "src"))
    import dock_layout
    return dock_layout


dock = load_dock_layout()
from i18n import t

APP = "herdr-talk"
DEFAULT_UDP_PORT = 47850  # discovery port, the same on every PC
UDP_PORT_VARIABLE = "HERDR_TALK_UDP_PORT"  # only for separate test networks: unset means the default


def parse_udp_port(text):
    """The discovery port for a value of HERDR_TALK_UDP_PORT: None or empty gives the default. Anything that is not
    a port number from 1 to 65535 raises ValueError; it never falls back silently."""
    if text is None or not text.strip():
        return DEFAULT_UDP_PORT
    stripped = text.strip()
    if stripped.isascii() and stripped.isdigit() and 1 <= int(stripped) <= 65535:
        return int(stripped)
    raise ValueError(f"{UDP_PORT_VARIABLE} must be a port number from 1 to 65535, got {text!r}")


def udp_port():
    return parse_udp_port(os.environ.get(UDP_PORT_VARIABLE))
ANNOUNCE_SECONDS = 2
PEER_TIMEOUT_SECONDS = 10
AUTO_RENAME_GAP_SECONDS = 60  # 자동 이름 변경 사이의 최소 간격
MAX_MESSAGE_BYTES = 64 * 1024
RECEIVE_TIMEOUT_SECONDS = 5
LOOP_POLL_SECONDS = 0.5  # 종료 신호를 이 간격으로 확인한다
MAX_TEXT_CHARS = 2000
SEND_WORKERS = 8  # 메시지를 동시에 보내는 피어 수의 상한
MAX_LINES = 2000
IDLE_LIMIT_SECONDS = 10 * 60   # leave automatically after this long without any chat activity
SEND_TIMEOUT_SECONDS = 3
CONNECTION_SECONDS = 3         # total time for one incoming connection: first byte, handshake and message
MAX_CONNECTIONS = 16           # incoming connections handled at once, all addresses together
MAX_PER_IP = 3                 # ... and from one address
MESSAGE_LIMIT = 20             # messages one sender address may deliver within MESSAGE_WINDOW_SECONDS
MESSAGE_WINDOW_SECONDS = 10
PHONE_SHOWN_SECONDS = 30       # how long the phone address and token stay on screen when nobody hides them
MAX_FRAME = 0xFFFF             # a Noise frame carries a 2-byte length

# Display names picked at random when --name is not given: the list for this PC's language, else every list.
# Each entry: language code -> (Windows language name, names).
NAMES_BY_LANGUAGE = {
    "ko": ("korean", ["민준", "서준", "도윤", "예준", "시우", "하준", "지호", "주원", "서연", "서윤"]),
    "en": ("english", ["Liam", "Olivia", "Noah", "Emma", "Oliver", "Ava", "Elijah", "Sophia", "James", "Isabella"]),
    "zh": ("chinese", ["Haoran", "Yifan", "Ziyi", "Jiahui", "Xinyi", "Yu", "Mei", "Jie", "Lin", "Ting"]),
    "ja": ("japanese", ["Haruto", "Yui", "Sota", "Hina", "Ren", "Sakura", "Yuto", "Riku", "Aoi", "Mio"]),
    "vi": ("vietnamese", ["Anh", "Minh", "Huong", "Linh", "Tuan", "Hoa", "Duc", "Thao", "Phuc", "Lan"]),
    "fr": ("french", ["Louis", "Gabriel", "Jules", "Lea", "Chloe", "Manon", "Lucas", "Hugo", "Camille", "Adam"]),
    "de": ("german", ["Ben", "Mia", "Emil", "Hannah", "Paul", "Lina", "Leon", "Sofia", "Finn", "Greta"]),
    "es": ("spanish", ["Lucia", "Martin", "Mateo", "Valentina", "Pablo", "Daniela", "Diego", "Paula", "Sergio", "Carmen"]),
    "ru": ("russian", ["Mikhail", "Alexander", "Maria", "Artem", "Anna", "Ivan", "Daria", "Dmitry", "Polina", "Egor"]),
    "id": ("indonesian", ["Budi", "Siti", "Agus", "Dewi", "Rizky", "Putri", "Adi", "Nur", "Fajar", "Ayu"]),
    "pt": ("portuguese", ["Miguel", "Helena", "Arthur", "Alice", "Heitor", "Laura", "Bernardo", "Julia", "Davi", "Theo"]),
}


def pick_name(names, taken=()):
    """Random name that is not in taken. If every name is taken, fall back to the whole list."""
    free = [name for name in names if name not in taken]
    return random.choice(free or names)


def unique_name(base, taken):
    """base with the smallest number from 2 that is not in taken, e.g. 지호 -> 지호2."""
    for number in itertools.count(2):
        candidate = f"{base[:30]}{number}"
        if candidate not in taken:
            return candidate


def resolve_name_clash(chat):
    """If a fresh peer uses our name and has the smaller id, rename this instance.
    Only one side renames (the one with the larger id), so both never change at once.
    Renames at most once per AUTO_RENAME_GAP_SECONDS. A chat with a key counts only peers whose announced key hashes
    to their id (a peer without one proves nothing, so it cannot take a name), and breaks an id tie, which two chats
    sharing one state folder have, by port. A chat without a key (tests, the old edition) counts every peer.
    Returns (old, new) when renamed, else None."""
    peers = chat.fresh_peers()
    name = chat.me["name"]
    keyed = chat.me.get("key") is not None
    mine = (chat.me["id"], chat.me["port"])
    if not any(info["name"] == name and (info.get("key") is not None or not keyed)
               and (info.get("id", pid), info["port"]) < mine for pid, info in peers.items()):
        return None
    last = chat.last_auto_rename
    if last is not None and time.monotonic() - last < AUTO_RENAME_GAP_SECONDS:
        return None
    chat.last_auto_rename = time.monotonic()
    new = unique_name(name, {info["name"] for info in peers.values()} | {name})
    chat.me["name"] = new
    return name, new


def default_name(taken=()):
    """Random name from the list matching the PC's language (e.g. Korean for a Korean locale)."""
    language = (locale.getlocale()[0] or "").lower()
    for code, (language_name, names) in NAMES_BY_LANGUAGE.items():
        if language.startswith(code) or language.startswith(language_name):
            return pick_name(names, taken)
    return pick_name([name for _, names in NAMES_BY_LANGUAGE.values() for name in names], taken)

_sink_lock = threading.Lock()
STOP = threading.Event()  # run() 이 끝날 때 설정한다. 네트워크 스레드는 이것을 보고 멈춘다.
_sink = None  # where say() writes; set by run()
_restore_screen = None  # undoes the full-screen mode while the chat runs; leave_when_idle() calls it


def say(line, mine=False):
    """Show one line in the chat (on screen, or plain stdout). mine marks a line the user sent."""
    with _sink_lock:
        if _sink is None:
            print(line, flush=True)
        else:
            _sink(line, mine)


class Chat:
    """Shared state: this instance, known peers, and the lines shown on screen."""

    def __init__(self, me, plaintext=True):
        # me: id, name, port; with an identity also key and secret (bytes). start_node() sets plaintext from config.json
        # (off unless the user turns it on). The default keeps a chat built without an identity, as in tests, plain.
        self.me = me
        self.plaintext = plaintext
        self.trusted = []         # verified peers: {"name", "key", "ip"}
        self.state_dir = None     # where /trust saves; None keeps it for this session
        self.received = {}        # sender address -> arrival times of recent messages
        self.host = None          # phone.Host while this node is the phone host (방장) of this PC
        self.phone_until = None   # time.monotonic() until which the phone address is on screen. Only the screen shows it.
        self.phone_lock = threading.Lock()
        self.phone_timer = None
        self.on_phone_change = None  # the screen sets this to redraw
        self.received_lock = threading.Lock()
        self.peers = {}
        self.peers_lock = threading.Lock()
        self.lines = []
        self.lines_lock = threading.Lock()  # 네트워크 스레드와 화면이 같은 목록을 쓴다
        self.last_activity = time.time()
        self.last_auto_rename = None  # 자동 이름 변경 시각 (time.monotonic)

    def touch(self):
        """Mark activity so the idle timer restarts. Used by every view, not only add_line()."""
        self.last_activity = time.time()

    def add_line(self, line, mine=False):
        self.touch()
        with self.lines_lock:
            self.lines.append((line, mine))
            del self.lines[:-MAX_LINES]

    def clear_lines(self):
        with self.lines_lock:
            self.lines.clear()

    def fresh_peers(self):
        cutoff = time.time() - PEER_TIMEOUT_SECONDS
        with self.peers_lock:
            return {pid: info for pid, info in self.peers.items() if info["seen"] >= cutoff}

    def is_host(self):
        return self.host is not None

    def try_become_host(self):
        """Become the phone host if no node on this PC is. Shows the phone address once when it succeeds."""
        if self.state_dir is None or STOP.is_set() or self.host is not None:
            return False
        try:
            host = phone.Host.take(self.state_dir, self.me["port"])
        except OSError:
            return False
        if host is None:
            return False
        self.host = host
        self.show_phone()
        return True

    def release_host(self):
        """Give the host lock back (the chat is ending). The next node takes over."""
        host, self.host = self.host, None
        if host is not None:
            host.release()

    def watch_host(self):
        """Keep taking the host lock while another node holds it, so a node takes over when the host ends.
        shortcut: whichever waiting node polls first wins, not strictly the oldest; upgrade if the order must be fixed."""
        def loop():
            while not STOP.is_set():
                self.try_become_host()
                STOP.wait(2)
        threading.Thread(target=loop, daemon=True).start()

    def _phone_changed(self):
        if self.on_phone_change:
            self.on_phone_change()

    def show_phone(self):
        with self.phone_lock:
            self.phone_until = time.monotonic() + PHONE_SHOWN_SECONDS
            if self.phone_timer:
                self.phone_timer.cancel()
            self.phone_timer = threading.Timer(PHONE_SHOWN_SECONDS + 0.1, self.phone_visible)
            self.phone_timer.daemon = True
            self.phone_timer.start()
        self._phone_changed()

    def hide_phone(self):
        with self.phone_lock:
            was_shown, self.phone_until = self.phone_until is not None, None
        if was_shown:
            self._phone_changed()

    def toggle_phone(self):
        if self.phone_visible():
            self.hide_phone()
        else:
            self.show_phone()

    def phone_visible(self):
        """True while the phone address should be on screen. Hides it, and asks for a redraw, when its time is up."""
        with self.phone_lock:
            if self.phone_until is None:
                return False
            if time.monotonic() < self.phone_until:
                return True
            self.phone_until = None
        self._phone_changed()
        return False

    def phone_overlay(self):
        """The rows to draw over the screen, read from phone.json now, so the token is in no chat line or copy. None when hidden."""
        if not self.phone_visible():
            return None
        info = phone.read(self.state_dir) if self.state_dir else None
        if info is None:
            return [t("phone.none")]
        return [t("phone.title"), *[f"  {url}" for url in info["urls"]], t("phone.token", token=info["token"]), t("phone.hint")]

    def trusts(self, key):
        """True for a key the user verified, and for this PC's own key (another chat pane on this PC)."""
        return key == self.me.get("key") or any(peer["key"] == key for peer in self.trusted)

    def trusted_name(self, key):
        """The name the user verified this key under. None for a key that was not verified, this PC's own key included."""
        return next((peer["name"] for peer in self.trusted if peer["key"] == key), None)

    def link_of(self, info):
        """"plain" (no key), "encrypted" (verified key) or "unverified" (a key nobody checked)."""
        key = info.get("key")
        if key is None:
            return "plain"
        return "encrypted" if self.trusts(key) else "unverified"

    def admit_sender(self, ip):
        """Accept a message connection only from a fresh peer's address, and only up to MESSAGE_LIMIT per window.
        shortcut: a peer whose announce has not arrived yet loses its first message, the limit is per IP; upgrade if messages go missing after a join."""
        if not any(info["ip"] == ip for info in self.fresh_peers().values()):
            return False
        now = time.monotonic()
        with self.received_lock:
            times = self.received.setdefault(ip, [])
            times[:] = [at for at in times if now - at < MESSAGE_WINDOW_SECONDS]
            if len(times) >= MESSAGE_LIMIT:
                return False
            times.append(now)
        return True

    def trust(self, code):
        """Verify the unverified peer whose code matches: remember its key, and say so."""
        own = self.me.get("key")
        found = []
        for info in self.fresh_peers().values():
            key = info.get("key")
            if own is not None and key is not None and self.link_of(info) == "unverified" \
                    and store.normalize_code(store.fingerprint(own, key)) == code:
                found.append(info)
        # Two different keys with the same code would mean a collision or an attacker: refuse rather than pick one.
        if len({info["key"] for info in found}) > 1:
            say(t("trust.ambiguous", code=code))
            return
        if not found:
            say(t("trust.none", code=code))
            return
        info = found[0]
        self.trusted.append({"name": info["name"], "key": info["key"], "ip": info["ip"]})
        try:
            if self.state_dir is not None:
                store.write_trusted(self.state_dir, self.trusted)
        except OSError as error:
            say(t("trust.save_failed", error=str(error)))
            return
        say(t("trust.done", name=info["name"], ip=info["ip"]))

    def handle(self, text):
        """Handle one line typed by the user. Return False to leave the chat."""
        text = text.strip()
        if not text:
            return True
        # Commands are echoed before they run, as in the Rust edition. Chat messages are not echoed.
        if text.startswith("/"):
            say(f"> {text}")
        if text == "/quit":
            return False
        if text == "/help":
            for line in HELP_BOX:
                say(line)
            return True
        if text == "/name" or text.startswith("/name "):
            new_name = text[5:].strip()[:32]
            if not new_name:
                say(t("usage.name"))
                return True
            old_name = self.me["name"]
            self.me["name"] = new_name
            say(t("name.changed", old=old_name, new=new_name))
            return True
        if text == "/lang" or text.startswith("/lang "):
            code = text[5:].strip().lower()
            if code not in NAMES_BY_LANGUAGE:
                say(t("usage.lang", codes="|".join(NAMES_BY_LANGUAGE)))
                return True
            old_name = self.me["name"]
            taken = {info["name"] for info in self.fresh_peers().values()} | {old_name}
            self.me["name"] = pick_name(NAMES_BY_LANGUAGE[code][1], taken)
            say(t("name.changed", old=old_name, new=self.me["name"]))
            return True
        if text == "/peers":
            found = self.fresh_peers()
            if not found:
                say(t("peers.none"))
            own = self.me.get("key")
            for info in found.values():
                link = self.link_of(info)
                if link == "encrypted":
                    state = t("peers.verified")
                elif link == "plain":
                    state = t("peers.plain")
                else:
                    state = t("peers.unverified", code=store.fingerprint(own, info["key"])) if own is not None else ""
                say(f"- {clean_text(info['name'], 32)} ({info['ip']}:{info['port']}) {state}")
            return True
        if text == "/phone" or text.startswith("/phone "):
            argument = text[6:].strip()
            if not argument:
                self.show_phone()
            elif argument == "new":
                if self.state_dir is not None and phone.regenerate_token(self.state_dir):
                    say(t("phone.new_done"))
                    self.show_phone()
                else:
                    say(t("phone.none"))
            else:
                say(t("usage.phone"))
            return True
        if text == "/trust" or text.startswith("/trust "):
            code = store.normalize_code(text[6:])
            if code:
                self.trust(code)
            else:
                say(t("usage.trust"))
            return True
        text = text[:MAX_TEXT_CHARS].replace("\n", " ")
        found = self.fresh_peers()
        if not found:
            say(t("peers.none_hint"))
            return True
        # Only peers this link allows get the message. Each skipped peer says why.
        targets = {}
        for pid, info in found.items():
            link = self.link_of(info)
            if link == "encrypted" or (link == "plain" and self.plaintext):
                targets[pid] = info
            elif link == "plain":
                say(t("peer.plain_off", name=info["name"]))
            else:
                say(t("peer.unverified", name=info["name"]))
        if not targets:
            return True
        # 피어마다 따로 보낸다. 응답 없는 피어가 다른 피어 전송을 기다리게 하지 않도록 동시에 보내고,
        # 실패 메시지는 원래 피어 순서대로 낸다. 동시에 보내는 수는 SEND_WORKERS 로 제한한다.
        with ThreadPoolExecutor(max_workers=min(SEND_WORKERS, len(targets))) as pool:
            sends = [(info, pool.submit(send_to, info, self.me, text)) for info in targets.values()]
            for info, send in sends:
                try:
                    send.result()
                except OSError:
                    say(t("peer.unreachable", name=info["name"]))
        say(f"{self.me['name']} {t('chat.you')}: {text}", mine=True)
        return True


def announce(udp_port, me):
    """Broadcast this instance's name, port and key every few seconds."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    try:
        while not STOP.is_set():
            fields = {"app": APP, "id": me["id"], "name": me["name"], "port": me["port"]}
            if me.get("key") is not None:
                fields["key"] = store.to_hex(me["key"])
            try:
                sock.sendto(json.dumps(fields).encode("utf-8"), ("255.255.255.255", udp_port))
            except OSError:
                pass
            STOP.wait(ANNOUNCE_SECONDS)
    finally:
        sock.close()


def listen_for_peers(udp_port, chat):
    """Remember every other instance that announces itself."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("", udp_port))
    sock.settimeout(LOOP_POLL_SECONDS)
    try:
        _listen_loop(sock, chat)
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


def _listen_loop(sock, chat):
    while not STOP.is_set():
        try:
            data, addr = sock.recvfrom(4096)
        except socket.timeout:
            continue
        except OSError:
            return  # 소켓이 닫혔다 (종료 중)
        try:
            info = json.loads(data.decode("utf-8"))
        except ValueError:
            continue
        # Only a well-formed announcement is kept: an int port in range and a string id.
        # Anything else could put odd values on screen (via /peers) or crash this thread.
        if not isinstance(info, dict) or info.get("app") != APP:
            continue
        peer_id = info.get("id")
        port = info.get("port")
        if not isinstance(peer_id, str) or not peer_id:
            continue
        if not isinstance(port, int) or isinstance(port, bool) or not 1 <= port <= 65535:
            continue
        # Another chat on this PC has our id (and key) but another port.
        if peer_id == chat.me["id"] and port == chat.me["port"]:
            continue
        name = remember_peer(chat, info, addr[0])
        if name == chat.me["name"]:
            renamed = resolve_name_clash(chat)
            if renamed:
                say(t("name.changed", old=renamed[0], new=renamed[1]))


def remember_peer(chat, info, ip):
    """Keep one announcement (already checked for shape). Returns the peer's name, or None when it is refused.
    The checks are the Rust edition's: a key must hash to the id (else it is forged), a key-derived id without its key is a
    peer stripping its key to force plain text, and this PC's own key is taken only from this PC."""
    peer_id, port = info["id"], info["port"]
    key = None
    raw = info.get("key")
    if isinstance(raw, str):
        key = store.from_hex(raw)
        if key is None or len(key) != 32 or store.peer_id(key) != peer_id:
            return None
    elif len(peer_id) == 64:
        return None
    own = chat.me.get("key")
    if key is not None and key == own and not is_local_address(ip):
        return None
    # A key the user trusts: an announcement may not rename its entry or move it to another address.
    trusted = key is not None and chat.trusts(key)
    trusted_name = chat.trusted_name(key) if key is not None else None
    name = trusted_name if trusted_name is not None else peer_name(info.get("name", "unknown"))
    entry = {"id": peer_id, "name": name, "ip": ip, "port": port, "seen": time.time(), "key": key}
    with chat.peers_lock:
        if trusted:
            # Only refreshes an entry at the same address. A new address adds an entry; the old one stays as it was.
            # Another pane on this PC (own key, local address) may rename itself, since its key is ours.
            for known in chat.peers.values():
                if known.get("id") == peer_id and known["port"] == port and known["ip"] == ip:
                    known["seen"] = time.time()
                    if key == own:
                        known["name"] = name
                    return name
        else:
            for known_key, known in list(chat.peers.items()):
                if known.get("id") == peer_id and known["port"] == port:
                    del chat.peers[known_key]
                    chat.peers[f"{peer_id}:{port}@{ip}"] = entry
                    return name
        chat.peers[f"{peer_id}:{port}@{ip}"] = entry
    say(t("peer.joined", name=name, ip=ip))
    # The host of this PC joined (its chat port is in phone.json, a local file): show the phone address on this screen only.
    info = phone.read(chat.state_dir) if chat.state_dir else None
    if info and info["owner_port"] == port and info["owner_pid"] != os.getpid() and is_local_address(ip):
        chat.show_phone()
    if key is not None and own is not None and not chat.trusts(key):
        # A new key from an address we trusted before is a changed key, not a new peer, unless the trusted key is
        # still announcing from that address: then this is another identity on the same PC.
        alive = chat.fresh_peers().values()
        changed = any(
            peer["ip"] == ip and not any(a["ip"] == ip and a.get("key") == peer["key"] for a in alive)
            for peer in chat.trusted
        )
        if changed:
            say(t("peer.key_changed", name=name, ip=ip))
        else:
            say(t("peer.fingerprint", name=name, ip=ip, label=store.key_label(key), code=store.fingerprint(own, key)))
    return name


def receive_messages(server, chat):
    """Accept TCP connections. Each connection carries one message."""
    server.settimeout(LOOP_POLL_SECONDS)
    open_counts = {}  # sender address -> connections open now
    lock = threading.Lock()

    def handle(conn, ip):
        try:
            receive_one(conn, chat)
        finally:
            with lock:
                open_counts[ip] -= 1
                if not open_counts[ip]:
                    del open_counts[ip]

    while not STOP.is_set():
        try:
            conn, addr = server.accept()
        except socket.timeout:
            continue
        except OSError:
            return  # 서버 소켓이 닫혔다 (종료 중)
        ip = addr[0]
        # Connections from unknown addresses, over the rate limit or over the open-connection limits are closed unread.
        if not chat.admit_sender(ip):
            conn.close()
            continue
        with lock:
            if sum(open_counts.values()) >= MAX_CONNECTIONS or open_counts.get(ip, 0) >= MAX_PER_IP:
                conn.close()
                continue
            open_counts[ip] = open_counts.get(ip, 0) + 1
        threading.Thread(target=handle, args=(conn, ip), daemon=True).start()


def clean_text(value, limit):
    """Replace control characters with spaces and cut to limit characters."""
    return "".join(char if char.isprintable() else " " for char in value)[:limit]


def peer_name(value):
    """입장 알림으로 받은 피어 이름. 메시지 보낸 사람 이름과 같은 규칙 (제어 문자 공백, 32자) 으로 정리하고,
    정리 후 비면 unknown 을 쓴다. 이름 길이 규칙 자체는 여기서 정하지 않는다."""
    if not isinstance(value, str):
        return "unknown"
    name = clean_text(value, 32)
    return name if name.strip() else "unknown"


def squeeze_spaces(value):
    """Collapse each run of spaces to one space."""
    return re.sub(" +", " ", value)


def sender_name(value):
    """A sender name as shown: cleaned, spaces squeezed, trimmed, and "unknown" when nothing is left."""
    return squeeze_spaces(clean_text(value, 32)).strip() or "unknown"


def plain_sender(value):
    """The sender of a plain message. Colons are removed, so the name cannot pass for the start of the text."""
    return sender_name(clean_text(value, 32).replace(":", "").replace("：", ""))


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


def receive_one(conn, chat):
    """A plain message starts with '{'. Anything else is a Noise handshake."""
    with conn:
        conn = _Deadline(conn, CONNECTION_SECONDS)
        try:
            first = conn.recv_exact(1)
            if first == b"{":
                if not chat.plaintext:
                    return
                data, key = _read_plain(conn, first), None
            else:
                data, key = _receive_encrypted(conn, first, chat)
        except (OSError, EOFError, ValueError, noise.NoiseError):
            return
    deliver(data, key, chat)


def _read_plain(conn, first):
    data = first
    while len(data) <= MAX_MESSAGE_BYTES:
        chunk = conn.recv(4096)
        if not chunk:
            break
        data += chunk
    if len(data) > MAX_MESSAGE_BYTES:
        raise ValueError("message too large")
    return data


def _receive_encrypted(conn, first, chat):
    """Noise IK responder. The handshake shows the sender's key before we answer, so an unverified sender gets no
    answer at all. Returns the message and the verified key."""
    secret = chat.me.get("secret")
    if secret is None:
        raise ValueError("no identity")
    hello = conn.recv_exact(int.from_bytes(first + conn.recv_exact(1), "big"))
    handshake = noise.Responder(secret)
    handshake.read_message1(hello)
    sender = handshake.remote_static
    if not chat.trusts(sender):
        raise ValueError("sender is not verified")
    conn.write_frame(handshake.write_message2())
    return handshake.recv.decrypt(conn.read_frame()), sender


def deliver(data, key, chat):
    """Show one message. With a key, the id must be that key's id. Without one, the id must not be a key-derived id."""
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
        say(f"{name if name is not None else sender_name(sender)}: {squeeze_spaces(' '.join(rows))}")
        return
    # Plain: no key to check. Every row carries the plain sender's label, so no row can pass for another sender.
    label = f"{plain_sender(sender)} ({t('chat.plain')})"
    for row in rows:
        say(f"{label}: {row}")


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
        except ConnectionResetError:
            pass  # the peer closed first (e.g. it dropped a plain message); the data was already sent


def _send_encrypted(conn, me, peer_key, message):
    """Noise IK initiator: handshake with the peer's key, then one encrypted message."""
    handshake = noise.Initiator(me["secret"], peer_key)
    conn.write_frame(handshake.write_message1())
    handshake.read_message2(conn.read_frame())
    conn.write_frame(handshake.send.encrypt(message))


# ---------------------------------------------------------------- 제어 채널
# Rust 판 (control.rs) 과 같은 형식: 127.0.0.1 위의 줄 단위 JSON. 첫 줄은 {"token": ...}, 이후 요청마다 한 줄 답.
# 요청: {"cmd": "lines"|"peers"|"send"|"shutdown"}. port 와 token 은 상태 폴더의 control-<pid>.json 에 적는다.

CONTROL_MAX_REQUEST_BYTES = 64 * 1024
CONTROL_MAX_CLIENTS = 4
CONTROL_IDLE_SECONDS = 300
CONTROL_LINES_SHOWN = 300


def state_dir():
    """Rust 판 state_dir() 과 같은 순서: HERDR_PLUGIN_STATE_DIR, 다음 LOCALAPPDATA/herdr-talk, 다음 ~/.local/state/herdr-talk."""
    folder = os.environ.get("HERDR_PLUGIN_STATE_DIR")
    if folder:
        return folder
    base = os.environ.get("LOCALAPPDATA") or os.path.join(os.environ.get("HOME", ""), ".local", "state")
    return os.path.join(base, "herdr-talk")


def control_answer(chat, request, allow_shutdown):
    cmd = request.get("cmd")
    if cmd == "lines":
        with chat.lines_lock:
            rows = list(chat.lines[-CONTROL_LINES_SHOWN:])
        return {"ok": True, "lines": [{"text": text, "mine": mine} for text, mine in rows]}
    if cmd == "peers":
        peers = [{"name": info["name"], "ip": info["ip"], "port": info["port"]} for info in chat.fresh_peers().values()]
        return {"ok": True, "peers": peers, "me": {"name": chat.me["name"], "port": chat.me["port"]}}
    if cmd == "send":
        text = request.get("text")
        if not isinstance(text, str):
            return {"ok": False, "error": "text missing"}
        chat.handle(text)  # same path as typing: commands and plain messages both work
        return {"ok": True}
    if cmd == "shutdown" and allow_shutdown:
        STOP.set()
        return {"ok": True}
    return {"ok": False, "error": "unknown cmd"}


def _read_request(reader):
    """One request line as a dict. None when the line is too long, not JSON, or the connection closed."""
    line = reader.readline(CONTROL_MAX_REQUEST_BYTES + 1)
    if not line.endswith(b"\n"):
        return None
    try:
        request = json.loads(line.decode("utf-8"))
    except ValueError:
        return None
    return request if isinstance(request, dict) else None


def _control_client(conn, chat, token, allow_shutdown):
    with conn:
        conn.settimeout(CONTROL_IDLE_SECONDS)
        reader = conn.makefile("rb")

        def reply(value):
            conn.sendall((json.dumps(value, ensure_ascii=False) + "\n").encode("utf-8"))

        try:
            first = _read_request(reader)
            if first is None:
                return
            given = first.get("token")
            if not isinstance(given, str) or not hmac.compare_digest(given.encode("utf-8"), token.encode("utf-8")):
                reply({"ok": False, "error": "bad token"})
                return
            reply({"ok": True})
            while True:
                request = _read_request(reader)
                if request is None:
                    return
                reply(control_answer(chat, request, allow_shutdown))
        except OSError:
            return


class Control:
    """Owns the control file and the listener; close() removes the file."""

    def __init__(self, listener, file):
        self.listener = listener
        self.file = file

    def close(self):
        self.listener.close()
        try:
            os.remove(self.file)
        except OSError:
            pass


def _process_alive(pid):
    """Whether a process with this pid exists. Windows asks the OS; elsewhere os.kill(pid, 0) (which only probes on POSIX)."""
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel32.OpenProcess.restype = wintypes.HANDLE
        handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not handle:
            return ctypes.get_last_error() == 5  # ERROR_ACCESS_DENIED: it exists, but belongs to someone else
        try:
            code = wintypes.DWORD()
            if not kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
                return True
            return code.value == 259  # STILL_ACTIVE
        finally:
            kernel32.CloseHandle(handle)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except OSError:
        return True
    return True


def remove_stale_control_files(state):
    """Delete control-<pid>.json of nodes whose process has ended. A live node keeps its file.
    Best effort: a file that cannot be removed is skipped, so this never stops a start."""
    try:
        names = os.listdir(state)
    except OSError:
        return
    for name in names:
        if not (name.startswith("control-") and name.endswith(".json")):
            continue
        try:
            pid = int(name[len("control-"):-len(".json")])
        except ValueError:
            continue
        if pid == os.getpid() or _process_alive(pid):
            continue
        try:
            os.remove(os.path.join(state, name))
        except OSError:
            pass


def start_control(chat, state, allow_shutdown=False):
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    token = secrets.token_hex(32)
    os.makedirs(state, mode=0o700, exist_ok=True)
    remove_stale_control_files(state)
    file = os.path.join(state, f"control-{os.getpid()}.json")
    try:
        os.remove(file)  # a leftover file of a reused pid is replaced, not reopened
    except FileNotFoundError:
        pass
    body = json.dumps({"port": listener.getsockname()[1], "token": token, "pid": os.getpid()})
    # The token is a credential: only this user may read the file (0o600 on Unix; on Windows the profile folder's ACL applies).
    with os.fdopen(os.open(file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w", encoding="utf-8") as out:
        out.write(body)
    active = threading.BoundedSemaphore(CONTROL_MAX_CLIENTS)

    def accept_loop():
        listener.settimeout(LOOP_POLL_SECONDS)
        while not STOP.is_set():
            try:
                conn, _ = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return  # closed (ending)
            if not active.acquire(blocking=False):
                conn.close()
                continue

            def serve_client(conn=conn):
                try:
                    _control_client(conn, chat, token, allow_shutdown)
                finally:
                    active.release()

            threading.Thread(target=serve_client, daemon=True).start()

    threading.Thread(target=accept_loop, daemon=True).start()
    return Control(listener, file)


def leave_when_idle(chat):
    """Exit after IDLE_LIMIT_SECONDS without activity. The full-screen mode is undone first, so the shell is left clean."""
    while True:
        time.sleep(10)
        if time.time() - chat.last_activity >= IDLE_LIMIT_SECONDS:
            if _restore_screen is not None:
                _restore_screen()
            print(t("idle.left", minutes=IDLE_LIMIT_SECONDS // 60), flush=True)
            os._exit(0)


def enable_vt():
    """Turn on ANSI escape handling in the Windows console (no-op elsewhere)."""
    if os.name == "nt":
        os.system("")


def char_width(text):
    """Terminal columns used by text. East Asian wide characters count as two."""
    return sum(2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1 for ch in text)


def read_char():
    """Block until one key is typed and return it as text. Windows reads keys; elsewhere reads one UTF-8 character."""
    if os.name == "nt":
        import msvcrt
        ch = msvcrt.getwch()
        if ch in ("\x00", "\xe0"):   # arrow and function keys send a second code; drop it
            msvcrt.getwch()
            return ""
        return ch
    fd = sys.stdin.fileno()
    first = os.read(fd, 1)
    need = 4 if first[0] >= 0xF0 else 3 if first[0] >= 0xE0 else 2 if first[0] >= 0xC0 else 1
    data = first + (os.read(fd, need - 1) if need > 1 else b"")
    return data.decode("utf-8", "ignore")


def wrap_line(line, cols):
    """Split one message into rows of at most cols columns. Breaks at spaces; a word longer than a row is cut by character."""
    rows, current = [], ""
    for word in (line or " ").split(" "):
        candidate = word if not current else current + " " + word
        if char_width(candidate) <= cols:
            current = candidate
            continue
        if current:
            rows.append(current)
        chunk, width = "", 0
        for ch in word:
            w = char_width(ch)
            if width + w > cols:
                rows.append(chunk)
                chunk, width = "", 0
            chunk += ch
            width += w
        current = chunk
    rows.append(current)
    return rows


HELP_LINES = [
    t("help.title_send"),
    t("help.send"),
    "",
    t("help.commands"),
    t("help.name"),
    t("help.lang"),
    t("help.codes"),
    t("help.peers"),
    t("help.trust"),
    t("help.phone"),
    t("help.help"),
    t("help.quit"),
    "",
    t("help.keys"),
    t("help.enter"),
    t("help.backspace"),
    t("help.ctrl_l"),
    t("help.ctrl_p"),
    t("help.ctrl_c"),
]
HELP_WIDTH = max(char_width(line) for line in HELP_LINES)
HELP_BORDER = "+" + "-" * (HELP_WIDTH + 2) + "+"
# Pad by display width, so wide characters (Korean, Japanese) keep the right edge straight.
HELP_BOX = [HELP_BORDER] + [f"| {line}{' ' * (HELP_WIDTH - char_width(line))} |" for line in HELP_LINES] + [HELP_BORDER]


def fit(text, cols):
    """The start of text that fits in cols terminal columns."""
    out, width = "", 0
    for ch in text:
        width += char_width(ch)
        if width > cols:
            break
        out += ch
    return out


def tail(text, cols):
    """The end of text that fits in cols columns, so the cursor stays visible."""
    out, width = "", 0
    for ch in reversed(text):
        width += char_width(ch)
        if width > cols:
            break
        out = ch + out
    return out


def screen_frame(lines, status, typed, cols, rows, overlay=None):
    """The whole chat screen as one string of ANSI codes, laid out like the Rust edition (screen.rs):
    a grey status row, the messages, a divider, and the input row at the bottom.
    Messages go from the top; mine are bold blue. Pure, so tests can check the layout."""
    cols, rows = max(20, cols), max(5, rows)
    area = rows - 3
    wrapped = [(row, mine) for line, mine in lines for row in wrap_line(line, cols - 1)]
    visible = wrapped[max(0, len(wrapped) - area):]
    status = fit(status, cols)
    out = ["\x1b[?25l", f"\x1b[1;1H\x1b[100m\x1b[37m{status}{' ' * (cols - char_width(status))}\x1b[0m"]
    for index in range(area):
        out.append(f"\x1b[{index + 2};1H\x1b[2K")
        if index < len(visible):
            row, mine = visible[index]
            out.append(f"\x1b[1m\x1b[34m{row}\x1b[0m" if mine else row)
    # The phone address and token, over the bottom rows of the messages. They are not chat lines.
    shown = (overlay or [])[:area]
    for index, text in enumerate(shown):
        text = fit(text, cols)
        out.append(f"\x1b[{area + 2 - len(shown) + index};1H\x1b[42m\x1b[97m{text}{' ' * (cols - char_width(text))}\x1b[0m")
    out.append(f"\x1b[{rows - 1};1H\x1b[90m{'-' * cols}\x1b[0m")
    out.append(f"\x1b[{rows};1H\x1b[2K> {tail(typed, cols - 3)}\x1b[?25h")
    return "".join(out)


def _enter_screen():
    """Switch to the alternate screen with the cursor hidden. On POSIX also read keys one at a time (raw mode).
    Windows reads keys with msvcrt and needs no mode change. Returns the function that undoes all of it."""
    enable_vt()
    sys.stdout.write("\x1b[?1049h\x1b[?25l")
    sys.stdout.flush()
    saved = None
    if os.name != "nt":
        import termios
        import tty
        fd = sys.stdin.fileno()
        saved = termios.tcgetattr(fd)
        tty.setraw(fd)
    done = []

    def leave():
        if done:
            return
        done.append(True)
        sys.stdout.write("\x1b[0m\x1b[?25h\x1b[?1049l")
        sys.stdout.flush()
        if saved is not None:
            import termios
            termios.tcsetattr(sys.stdin.fileno(), termios.TCSADRAIN, saved)

    return leave


def on_key(ch, chat, typed):
    """Apply one key to the typed text (a one-item list, changed in place). The keys are the Rust edition's.
    Return False to leave the chat."""
    if ch != "\x10":             # the phone address hides on the first key that is not its own toggle
        chat.hide_phone()
    if ch in ("\x03", "\x04"):    # Ctrl+C, Ctrl+D
        return False
    if ch == "\x10":             # Ctrl+P: show or hide the phone address and token
        chat.toggle_phone()
        return True
    if ch == "\x0c":              # Ctrl+L: clear the messages on screen
        chat.clear_lines()
    elif ch == "\x0e":            # Ctrl+N: start a rename
        typed[0] = "/name "
    elif ch in ("\r", "\n"):      # Enter: send, or run a command
        text, typed[0] = typed[0], ""
        return chat.handle(text)
    elif ch in ("\x08", "\x7f"):  # Backspace
        typed[0] = typed[0][:-1]
    elif ch and ch.isprintable():
        typed[0] += ch
    return True


def run_ansi(chat):
    """The one chat screen, drawn with ANSI codes and the standard library only, so it looks the same on every console."""
    global _sink, _restore_screen
    leave = _enter_screen()
    typed = [""]
    screen_lock = threading.Lock()

    def redraw():
        size = shutil.get_terminal_size((80, 24))
        status = t("status.line", name=chat.me["name"], peers=len(chat.fresh_peers()))
        if chat.is_host():
            status = f"{t('host.badge')} {status}"
        with chat.lines_lock:
            lines = list(chat.lines)
        frame = screen_frame(lines, status, typed[0], size.columns, size.lines, chat.phone_overlay())
        with screen_lock:
            sys.stdout.write(frame)
            sys.stdout.flush()

    def add(line, mine=False):
        chat.add_line(line, mine)
        redraw()

    with _sink_lock:
        _sink = add
    chat.on_phone_change = redraw
    _restore_screen = leave
    redraw()
    threading.Thread(target=leave_when_idle, args=(chat,), daemon=True).start()
    try:
        while on_key(read_char(), chat, typed):
            redraw()
    finally:
        chat.on_phone_change = None
        with _sink_lock:
            _sink = None
        _restore_screen = None
        leave()
    say(t("bye"))
    return 0


def herdr_cli(*args):
    completed = subprocess.run(
        [os.environ.get("HERDR_BIN_PATH", "herdr"), *args],
        stdin=subprocess.DEVNULL, capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    if completed.returncode != 0:
        raise RuntimeError((completed.stdout + completed.stderr).strip() or f"herdr {' '.join(args)} failed")
    output = completed.stdout.strip().lstrip("﻿")
    return json.loads(output).get("result", {}) if output else {}


def open_chat(_args):
    """현재 pane 아래에 pane 을 나눠 chat 을 실행한다. 대화가 끝나면(Ctrl+C) 그 셸 프롬프트가 남는다."""
    anchor = os.environ.get("HERDR_PANE_ID")
    if not anchor:
        print(t("open.no_pane"), file=sys.stderr)
        return 1
    src_dir = os.path.dirname(os.path.abspath(__file__))
    try:
        pane_id = herdr_cli("pane", "split", anchor, "--direction", "down", "--cwd", src_dir)["pane"]["pane_id"]
        herdr_cli("pane", "run", pane_id, sys.executable, "-X", "utf8", os.path.abspath(__file__), "run")
    except (RuntimeError, ValueError, KeyError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


def start_node(args, state=None, plaintext=None):
    """Bind the sockets and start the network threads. Returns (server, chat).
    Keys and trusted peers live in the state folder (shared with the Rust edition); plain messages follow config.json."""
    global _sink
    state = state or state_dir()
    public, secret = store.load_or_create_identity(state)
    trusted = store.read_trusted(state)
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("", args.tcp_port))
    server.listen()
    name = args.name or default_name()
    me = {"id": store.peer_id(public), "name": name, "port": server.getsockname()[1], "key": public, "secret": secret}

    chat = Chat(me, plaintext=store.plaintext_allowed() if plaintext is None else plaintext)
    chat.trusted = trusted
    chat.state_dir = state
    # The first node on this PC is the host (방장). Without --name it is called that; with one, its name stays and the status row shows a badge.
    if chat.try_become_host() and not args.name:
        me["name"] = t("host.name")
    with _sink_lock:
        _sink = chat.add_line
    threading.Thread(target=receive_messages, args=(server, chat), daemon=True).start()
    threading.Thread(target=announce, args=(args.udp_port, me), daemon=True).start()
    threading.Thread(target=listen_for_peers, args=(args.udp_port, chat), daemon=True).start()
    return server, chat


def serve(args):
    """Chat without a screen, for a GUI: only the control channel runs. Ends on Ctrl+C or a `shutdown` request."""
    server, chat = start_node(args)
    control = start_control(chat, state_dir(), allow_shutdown=True)
    chat.watch_host()
    STOP.wait(ANNOUNCE_SECONDS + 0.5)
    renamed = resolve_name_clash(chat)
    if renamed:
        say(t("name.changed", old=renamed[0], new=renamed[1]))
    say(t("start.hello", name=chat.me["name"], port=chat.me["port"]))
    try:
        while not STOP.wait(1):
            pass
    except KeyboardInterrupt:
        pass
    finally:
        STOP.set()
        chat.release_host()
        control.close()
        server.close()
    return 0


def run(args):
    server, chat = start_node(args)
    me = chat.me
    control = start_control(chat, state_dir())
    chat.watch_host()  # the first node on this PC owns the phone address; another takes over when it ends

    if args.dock:
        herdr_api.try_request("pane.report_agent", {"pane_id": args.pane, "source": SOURCE, "agent": AGENT_ID, "state": "idle"})
        threading.Thread(target=keep_dock_block, args=(args.pane, chat), daemon=True).start()
    # Wait one announce cycle so peers that are already here are known before the name is final.
    STOP.wait(ANNOUNCE_SECONDS + 0.5)
    renamed = resolve_name_clash(chat)
    if renamed:
        say(t("name.changed", old=renamed[0], new=renamed[1]))
    say(t("start.hello", name=me["name"], port=me["port"]))
    try:
        return run_ansi(chat)
    finally:
        STOP.set()
        chat.release_host()
        control.close()
        server.close()
        if args.dock:
            herdr_api.try_request("pane.release_agent", {"pane_id": args.pane, "source": SOURCE, "agent": AGENT_ID})
            dock.release_dock_pane(DOCK_KEY)


# ---------------------------------------------------------------- herdr-dock 블록
# 공용 herdr-dock workspace 의 한 칸에 talk 대화창을 두고, 접속자 수를 에이전트 목록에 보여 준다.
# 블록을 클릭하면 그 pane 이 포커스되고 대화창이 그대로 보인다. 다른 dock plugin 과 자리 규칙을 맞춘다.

DOCK_KEY = "herdr-talk-python"
DOCK_REPORT_SECONDS = 2
DOCK_TOKEN_TTL_MS = 15_000
AGENT_ID = "talk"
SOURCE = "plugin:herdr-talk-python"


def keep_dock_block(pane_id, chat):
    """블록에 접속자 수를 계속 알린다. 대화가 끝나 STOP 이 설정되면 멈춘다."""
    while not STOP.is_set():
        herdr_api.try_request("pane.report_metadata", {
            "pane_id": pane_id,
            "source": SOURCE,
            "display_agent": t("dock.agent_name", count=len(chat.fresh_peers())),
            "tokens": {},
            "ttl_ms": DOCK_TOKEN_TTL_MS,
        })
        STOP.wait(DOCK_REPORT_SECONDS)


def dock_start(_args):
    folder = dock.plugins_folder()
    stored = dock.read_dock_pane(folder, DOCK_KEY)
    agents = herdr_api.cli("agent", "list").get("agents", [])
    if stored and any(a["pane_id"] == stored and a.get("agent") == AGENT_ID for a in agents):
        return 0  # 이미 떠 있다
    _, pane_id = dock.ensure_dock_pane(DOCK_KEY, folder)
    herdr_api.cli("pane", "run", pane_id, sys.executable, "-X", "utf8", os.path.abspath(__file__),
                  "run", "--dock", "--pane", pane_id)
    return 0


def dock_stop(_args):
    folder = dock.plugins_folder()
    pane_id = dock.read_dock_pane(folder, DOCK_KEY)
    if pane_id:
        herdr_api.try_request("pane.release_agent", {"pane_id": pane_id, "source": SOURCE, "agent": AGENT_ID})
        workspace_id = dock.dock_workspace_id()
        others = [p for p in dock.workspace_panes(workspace_id) if p["pane_id"] != pane_id and not dock.is_sidebar(p)] if workspace_id else []
        if others:
            herdr_api.cli("pane", "close", pane_id, check=False)
        elif workspace_id:
            herdr_api.cli("workspace", "close", workspace_id, check=False)
    dock.release_dock_pane(DOCK_KEY)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(prog="herdr-talk")
    commands = parser.add_subparsers(dest="command", required=True)
    chat = commands.add_parser("run", help="open the chat in this pane")
    chat.add_argument("--name", default=None, help="name shown to others (random if omitted)")
    chat.add_argument("--tcp-port", type=int, default=0, help="TCP port for messages (0 = pick one)")
    chat.add_argument("--udp-port", type=int, default=None, help="discovery port (keep the same on every PC; default HERDR_TALK_UDP_PORT or 47850)")
    chat.add_argument("--dock", action="store_true", help="show this pane as the herdr-dock block")
    chat.add_argument("--pane", default=None, help="herdr pane id of the dock block (with --dock)")
    serve_parser = commands.add_parser("serve", help="run without a screen; a GUI talks to it over the control channel")
    serve_parser.add_argument("--name", default=None, help="name shown to others (random if omitted)")
    serve_parser.add_argument("--tcp-port", type=int, default=0, help="TCP port for messages (0 = pick one)")
    serve_parser.add_argument("--udp-port", type=int, default=None, help="discovery port (keep the same on every PC; default HERDR_TALK_UDP_PORT or 47850)")
    commands.add_parser("open", help="split the current pane and run the chat in it")
    commands.add_parser("dock-start", help="show the chat as a block in the herdr-dock workspace")
    commands.add_parser("dock-stop", help="remove the dock block")
    args = parser.parse_args(argv)
    try:
        return _dispatch(args)
    except (OSError, ValueError) as error:  # e.g. a damaged identity.json: say so, never make a new key
        print(error, file=sys.stderr)
        return 1


def _dispatch(args):
    if args.command in ("run", "serve") and args.udp_port is None:
        args.udp_port = udp_port()  # a bad HERDR_TALK_UDP_PORT is reported by main() and stops the start
    if args.command == "run":
        return run(args)
    if args.command == "serve":
        return serve(args)
    if args.command == "open":
        return open_chat(args)
    if args.command == "dock-start":
        return dock_start(args)
    if args.command == "dock-stop":
        return dock_stop(args)
    return 1


if __name__ == "__main__":
    sys.exit(main())
