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
import json
import locale
import os
import random
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
UDP_PORT = 47850          # discovery port, the same on every PC
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
    Renames at most once per AUTO_RENAME_GAP_SECONDS. This edition has no keys, so a peer's id and name cannot
    be verified: anyone on the network can announce a small id and our name to make us rename (the limit only
    slows that down).
    Returns (old, new) when renamed, else None."""
    peers = chat.fresh_peers()
    name = chat.me["name"]
    if not any(info["name"] == name and pid < chat.me["id"] for pid, info in peers.items()):
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

    def __init__(self, me):
        self.me = me
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
            for info in found.values():
                say(f"- {clean_text(info['name'], 32)} ({info['ip']}:{info['port']})")
            return True
        text = text[:MAX_TEXT_CHARS].replace("\n", " ")
        targets = self.fresh_peers()
        if not targets:
            say(t("peers.none_hint"))
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
    """Broadcast this instance's name and TCP port every few seconds."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    try:
        while not STOP.is_set():
            payload = json.dumps({"app": APP, **me}).encode("utf-8")
            try:
                sock.sendto(payload, ("255.255.255.255", udp_port))
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
        if not isinstance(info, dict) or info.get("app") != APP or info.get("id") == chat.me["id"]:
            continue
        peer_id = info.get("id")
        port = info.get("port")
        if not isinstance(peer_id, str) or not peer_id:
            continue
        if not isinstance(port, int) or isinstance(port, bool) or not 1 <= port <= 65535:
            continue
        name = peer_name(info.get("name", "unknown"))
        with chat.peers_lock:
            is_new = peer_id not in chat.peers
            chat.peers[peer_id] = {
                "name": name,
                "ip": addr[0],
                "port": port,
                "seen": time.time(),
            }
        if is_new:
            say(t("peer.joined", name=name, ip=addr[0]))
        if name == chat.me["name"]:
            renamed = resolve_name_clash(chat)
            if renamed:
                say(t("name.changed", old=renamed[0], new=renamed[1]))


def receive_messages(server):
    """Accept TCP connections. Each connection carries one JSON message."""
    server.settimeout(LOOP_POLL_SECONDS)
    while not STOP.is_set():
        try:
            conn, _ = server.accept()
        except socket.timeout:
            continue
        except OSError:
            return  # 서버 소켓이 닫혔다 (종료 중)
        threading.Thread(target=receive_one, args=(conn,), daemon=True).start()


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


def receive_one(conn):
    conn.settimeout(RECEIVE_TIMEOUT_SECONDS)  # 멈춘 연결이 스레드를 잡고 있지 않게 한다
    with conn:
        data = b""
        while len(data) <= MAX_MESSAGE_BYTES:
            chunk = conn.recv(4096)
            if not chunk:
                break
            data += chunk
    if len(data) > MAX_MESSAGE_BYTES:
        return
    try:
        message = json.loads(data.decode("utf-8"))
    except ValueError:
        return
    if not isinstance(message, dict) or message.get("app") != APP:
        return
    sender, body = message.get("from"), message.get("text")
    if not isinstance(sender, str) or not isinstance(body, str):
        return  # 형식이 틀린 메시지는 버린다
    sender = clean_text(sender, 32) or "unknown"
    body = clean_text(body, MAX_TEXT_CHARS)
    if not body.strip():
        return
    say(f"{sender}: {body}")


def send_to(peer, me, text):
    message = {"app": APP, "id": me["id"], "from": me["name"], "text": text}
    with socket.create_connection((peer["ip"], peer["port"]), timeout=3) as conn:
        conn.sendall(json.dumps(message).encode("utf-8"))
        conn.shutdown(socket.SHUT_WR)


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
    t("help.help"),
    t("help.quit"),
    "",
    t("help.keys"),
    t("help.enter"),
    t("help.backspace"),
    t("help.ctrl_l"),
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


def screen_frame(lines, status, typed, cols, rows):
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
    if ch in ("\x03", "\x04"):    # Ctrl+C, Ctrl+D
        return False
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
        with chat.lines_lock:
            lines = list(chat.lines)
        frame = screen_frame(lines, status, typed[0], size.columns, size.lines)
        with screen_lock:
            sys.stdout.write(frame)
            sys.stdout.flush()

    def add(line, mine=False):
        chat.add_line(line, mine)
        redraw()

    with _sink_lock:
        _sink = add
    _restore_screen = leave
    redraw()
    threading.Thread(target=leave_when_idle, args=(chat,), daemon=True).start()
    try:
        while on_key(read_char(), chat, typed):
            redraw()
    finally:
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


def run(args):
    global _sink
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("", args.tcp_port))
    server.listen()
    name = args.name or default_name()
    me = {"id": uuid.uuid4().hex, "name": name, "port": server.getsockname()[1]}

    chat = Chat(me)
    with _sink_lock:
        _sink = chat.add_line
    threading.Thread(target=receive_messages, args=(server,), daemon=True).start()
    threading.Thread(target=announce, args=(args.udp_port, me), daemon=True).start()
    threading.Thread(target=listen_for_peers, args=(args.udp_port, chat), daemon=True).start()

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
    chat.add_argument("--udp-port", type=int, default=UDP_PORT, help="discovery port (keep the same on every PC)")
    chat.add_argument("--dock", action="store_true", help="show this pane as the herdr-dock block")
    chat.add_argument("--pane", default=None, help="herdr pane id of the dock block (with --dock)")
    commands.add_parser("open", help="split the current pane and run the chat in it")
    commands.add_parser("dock-start", help="show the chat as a block in the herdr-dock workspace")
    commands.add_parser("dock-stop", help="remove the dock block")
    args = parser.parse_args(argv)
    if args.command == "run":
        return run(args)
    if args.command == "open":
        return open_chat(args)
    if args.command == "dock-start":
        return dock_start(args)
    if args.command == "dock-stop":
        return dock_stop(args)
    return 1


if __name__ == "__main__":
    sys.exit(main())
