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
import hmac
import json
import os
import secrets
import shutil
import socket
import sys
import threading
import time
import unicodedata

from core.names import (  # re-exported: the name rules live in core/names.py
    NAMES_BY_LANGUAGE, clean_text, default_name, peer_name, pick_name, plain_sender, sender_name, squeeze_spaces, unique_name)
import herdr_api
import relay_chat
from extend import settings_form as relay_form
import relay_settings
from core import relay_rest
from core import sessions
from core import store
from core.events import Line, Notice
from core.chat import (  # the chat state and the LAN receive side: core/chat.py, re-exported for the screen and tests
    Chat, RelayHooks, event_rows, event_texts, print_event, remember_peer, reveal_message, read_reveal, exchange_reveal, show_code, reveal_later, receive_messages, receive_one, listen_for_peers, _listen_loop, _read_plain, _receive_encrypted, resolve_name_clash, MAX_TOTAL_PEERS, PEER_TIMEOUT_SECONDS, MAX_PEERS, JOIN_LIMIT_SECONDS, AUTO_RENAME_GAP_SECONDS, SEND_WORKERS, MAX_LINES, MESSAGE_LIMIT, MESSAGE_WINDOW_SECONDS, PHONE_SHOWN_SECONDS, REVEAL_RETRY_SECONDS, LOOP_POLL_SECONDS, MAX_CONNECTIONS, MAX_PER_IP, CONNECTION_SECONDS, MAX_MESSAGE_BYTES)  # noqa: F401
from core.net import (  # the announce and the send: core/net.py
    APP, ANNOUNCE_SECONDS, SEND_TIMEOUT_SECONDS, MAX_FRAME, MAX_TEXT_CHARS, STOP, _PEER_CLOSED_ERRNOS, _PEER_CLOSED_WINERRORS, announce, is_local_address, _Deadline, _peer_closed, send_to, _send_encrypted, deliver)  # noqa: F401


def load_dock_layout():
    """설치된 herdr-dock-grid 가 있으면 그 원본을, 없으면 이 plugin 의 복사본(src/dock_layout.py)을 쓴다."""
    grid = herdr_api.installed_plugin_path("herdr-dock-grid")
    if grid:
        sys.path.insert(0, os.path.join(grid, "src"))
    import dock_layout
    return dock_layout


dock = load_dock_layout()
from i18n import cycle_language, t


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
IDLE_LIMIT_SECONDS = 10 * 60   # leave automatically after this long without any chat activity

# Display names picked at random when --name is not given: the list for this PC's language, else every list.
# Each entry: language code -> (Windows language name, names).


_restore_screen = None  # undoes the full-screen mode while the chat runs; leave_when_idle() calls it


# ---------------------------------------------------------------- 제어 채널
# Rust 판 (control.rs) 과 같은 형식: 127.0.0.1 위의 줄 단위 JSON. 첫 줄은 {"token": ...}, 이후 요청마다 한 줄 답.
# 요청: {"cmd": "lines"|"peers"|"send"|"shutdown"}. port 와 token 은 상태 폴더의 control-<pid>.json 에 적는다.

CONTROL_MAX_REQUEST_BYTES = 64 * 1024
CONTROL_MAX_CLIENTS = 4
CONTROL_IDLE_SECONDS = 300
CONTROL_LINES_SHOWN = 300


def state_dir():
    """This process's node folder (see sessions.py): the session chosen at start, else the base folder."""
    return sessions.state_dir()


def control_answer(chat, request, allow_shutdown):
    cmd = request.get("cmd")
    if cmd == "lines":
        with chat.lines_lock:
            rows = list(chat.lines[-CONTROL_LINES_SHOWN:])
        return {"ok": True, "lines": [{"text": f"{sender} > {text}" if sender else text, "mine": mine} for text, mine, sender, _ in rows]}
    if cmd == "peers":
        peers = [{"name": info["name"], "ip": info["ip"], "port": info["port"]} for info in chat.fresh_peers().values()]
        mode = relay_settings.SUPABASE if chat.supabase else relay_settings.LOCAL
        return {"ok": True, "peers": peers, "me": {"name": chat.me["name"], "port": chat.me["port"]}, "mode": mode, "relay": chat.relay_status}
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
        if pid == os.getpid() or sessions.control_live(state, pid):
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
    # "started" tells this pid from a later process with the same pid (sessions.control_live); null when unknown.
    body = json.dumps({"port": listener.getsockname()[1], "token": token, "pid": os.getpid(),
                       "started": sessions.process_start_time(os.getpid())})
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


char_width = relay_form.char_width  # one width rule for the chat and the forms


KEY_F1 = "\x00;"  # F1 on Windows: a 0 code, then ;. Ctrl+H is not used: Windows sends it as Backspace.


def read_char():
    """Block until one key is typed and return it as text. Windows reads keys; elsewhere reads one UTF-8 character."""
    if os.name == "nt":
        import msvcrt
        ch = msvcrt.getwch()
        if ch in ("\x00", "\xe0"):   # arrow and function keys send a second code; only F1 is used
            code = msvcrt.getwch()
            return KEY_F1 if code == ";" else relay_form.KEY_F2 if code == "<" else relay_form.KEY_F3 if code == "=" else relay_form.KEY_F9 if code == "C" else ""
        return ch
    fd = sys.stdin.fileno()
    first = os.read(fd, 1)
    if first == b"":
        # An arrow or function key sends Esc plus more bytes at once: drop them, so only a lone Esc is Esc.
        import select
        if select.select([fd], [], [], 0.02)[0]:
            os.read(fd, 16)
            return ""
        return ""
    need = 4 if first[0] >= 0xF0 else 3 if first[0] >= 0xE0 else 2 if first[0] >= 0xC0 else 1
    data = first + (os.read(fd, need - 1) if need > 1 else b"")
    return data.decode("utf-8", "ignore")


def read_char_timeout(seconds):
    """A typed key as text, or None when nothing was typed within `seconds`."""
    deadline = time.monotonic() + seconds
    if os.name == "nt":
        import msvcrt
        while time.monotonic() < deadline:
            if msvcrt.kbhit():
                return read_char()
            time.sleep(0.03)
        return None
    import select
    return read_char() if select.select([sys.stdin.fileno()], [], [], seconds)[0] else None


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


ACCENT, BORDER, MUTED = relay_form.ACCENT, relay_form.BORDER, relay_form.MUTED
TEXT, BOLD = relay_form.TEXT, relay_form.BOLD


def message_rows(text, mine, sender, stamp, cols):
    """The styled rows of one chat line, as Rust screen.rs message_rows: "[time] name > text", the text wrapped under its
    start. Mine is bold green; the name is the title colour. A notice (no sender) is muted."""
    width = cols - 1
    if sender is None:
        return [f"{MUTED}{row}" for row in wrap_line(text, width)]
    body = f"{BOLD}{ACCENT}" if mine else TEXT
    head = f"[{stamp}] {sender} > "
    room = width - char_width(head)
    if room < 8:
        return [f"{body}{row}" for row in wrap_line(f"{sender} > {text}", width)]
    rows = wrap_line(text, room)
    first = f"{MUTED}[{stamp}] {BOLD}{ACCENT}{sender}{relay_form.RESET}{MUTED} > {body}{rows[0]}"
    return [first] + [f"{' ' * char_width(head)}{body}{row}" for row in rows[1:]]


MAX_NODE_ROWS = 6  # people listed under the CONNECTED NODES title; more become one "…외 N명" row


def screen_frame(lines, status, typed, cols, rows, overlay=None, nodes=(), mode=""):
    """The whole chat screen as one string of ANSI codes, in the GUI's retro layout: the header and a divider,
    COMMUNICATION LOG with the messages, the CONNECTED NODES row, a divider, the keyboard guide, the input row, the status
    row, and the made-by footer alone on the bottom row (the status text is long, so it must not share that row).
    Messages go from the top; mine are bold green. `nodes` are the node labels, `mode` the right end of the header.
    Pure, so tests can check the layout."""
    cols, rows = max(20, cols), max(10, rows)
    # The node list: a title row, then one row per person (the first is this PC). Long lists are cut, as the screen is small.
    room = max(2, min(MAX_NODE_ROWS, rows - 11))  # this PC's row always shows, with the cut row when needed
    people = list(nodes)
    if len(people) > room:
        kept = max(0, room - 1)
        people = people[:kept] + [t("nodes.more", n=len(nodes) - kept)]
    block = 1 + len(people)
    area = rows - 7 - block   # rows: header, divider, log title, messages, nodes (block), divider, input, status, footer
    wrapped = []
    for entry in lines:
        line, mine, sender, stamp = (tuple(entry) + (None, ""))[:4]  # a (text, mine) pair is a notice
        wrapped += message_rows(line, mine, sender, stamp, cols)
    visible = wrapped[max(0, len(wrapped) - area):]
    # Each row is overwritten in place and the rest of it cleared (ESC K); no whole-row blank first, so nothing flickers.
    out = []

    def put(row, content):
        out.append(relay_form.place_row(row, content))

    put(1, f"{ACCENT}{fit(relay_form.header(cols, 'SYS.VER 1.0 · ' + (mode or 'SYSTEM READY')), cols - 1)}")
    put(2, f"{BORDER}{relay_form.divider(cols)}")
    put(3, f"{ACCENT}COMMUNICATION LOG")
    for index in range(area):
        if index < len(visible):
            put(index + 4, visible[index])
        else:
            put(index + 4, "")
    # The phone address and token, over the bottom rows of the messages. They are not chat lines.
    shown = (overlay or [])[:area]
    for index, text in enumerate(shown):
        text = fit(text, cols - 1)
        put(area + 4 - len(shown) + index, f"{relay_form.OVERLAY}{text}{' ' * (cols - 1 - char_width(text))}")
    top = 4 + area
    put(top, f"{ACCENT}CONNECTED NODES")
    for index, person in enumerate(people):
        put(top + 1 + index, f"{ACCENT if index == 0 else TEXT}{fit(person, cols - 1)}")
    put(top + block, f"{BORDER}{relay_form.divider(cols)}")
    shown_typed = tail(typed, cols - 13)
    typed_shown = f"{ACCENT}{shown_typed}_" if typed else f"{MUTED}{t('chat.input_hint')}"
    put(rows - 2, f"MESSAGE > {typed_shown}")
    # The status row ends with the tag and the key hint for the help box (F1 or /help opens it); the status text is cut to fit.
    hint = t("status.help_hint")
    ending = f"{relay_form.TAG}  {hint}"
    left = fit(status, cols - 1 - char_width(ending) - 2)
    gap = " " * max(0, cols - 1 - char_width(left) - char_width(ending))
    put(rows - 1, f"{ACCENT}{left}{gap}{relay_form.TAG}  {MUTED}{hint}")
    # The bottom row: made by Python on the right, nothing else.
    footer = fit(relay_form.FOOTER, cols - 1)
    put(rows, f"{' ' * (cols - 1 - char_width(footer))}{MUTED}{footer}")
    # The real cursor goes to the `_` on the input line (the frame's last write is the footer row, so it would sit there).
    cursor_col = 1 + char_width("MESSAGE > ") + char_width(shown_typed)
    return "".join(out) + f"\x1b[{rows - 2};{cursor_col}H\x1b[?25h"


def _enter_screen():
    """Switch to the alternate screen with the cursor hidden. On POSIX also read keys one at a time (raw mode).
    Windows reads keys with msvcrt and needs no mode change. Returns the function that undoes all of it."""
    global _last_frame, _kept_screen
    if _kept_screen is not None:  # the first-run screen left the full-screen mode on for the chat: no flash between them
        leave, _kept_screen = _kept_screen, None
        _last_frame = None
        return leave
    enable_vt()
    sys.stdout.write("\x1b[?1049h\x1b[?25l")
    sys.stdout.flush()
    _last_frame = None  # the alternate screen starts blank
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


_kept_screen = None  # the undo function of a full-screen mode that is still on, for the next _enter_screen() to take over
_last_frame = None  # what is on screen now; a frame equal to it is not written again (no flicker on idle redraws)


def _draw(frame):
    global _last_frame
    if frame == _last_frame:
        return
    _last_frame = frame
    sys.stdout.write(frame)
    sys.stdout.flush()


def settings_screen(state, open_supabase=False, left=False, notice=""):
    """The Ctrl+R screen (inside the full-screen mode that is already on). Returns ("saved" | "cancel", message)."""
    form = relay_form.SettingsForm(state)
    form.message = notice
    if open_supabase:
        form.open_supabase(left)
    dirty = True
    while True:
        if dirty:
            size = shutil.get_terminal_size((80, 24))
            _draw(form.render(size.columns, size.lines))
            dirty = False
        key = read_char_timeout(0.2)
        if key is None:
            dirty = form.poll()  # a create-room answer that has come back
            continue
        outcome = form.on_key(key)
        dirty = True
        if outcome:
            return outcome, form.message


def first_run_screen(state):
    """Ask how to connect before the chat starts. Returns "done" (the mode was saved, or 30 seconds passed and the chat
    starts in local mode without saving) or "quit" (Esc)."""
    global _kept_screen
    leave = _enter_screen()
    keep = False  # on "done" the chat follows at once, so the full-screen mode stays on
    try:
        choice = relay_form.FirstRun()
        shown = object()
        while True:
            left = choice.remaining()
            if left != shown:
                size = shutil.get_terminal_size((80, 24))
                _draw(choice.render(size.columns, size.lines))
                shown = left
            key = read_char_timeout(0.25)
            if key is None:
                if choice.timed_out():
                    keep = True
                    return "done"
                continue
            picked = choice.on_key(key)
            shown = object()
            if picked == "quit":
                return "quit"
            if picked == "local":
                relay_settings.set_mode(state, relay_settings.LOCAL)
                keep = True
                return "done"
            if picked == "supabase":
                if settings_screen(state, open_supabase=True)[0] == "saved":
                    keep = True
                    return "done"
                choice.restart_timer()
    finally:
        if keep:
            _kept_screen = leave
        else:
            leave()


def on_key(ch, chat, typed):
    """Apply one key to the typed text (a one-item list, changed in place). The keys are the Rust edition's.
    Return False to leave the chat."""
    if ch == relay_form.KEY_F3:  # the language is changed at once and the screen is drawn again by the caller
        cycle_language()
        return True
    if chat.help_open:  # the key that closes the help box is not typed (Ctrl+C still leaves the chat)
        chat.help_open = False
        return ch not in ("\x03", "\x04")
    if ch == KEY_F1:
        chat.help_open = True
        return True
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
    global _restore_screen
    leave = _enter_screen()
    typed = [""]
    screen_lock = threading.Lock()

    paused = [False]

    def redraw():
        if paused[0]:
            return
        size = shutil.get_terminal_size((80, 24))
        status = t("status.line", name=chat.me["name"], peers=len(chat.fresh_peers()))
        status += " · " + t("mode.supabase" if chat.supabase else "mode.local")
        if chat.supabase and chat.state_dir:
            saved = relay_settings.load_saved(chat.state_dir)
            if saved["room_code"]:
                status += f" · room ID: {saved['room_code']} · setup code: {saved['setup_code'] or '-'}"
        if chat.relay_status:
            status += f" · {chat.relay_status}"
        if chat.is_host():
            status = f"{t('host.badge')} {status}"
        with chat.lines_lock:
            lines = list(chat.lines)
        nodes = [f"{'HOST/YOU' if chat.is_host() else 'YOU'} {chat.me['name']}"]
        for info in chat.fresh_peers().values():
            link = chat.link_of(info)
            number = chat.numbers.get(info.get("key")) if link == "unverified" else None
            prefix = f"[{number}] " if number is not None else ""
            state = t("nodes.verified") if link == "encrypted" else t("nodes.waiting")
            nodes.append(f"{prefix}{info['name']} ({info['ip']})  {state}")
        mode = "CLOUD SERVICE" if chat.supabase else "BASE STATION"
        overlay = HELP_BOX if chat.help_open else chat.phone_overlay()
        frame = screen_frame(lines, status, typed[0], size.columns, size.lines, overlay, nodes, mode)
        with screen_lock:
            _draw(frame)

    def show(event):
        chat.add_event(event)
        redraw()

    chat.on_event = show
    chat.on_phone_change = redraw
    _restore_screen = leave
    redraw()
    threading.Thread(target=leave_when_idle, args=(chat,), daemon=True).start()
    try:
        while True:
            ch = read_char()
            if ch == "":  # Ctrl+R: connection mode and relay settings
                paused[0] = True
                try:
                    outcome, message = settings_screen(chat.state_dir) if chat.state_dir else ("cancel", "")
                finally:
                    paused[0] = False
                if outcome == "saved":
                    chat.start_relay()
                    chat.emit(Line(message))
                else:
                    chat.emit(Notice("settings.cancel_discard"))
                redraw()
                continue
            if not on_key(ch, chat, typed):
                if not (chat.supabase and chat.state_dir):
                    break
                # Leaving a room chat leaves the room (this node's relay row is deleted) and opens the settings screen with the
                # saved server info kept. Saving joins again; Esc ends the chat.
                paused[0] = True
                chat.stop_relay()
                leaver = threading.Thread(target=relay_chat.leave, args=(chat.state_dir,), daemon=True)  # the delete runs in the background
                leaver.start()
                try:
                    outcome, message = settings_screen(chat.state_dir, open_supabase=True, left=True)
                finally:
                    paused[0] = False
                leaver.join(relay_rest.TIMEOUT_SECONDS + 1)  # a delete still running must not remove the row published next
                if outcome != "saved":
                    break
                typed[0] = ""
                chat.start_relay()
                chat.emit(Line(message))
            redraw()
    finally:
        chat.on_phone_change = None
        chat.on_event = Chat.on_event  # back to stdout: the screen is gone
        _restore_screen = None
        leave()
    chat.emit(Notice("bye"))
    return 0


def herdr_cli(*args):
    return herdr_api.cli(*args)  # raises herdr_api.HerdrError on failure or after the CLI time limit


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
    except (herdr_api.HerdrError, ValueError, KeyError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


def start_node(args, state=None, plaintext=None):
    """Bind the sockets and start the network threads. Returns (server, chat).
    Keys and trusted peers live in the state folder (shared with the Rust edition); plain messages follow config.json."""
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
    # The first node on this PC is the host (방장): the badge shows it; the name stays a country name, as on every node.
    chat.try_become_host()
    chat.on_event = chat.add_event
    threading.Thread(target=receive_messages, args=(server, chat), daemon=True).start()
    threading.Thread(target=announce, args=(args.udp_port, me), daemon=True).start()
    threading.Thread(target=listen_for_peers, args=(args.udp_port, chat), daemon=True).start()
    return server, chat


def serve(args):
    """Chat without a screen, for a GUI: only the control channel runs. Ends on Ctrl+C or a `shutdown` request."""
    server, chat = start_node(args)
    _tidy_sessions(chat)
    control = start_control(chat, state_dir(), allow_shutdown=True)
    chat.watch_host()
    chat.start_relay()
    STOP.wait(ANNOUNCE_SECONDS + 0.5)
    renamed = resolve_name_clash(chat)
    if renamed:
        chat.emit(Notice("name.changed", dict(old=renamed[0], new=renamed[1])))
    chat.emit(Notice("start.hello", dict(name=chat.me["name"], port=chat.me["port"])))
    try:
        while not STOP.wait(1):
            pass
    except KeyboardInterrupt:
        pass
    finally:
        STOP.set()
        chat.stop_relay()
        chat.release_host()
        control.close()
        server.close()
    return 0


def run(args):
    server, chat = start_node(args)
    _tidy_sessions(chat)
    me = chat.me
    control = start_control(chat, state_dir())
    chat.watch_host()  # the first node on this PC owns the phone address; another takes over when it ends
    try:
        quit_first_run = bool(chat.state_dir and relay_settings.needs_first_run(chat.state_dir) and first_run_screen(chat.state_dir) == "quit")
    except KeyboardInterrupt:  # Ctrl+C on the first-run screen is a quit, not a crash
        quit_first_run = True
    if quit_first_run:
        STOP.set()
        chat.release_host()
        control.close()
        server.close()
        return 0
    # A room that refuses this trust code is not entered: back to the settings screen with the reason, until saved or cancelled.
    while chat.state_dir and relay_chat.room_refuses(chat.state_dir):
        outcome, _ = settings_screen(chat.state_dir, open_supabase=True, notice=relay_rest.TRUST_WRONG)
        if outcome != "saved":
            STOP.set()
            chat.release_host()
            control.close()
            server.close()
            return 0
    chat.start_relay()

    if args.dock:
        herdr_api.try_request("pane.report_agent", {"pane_id": args.pane, "source": SOURCE, "agent": AGENT_ID, "state": "idle"})
        threading.Thread(target=keep_dock_block, args=(args.pane, chat), daemon=True).start()
    # Wait one announce cycle so peers that are already here are known before the name is final.
    STOP.wait(ANNOUNCE_SECONDS + 0.5)
    renamed = resolve_name_clash(chat)
    if renamed:
        chat.emit(Notice("name.changed", dict(old=renamed[0], new=renamed[1])))
    chat.emit(Notice("start.hello", dict(name=me["name"], port=me["port"])))
    try:
        return run_ansi(chat)
    finally:
        STOP.set()
        chat.stop_relay()
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
    chat.add_argument("--session", default=None, help="use this node folder (an id, or default); default: a new node")
    serve_parser = commands.add_parser("serve", help="run without a screen; a GUI talks to it over the control channel")
    serve_parser.add_argument("--name", default=None, help="name shown to others (random if omitted)")
    serve_parser.add_argument("--tcp-port", type=int, default=0, help="TCP port for messages (0 = pick one)")
    serve_parser.add_argument("--udp-port", type=int, default=None, help="discovery port (keep the same on every PC; default HERDR_TALK_UDP_PORT or 47850)")
    serve_parser.add_argument("--session", default=None, help="use this node folder (an id, or default); default: a new node")
    commands.add_parser("sessions", help="list the node folders with their connection mode")
    commands.add_parser("open", help="split the current pane and run the chat in it")
    commands.add_parser("dock-start", help="show the chat as a block in the herdr-dock workspace")
    commands.add_parser("dock-stop", help="remove the dock block")
    args = parser.parse_args(argv)
    try:
        return _dispatch(args)
    except (OSError, ValueError) as error:  # e.g. a damaged identity.json: say so, never make a new key
        print(error, file=sys.stderr)
        return 1
    finally:
        try:  # the node's own relay row goes first; a failure here never stops the exit
            relay_chat.leave(sessions.state_dir())
        except Exception:  # noqa: BLE001 - leaving must not block the exit
            pass
        failed = sessions.remove_own_session()  # a node's own folder goes when it stops
        if failed:
            print(f"could not remove session {failed}", file=sys.stderr)


def list_sessions_command():
    base = sessions.base_dir()
    for session_id, folder in [(sessions.DEFAULT_SESSION, base), *sessions.list_sessions(base)]:
        print(f"{session_id}  {folder}  mode={relay_settings.load_saved(folder)['mode']}")
    return 0


def _tidy_sessions(chat):
    """Remove node folders whose node has ended without cleaning up, each after its relay row (best effort). In the
    background: the start does not wait for the relay. Quiet: a folder that cannot be removed is only named, as a chat
    line (stderr would break the full screen)."""
    def leave(folder):
        relay_chat.leave(folder, timeout=relay_chat.DEAD_LEAVE_SECONDS)

    def tidy():
        for session_id in sessions.remove_dead_sessions(sessions.base_dir(), leave=leave):
            chat.emit(Line(f"could not remove dead session {session_id}"))

    threading.Thread(target=tidy, daemon=True).start()


def _dispatch(args):
    if args.command == "sessions":
        return list_sessions_command()
    if args.command in ("run", "serve"):
        sessions.choose_session(args.session)  # a bad or missing --session is reported by main()
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
