"""The chat state (Chat), the peer rules (remember_peer, the limits), the reveal exchange and the receiving side of the
LAN network. The network functions it uses are in core/net.py. talk.py re-exports the names.
"""

import json
import math
import os
import socket
import threading
import time
from concurrent.futures import ThreadPoolExecutor

from core import noise, phone, relay_chat, relay_settings, store
from core.events import Line, Message, Notice, PeerRow, Status
from core.names import NAMES_BY_LANGUAGE, clean_text, peer_name, pick_name, unique_name
from core.net import APP, MAX_TEXT_CHARS, SEND_TIMEOUT_SECONDS, STOP, _Deadline, deliver, is_local_address, send_to


MAX_TOTAL_PEERS = 10  # everyone in the LAN chat (verified or not) besides this PC: a new peer past this many is ignored


PEER_TIMEOUT_SECONDS = 10


MAX_PEERS = 10  # people in a LAN chat, this PC included; only verified ones count: a new peer is ignored once that many others are verified


JOIN_LIMIT_SECONDS = 30  # a LAN peer must be joined (/join) this long after its first announce, or it is kicked for this chat


AUTO_RENAME_GAP_SECONDS = 60  # 자동 이름 변경 사이의 최소 간격


MAX_MESSAGE_BYTES = 64 * 1024


LOOP_POLL_SECONDS = 0.5  # 종료 신호를 이 간격으로 확인한다


SEND_WORKERS = 8  # 메시지를 동시에 보내는 피어 수의 상한


MAX_LINES = 2000


CONNECTION_SECONDS = 3         # total time for one incoming connection: first byte, handshake and message


MAX_CONNECTIONS = 16           # incoming connections handled at once, all addresses together


MAX_PER_IP = 3                 # ... and from one address


MESSAGE_LIMIT = 20             # messages one sender address may deliver within MESSAGE_WINDOW_SECONDS


MESSAGE_WINDOW_SECONDS = 10




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


class Chat:
    """Shared state: this instance, known peers, and the lines shown on screen."""

    on_event = None  # where events go: the screen sets it (extend/screen.py); None drops them

    def __init__(self, me, plaintext=True):
        # me: id, name, port; with an identity also key and secret (bytes). start_node() sets plaintext from config.json
        # (off unless the user turns it on). The default keeps a chat built without an identity, as in tests, plain.
        self.me = me
        self.plaintext = plaintext
        # SAS: one nonce per peer, kept secret until a reveal to that peer. Only its commitment is announced, in
        # me["commits"] = {peer key hex: commitment for that peer} (read by announce, see my_nonce_for).
        self.my_nonces = {}       # peer key -> (nonce, commitment) for that peer
        self.commits = {}         # peer key -> commitment it gave us for us, pinned at first sight (setdefault)
        self.refused = set()      # (peer key, commitment) pairs already reported as refused, so the warning comes once
        self.reveals = {}         # peer key -> nonce it revealed (checked against its commitment)
        self.revealing = set()    # peers whose reveal is being fetched right now (one worker each)
        if me.get("key") is not None:
            me["commits"] = {}
        self.trusted = []         # verified peers: {"name", "key", "ip"}
        self.waiting_since = {}   # key -> time of its first announce, until it is joined (starts the JOIN_LIMIT_SECONDS clock)
        self.kicked = set()       # keys kicked for not joining in time: ignored for the rest of this chat
        self.state_dir = None     # where /trust saves; None keeps it for this session
        self.received = {}        # sender address -> arrival times of recent messages
        self.host = None          # phone.Host while this node is the host (방장) of this PC
        self.help_open = False       # the help box is on screen (F1 or /help); the next key closes it
        self.numbers = {}            # waiting peer's key -> its number ([n]); given once, never reused or renumbered
        self.next_number = 1
        self.received_lock = threading.Lock()
        self.peers = {}
        self.room_names = []  # the other people in the room (CLOUD SERVICE), for the CONNECTED NODES list
        self.lan_listening = True  # run() turns it off until start_relay has set the mode: before that no LAN peer is remembered
        self.peers_lock = threading.Lock()
        self.lines = []
        self.lines_lock = threading.Lock()  # 네트워크 스레드와 화면이 같은 목록을 쓴다
        self.last_activity = time.time()
        self.last_auto_rename = None  # 자동 이름 변경 시각 (time.monotonic)
        self.supabase = False     # connection mode: True sends and shows chat only through the relay (no LAN chat)
        self.relay = None         # relay_chat.Relay while the relay runs
        self.relay_status = ""    # short text for the status row, empty when the relay is off

    def emit(self, event):
        """Report an event (core/events.py) to whoever shows it: on_event. None drops it."""
        if self.on_event is not None:
            self.on_event(event)

    def touch(self):
        """Mark activity so the idle timer restarts. Used by every view, not only add_line()."""
        self.last_activity = time.time()

    def add_line(self, line, mine=False, sender=None):
        self.touch()
        stamp = time.strftime("%H:%M")  # the time the line first shows; it is not stored anywhere else
        with self.lines_lock:
            self.lines.append((line, mine, sender, stamp))
            del self.lines[:-MAX_LINES]

    def clear_lines(self):
        with self.lines_lock:
            self.lines.clear()

    def fresh_peers(self):
        cutoff = time.time() - PEER_TIMEOUT_SECONDS
        with self.peers_lock:
            fresh = {pid: info for pid, info in self.peers.items() if info["seen"] >= cutoff}
        return {pid: info for pid, info in fresh.items() if info.get("key") is None or not self.is_kicked(info["key"], info["name"], info["ip"])}

    def join_seconds_left(self, key):
        """Whole seconds left before a waiting key is kicked, or None when it is not waiting (joined, or no clock yet)."""
        since = self.waiting_since.get(key)
        if since is None or self.trusts(key):
            return None
        return max(0, math.ceil(JOIN_LIMIT_SECONDS - (time.time() - since)))

    def rejoin_kicked(self):
        """/rejoin: forget the kicked keys, so the next announce finds them again with a new 60-second clock. Verified peers
        are not touched (they are never kicked)."""
        for key in self.kicked:
            self.waiting_since.pop(key, None)
        self.kicked.clear()

    def is_kicked(self, key, name, ip):
        """True for a kicked key: not joined within JOIN_LIMIT_SECONDS (kicked for the rest of this chat). A joined key is never
        kicked. The first kick says so once in the chat. CLOUD SERVICE has no LAN chat: nobody is kicked there."""
        if self.supabase or self.trusts(key):
            return False
        if key in self.kicked:
            return True
        since = self.waiting_since.get(key)
        if since is None or time.time() - since < JOIN_LIMIT_SECONDS:
            return False
        self.kicked.add(key)
        self.emit(Notice("peer.kicked", {"name": name, "ip": ip, "seconds": JOIN_LIMIT_SECONDS}))
        return True

    def is_host(self):
        """Supabase: this PC's node is the room's owner (who created it). LAN: this PC holds the phone host lock."""
        if self.supabase:
            relay = self.relay
            return relay is not None and relay.owner_pk is not None and relay.owner_pk == relay.own_pk
        return self.host is not None

    def try_become_host(self):
        """Become the host (방장) of this PC if no node on this PC is."""
        if self.state_dir is None or STOP.is_set() or self.host is not None:
            return False
        try:
            host = phone.Host.take(self.state_dir, self.me["port"])
        except OSError:
            return False
        if host is None:
            return False
        self.host = host
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

    def my_nonce_for(self, key):
        """This node's nonce for one peer: made once per peer, never shared with another peer. Its commitment is put in
        the announce (me["commits"]) for that peer, so the peer can check the reveal later."""
        entry = self.my_nonces.get(key)
        if entry is None:
            nonce = os.urandom(32)
            entry = (nonce, store.commit(self.me["key"], nonce))
            self.my_nonces[key] = entry
            self.me["commits"][store.to_hex(key)] = store.to_hex(entry[1])
        return entry[0]

    def sas_for(self, key):
        """The code for a peer, once both nonces are known (after the reveal exchange). None before that."""
        reveal = self.reveals.get(key)
        if reveal is None or self.me.get("key") is None or key not in self.my_nonces:
            return None
        return store.sas_code(self.me["key"], self.my_nonces[key][0], key, reveal)

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
        """Verify a waiting LAN peer: by its number ([n], from /trust n) or by its code (/trust CODE)."""
        if code.isdigit() and len(code) <= 2:  # a number is 1 or 2 digits; a code is 8 characters, digits or not
            number = int(code)
            key = next((key for key, given in self.numbers.items() if given == number), None)
            if key is None:
                self.emit(Notice("trust.no_number", {"n": code}))
                return
            info = next((info for info in self.fresh_peers().values() if info.get("key") == key), None)
            if info is None or self.link_of(info) != "unverified":
                self.emit(Notice("trust.already", {"n": code}))
                return
            self.trust_info(info)
            return
        own = self.me.get("key")
        found = []
        for info in self.fresh_peers().values():
            key = info.get("key")
            sas = self.sas_for(key) if key is not None else None
            if own is not None and sas is not None and self.link_of(info) == "unverified" \
                    and store.normalize_code(sas) == code:
                found.append(info)
        # Two different keys with the same code would mean a collision or an attacker: refuse rather than pick one.
        if len({info["key"] for info in found}) > 1:
            self.emit(Notice("trust.ambiguous", {"code": code}))
            return
        if not found:
            self.emit(Notice("trust.none", {"code": code}))
            return
        self.trust_info(found[0])

    def waiting_number(self, key):
        """The number of a waiting peer: given once, the first time it is shown, and kept until it is trusted."""
        if key not in self.numbers:
            self.numbers[key] = self.next_number
            self.next_number += 1
        return self.numbers[key]

    def trust_info(self, info):
        """Remember a verified peer's key and name (saved when the node has a state folder), and say so."""
        self.trusted.append({"name": info["name"], "key": info["key"], "ip": info["ip"]})
        self.numbers.pop(info["key"], None)  # the others keep their numbers
        try:
            if self.state_dir is not None:
                store.write_trusted(self.state_dir, self.trusted)
        except OSError as error:
            self.emit(Notice("trust.save_failed", {"error": str(error)}))
            return
        self.emit(Notice("trust.done", {"name": info["name"], "ip": info["ip"]}))

    def start_relay(self):
        """(Re)start the relay for the saved settings and set the connection mode. Returns the status text."""
        self.stop_relay()
        settings = relay_settings.effective(self.state_dir) if self.state_dir else relay_settings.empty()
        self.supabase = settings["mode"] == relay_settings.SUPABASE
        if self.supabase:  # LAN peers heard while the settings were open (before the mode was known) are dropped
            with self.peers_lock:
                self.peers.clear()
            self.waiting_since.clear()
            self.kicked.clear()
        self.lan_listening = True
        secret, key = self.me.get("secret"), self.me.get("key")
        self.relay_status = ""
        if not self.supabase or secret is None or key is None:
            return ""
        self.relay, self.relay_status = relay_chat.start(RelayHooks(self), self.state_dir, settings)
        if self.relay_status:
            self.emit(Line(self.relay_status))
        return self.relay_status

    def stop_relay(self):
        self.room_names = []
        relay, self.relay = self.relay, None
        if relay is not None:
            relay.stop()

    def _send_over_relay(self, text):
        if self.relay is None:
            self.emit(Notice("relay.not_sent"))
            return True
        self.relay.send(text)  # the relay thread shows the line once it is sent (Message, as on the LAN)
        return True

    def handle(self, text):
        """Handle one line typed by the user. Return False to leave the chat."""
        text = text.strip()
        if not text:
            return True
        # Commands are echoed before they run, as in the Rust edition. Chat messages are not echoed.
        if text.startswith("/"):
            self.emit(Line(f"> {text}"))
        if text == "/quit":
            return False
        if text == "/help":
            self.help_open = True  # the help box is drawn over the messages until a key is pressed
            return True
        if text == "/name" or text.startswith("/name "):
            new_name = text[5:].strip()[:32]
            if not new_name:
                self.emit(Notice("usage.name"))
                return True
            old_name = self.me["name"]
            self.me["name"] = new_name
            self.emit(Notice("name.changed", dict(old=old_name, new=new_name)))
            return True
        if text == "/lang" or text.startswith("/lang "):
            code = text[5:].strip().lower()
            if code not in NAMES_BY_LANGUAGE:
                self.emit(Notice("usage.lang", dict(codes="|".join(NAMES_BY_LANGUAGE))))
                return True
            old_name = self.me["name"]
            taken = {info["name"] for info in self.fresh_peers().values()} | {old_name}
            self.me["name"] = pick_name(NAMES_BY_LANGUAGE[code][1], taken)
            self.emit(Notice("name.changed", dict(old=old_name, new=self.me["name"])))
            return True
        if text == "/peers":
            found = self.fresh_peers()
            if not found:
                self.emit(Notice("peers.none"))
            own = self.me.get("key")
            for info in found.values():
                link = self.link_of(info)
                code = None
                if link in ("encrypted", "plain"):
                    state = "verified" if link == "encrypted" else "plain"
                else:
                    code = self.sas_for(info["key"]) if info.get("key") is not None else None
                    state = "unverified" if code is not None else ("pending" if own is not None else "")
                number = self.numbers.get(info.get("key"))
                prefix = f"[{number}] " if number is not None else ""
                self.emit(PeerRow(prefix, clean_text(info["name"], 32), info["ip"], info["port"], state, code))
            return True
        if text == "/rejoin":
            self.rejoin_kicked()
            self.emit(Notice("rejoin.done", {"seconds": JOIN_LIMIT_SECONDS}))
            return True
        if text == "/join" or text.startswith("/join "):
            code = store.normalize_code(text[5:])
            if code:
                self.trust(code)
            else:
                self.emit(Notice("usage.trust"))
            return True
        text = text[:MAX_TEXT_CHARS].replace("\n", " ")
        if self.supabase:
            return self._send_over_relay(text)
        found = self.fresh_peers()
        if not found:
            self.emit(Notice("peers.none_hint"))
            return True
        # Only peers this link allows get the message. Each skipped peer says why.
        targets = {}
        for pid, info in found.items():
            link = self.link_of(info)
            if link == "encrypted" or (link == "plain" and self.plaintext):
                targets[pid] = info
            elif link == "plain":
                self.emit(Notice("peer.plain_off", {"name": info["name"]}))
            else:
                self.emit(Notice("peer.unverified", {"name": info["name"]}))
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
                    self.emit(Notice("peer.unreachable", {"name": info["name"]}))
        self.emit(Message(self.me["name"], text, mine=True))
        return True


class RelayHooks:
    """What relay_chat needs from the chat (relay_chat does not import this module)."""

    def __init__(self, chat):
        self.chat = chat

    def emit(self, event):
        if isinstance(event, Status):
            self.chat.relay_status = event.text  # the status row reads it at the next redraw
        else:
            self.chat.emit(event)

    def trusts(self, noise_key):
        return self.chat.trusts(noise_key)

    def trusted_name(self, noise_key):
        return self.chat.trusted_name(noise_key)

    def my_name(self):
        return self.chat.me["name"]

    def members(self, names):
        self.chat.room_names = list(names)


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


SOCKET_ERROR_LIMIT = 50  # socket errors in a row before a receive loop gives up (a Windows reset or an oversized datagram is one)


def _socket_failed(chat, errors):
    """One socket error in a receive loop: wait a moment so a steady error does not spin, or give up after too many in a
    row (one notice). True when the loop must end."""
    if errors > SOCKET_ERROR_LIMIT:
        chat.emit(Notice("net.socket_failed"))
        return True
    STOP.wait(LOOP_POLL_SECONDS)
    return False


def _listen_loop(sock, chat):
    errors = 0
    while not STOP.is_set():
        try:
            data, addr = sock.recvfrom(4096)
        except socket.timeout:
            continue
        except OSError:  # a closed socket also lands here; STOP is set then, so the loop ends
            errors += 1
            if STOP.is_set() or _socket_failed(chat, errors):
                return
            continue
        errors = 0
        try:
            info = json.loads(data.decode("utf-8"))
        except ValueError:
            continue
        # Only a well-formed announcement is kept: an int port in range and a string id.
        # Anything else could put odd values on screen (via /peers) or crash this thread.
        if not isinstance(info, dict) or info.get("app") != APP:
            continue
        if chat.supabase or not chat.lan_listening:  # CLOUD SERVICE has no LAN chat, and before the first-run choice the mode is unknown
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
                chat.emit(Notice("name.changed", {"old": renamed[0], "new": renamed[1]}))


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
    announces_commits = "commits" in info  # read before `info` is reused below
    # The commitment (SAS): the one the peer gave us for us. It is pinned at first sight; a different one is refused.
    commit = None
    if key is not None and own is not None:
        commits = info.get("commits")
        raw = commits.get(store.to_hex(own)) if isinstance(commits, dict) else None
        if isinstance(raw, str):
            commit = store.from_hex(raw)
            if commit is None or len(commit) != 32:
                commit = None
    if commit is not None:
        pinned = chat.commits.setdefault(key, commit)
        if pinned != commit:
            if (key, commit) not in chat.refused:
                chat.refused.add((key, commit))
                chat.emit(Notice("peer.key_changed", {"name": peer_name(info.get("name", "unknown")), "ip": ip}))  # shortcut: reused wording
            return None
    if key is not None and own is not None:
        chat.my_nonce_for(key)  # our nonce for this peer; its commitment goes out in the next announce
    # A key the user trusts: an announcement may not rename its entry or move it to another address.
    trusted = key is not None and chat.trusts(key)
    trusted_name = chat.trusted_name(key) if key is not None else None
    name = trusted_name if trusted_name is not None else peer_name(info.get("name", "unknown"))
    # A key kicked for not joining in time stays out for this chat. Otherwise its join clock starts with this announce.
    if key is not None and chat.is_kicked(key, name, ip):
        return None
    if key is not None and not trusted:
        chat.waiting_since.setdefault(key, time.time())
    # At most MAX_PEERS people including this PC, counting only verified peers: a new peer past that is ignored (a known
    # peer's announcements always pass). An unverified peer does not take a place. The MAX_TOTAL_PEERS cap is for unverified
    # and plain entries only, so a verified peer is never shut out by them.
    here = chat.fresh_peers()
    known_peer = any(known.get("id") == peer_id for known in here.values())
    verified = sum(1 for known in here.values() if known.get("key") is not None and chat.trusts(known["key"]))
    full = verified >= MAX_PEERS - 1 or (not trusted and len(here) >= MAX_TOTAL_PEERS)
    if not known_peer and full:
        return None
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
    chat.emit(Notice("peer.joined", {"name": name, "ip": ip}))
    if key is not None and own is not None and not chat.trusts(key):
        # A new key from an address we trusted before is a changed key, not a new peer, unless the trusted key is
        # still announcing from that address: then this is another identity on the same PC.
        alive = chat.fresh_peers().values()
        changed = any(
            peer["ip"] == ip and not any(a["ip"] == ip and a.get("key") == peer["key"] for a in alive)
            for peer in chat.trusted
        )
        if changed:
            chat.emit(Notice("peer.key_changed", {"name": name, "ip": ip}))
        elif commit is not None:
            # The code is shown only after both nonces are known (see reveal_later), never from the announce alone.
            if key not in chat.revealing:
                chat.revealing.add(key)
                threading.Thread(target=reveal_later, args=(chat, key, ip, port), daemon=True).start()
        elif not announces_commits:
            # An old version announces no commitments at all. A peer that has them but none for this PC is a first contact: silent.
            # shortcut: a Korean-only notice; move it to the locales when the wording is settled.
            chat.emit(Notice("peer.old_version", {"name": name, "ip": ip}))
    return name


REVEAL_RETRY_SECONDS = 2


def reveal_message(chat, peer_key):
    """The reveal inside the Noise session: this PC's key and its nonce for that peer (plain JSON, as in the Rust edition)."""
    nonce = chat.my_nonce_for(peer_key)
    return json.dumps({"app": APP, "type": "reveal", "pk": store.to_hex(chat.me["key"]), "nonce": store.to_hex(nonce)}).encode("utf-8")


def read_reveal(chat, expected, payload):
    """Check a reveal from `expected` against its announced commitment. Returns the nonce; ValueError when it does not match."""
    try:
        value = json.loads(payload.decode("utf-8"))
    except ValueError:
        raise ValueError("reveal is not JSON")
    if not isinstance(value, dict) or value.get("app") != APP or value.get("type") != "reveal":
        raise ValueError("not a reveal")
    pk = store.from_hex(value["pk"]) if isinstance(value.get("pk"), str) else None
    if pk != expected:
        raise ValueError("reveal is for another key")
    nonce = store.from_hex(value["nonce"]) if isinstance(value.get("nonce"), str) else None
    if nonce is None or len(nonce) != 32:
        raise ValueError("reveal nonce is not 32 bytes")
    commit = chat.commits.get(expected)
    if commit is None:
        raise ValueError("no commitment from this peer")
    if store.commit(expected, nonce) != commit:
        raise ValueError("reveal does not match the commitment")
    return nonce


def exchange_reveal(chat, ip, port, key):
    """The initiator's half: connect with the peer's key, send our reveal, read the peer's reveal back."""
    with socket.create_connection((ip, port), timeout=SEND_TIMEOUT_SECONDS) as raw:
        conn = _Deadline(raw, SEND_TIMEOUT_SECONDS)
        handshake = noise.Initiator(chat.me["secret"], key)
        conn.write_frame(handshake.write_message1())
        handshake.read_message2(conn.read_frame())
        conn.write_frame(handshake.send.encrypt(reveal_message(chat, key)))
        return read_reveal(chat, key, handshake.recv.decrypt(conn.read_frame()))


def show_code(chat, key):
    """The line with the code for a peer (only once its nonce is known)."""
    code = chat.sas_for(key)
    if code is None:
        return
    with chat.peers_lock:
        found = next((info for info in chat.peers.values() if info.get("key") == key), None)
    name, ip = (found["name"], found["ip"]) if found else ("?", "?")
    number = chat.waiting_number(key)
    # One chat row per line: the code sits on its own indented row. The number is what /trust n takes.
    chat.emit(Notice("peer.fingerprint", {"name": f"[{number}] {name}", "ip": ip, "code": code}))


def reveal_later(chat, key, ip, port):
    """Fetch a peer's reveal in the background, then show its code. Keeps trying while the peer announces (a known peer's
    announce does not ask again); stops when the code is known or the peer is gone."""
    try:
        while True:
            announcing = any(info.get("key") == key for info in chat.fresh_peers().values())
            if STOP.is_set() or chat.sas_for(key) is not None or not announcing:
                break
            try:
                nonce = exchange_reveal(chat, ip, port, key)
            except (OSError, ValueError, noise.NoiseError):
                STOP.wait(REVEAL_RETRY_SECONDS)
                continue
            chat.reveals.setdefault(key, nonce)
            show_code(chat, key)
            break
    finally:
        chat.revealing.discard(key)


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

    errors = 0
    while not STOP.is_set():
        try:
            conn, addr = server.accept()
        except socket.timeout:
            continue
        except OSError:  # a closed server socket lands here too; STOP is set then, so the loop ends
            errors += 1
            if STOP.is_set() or _socket_failed(chat, errors):
                return
            continue
        errors = 0
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
    if data is None:
        return  # a reveal exchange: nothing to show
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
    """Noise IK responder. The handshake shows the sender's key before we answer. An unverified sender is answered only
    to exchange reveals (it must have announced a commitment). Returns (message, verified key), or (None, None) for a reveal."""
    secret = chat.me.get("secret")
    if secret is None:
        raise ValueError("no identity")
    hello = conn.recv_exact(int.from_bytes(first + conn.recv_exact(1), "big"))
    handshake = noise.Responder(secret)
    handshake.read_message1(hello)
    sender = handshake.remote_static
    trusted = chat.trusts(sender)
    if not trusted and sender not in chat.commits:
        raise ValueError("sender is not verified")
    conn.write_frame(handshake.write_message2())
    payload = handshake.recv.decrypt(conn.read_frame())
    if trusted:
        return payload, sender
    nonce = read_reveal(chat, sender, payload)
    if chat.reveals.get(sender, nonce) != nonce:
        # A second, different reveal from the same peer: refuse the connection, with one line of warning.
        with chat.peers_lock:
            found = next((info for info in chat.peers.values() if info.get("key") == sender), None)
        chat.emit(Notice("trust.wrong_code", {"name": found["name"] if found else "?"}))  # shortcut: reused wording
        raise ValueError("a different reveal from this peer was already given")
    if sender not in chat.reveals:
        chat.reveals[sender] = nonce
        show_code(chat, sender)
    # Our reveal (our nonce for this peer) goes back on the same connection, so the initiator gets it too.
    conn.write_frame(handshake.send.encrypt(reveal_message(chat, sender)))
    return None, None
