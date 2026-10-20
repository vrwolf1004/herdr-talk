"""herdr-talk: simple chat between PCs on the same network (phase 1, no server).

How it works
  - Every instance broadcasts its name and TCP port over UDP every few seconds.
  - Other instances on the same network hear the broadcast and remember the peer.
  - A message is sent straight to each peer over TCP. Nothing is stored.

Screen
  - With prompt_toolkit installed: messages scroll in the upper area, the input line stays at the bottom.
  - Without it (or without a usable console): plain line-by-line chat.

Commands (typed in the input line)
  <text>     send to every peer
  /peers     list peers seen recently
  /help      show commands
  /quit      leave the chat
"""

import argparse
import asyncio
import json
import locale
import os
import random
import shutil
import socket
import sys
import threading
import time
import unicodedata
import uuid

APP = "herdr-talk"
UDP_PORT = 47850          # discovery port, the same on every PC
ANNOUNCE_SECONDS = 2
PEER_TIMEOUT_SECONDS = 10
MAX_MESSAGE_BYTES = 64 * 1024
MAX_TEXT_CHARS = 2000
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


def default_name():
    """Random name from the list matching the PC's language (e.g. Korean for a Korean locale)."""
    language = (locale.getlocale()[0] or "").lower()
    for code, (language_name, names) in NAMES_BY_LANGUAGE.items():
        if language.startswith(code) or language.startswith(language_name):
            return random.choice(names)
    return random.choice([name for _, names in NAMES_BY_LANGUAGE.values() for name in names])

_sink_lock = threading.Lock()
_sink = None  # where say() writes; set by run()


def say(line, style=""):
    """Show one line in the chat (scrolling area, or plain stdout). style is a prompt_toolkit style class."""
    with _sink_lock:
        if _sink is None:
            print(line, flush=True)
        else:
            _sink(line, style)


class Chat:
    """Shared state: this instance, known peers, and the lines shown on screen."""

    def __init__(self, me):
        self.me = me
        self.peers = {}
        self.peers_lock = threading.Lock()
        self.lines = []
        self.app = None
        self.last_activity = time.time()

    def add_line(self, line, style=""):
        self.last_activity = time.time()
        self.lines.append((style, line))
        del self.lines[:-MAX_LINES]
        if self.app is not None:
            self.app.invalidate()

    def fresh_peers(self):
        cutoff = time.time() - PEER_TIMEOUT_SECONDS
        with self.peers_lock:
            return {pid: info for pid, info in self.peers.items() if info["seen"] >= cutoff}

    def handle(self, text):
        """Handle one line typed by the user. Return False to leave the chat."""
        text = text.strip()
        if not text:
            return True
        if text == "/quit":
            return False
        if text == "/help":
            say("<text> send to all, /name <new> change name, /lang <code> random name for a language, /peers list peers, /quit leave")
            return True
        if text == "/name" or text.startswith("/name "):
            new_name = text[5:].strip()[:32]
            if not new_name:
                say("usage: /name <new name>   (or press Ctrl+N)")
                return True
            old_name = self.me["name"]
            self.me["name"] = new_name
            say(f"* {old_name} is now {new_name}")
            return True
        if text == "/lang" or text.startswith("/lang "):
            code = text[5:].strip().lower()
            if code not in NAMES_BY_LANGUAGE:
                say("usage: /lang <" + "|".join(NAMES_BY_LANGUAGE) + ">")
                return True
            old_name = self.me["name"]
            self.me["name"] = random.choice(NAMES_BY_LANGUAGE[code][1])
            say(f"* {old_name} is now {self.me['name']}")
            return True
        if text == "/peers":
            found = self.fresh_peers()
            if not found:
                say("no peers yet")
            for info in found.values():
                say(f"- {info['name']} ({info['ip']}:{info['port']})")
            return True
        text = text[:MAX_TEXT_CHARS].replace("\n", " ")
        targets = self.fresh_peers()
        if not targets:
            say("no peers yet. Start herdr-talk on another PC on the same network.")
            return True
        for info in targets.values():
            try:
                send_to(info, self.me, text)
            except OSError:
                say(f"! could not reach {info['name']}")
        say(f"{self.me['name']} (you): {text}", style="class:mine")
        return True


def announce(udp_port, me):
    """Broadcast this instance's name and TCP port every few seconds."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    while True:
        payload = json.dumps({"app": APP, **me}).encode("utf-8")
        try:
            sock.sendto(payload, ("255.255.255.255", udp_port))
        except OSError:
            pass
        time.sleep(ANNOUNCE_SECONDS)


def listen_for_peers(udp_port, chat):
    """Remember every other instance that announces itself."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("", udp_port))
    while True:
        data, addr = sock.recvfrom(4096)
        try:
            info = json.loads(data.decode("utf-8"))
        except ValueError:
            continue
        if info.get("app") != APP or info.get("id") == chat.me["id"]:
            continue
        peer_id = info.get("id", "")
        with chat.peers_lock:
            is_new = peer_id not in chat.peers
            chat.peers[peer_id] = {
                "name": info.get("name", "unknown"),
                "ip": addr[0],
                "port": info.get("port", 0),
                "seen": time.time(),
            }
        if is_new:
            say(f"* {info.get('name', 'unknown')} joined ({addr[0]})")


def receive_messages(server):
    """Accept TCP connections. Each connection carries one JSON message."""
    while True:
        conn, _ = server.accept()
        threading.Thread(target=receive_one, args=(conn,), daemon=True).start()


def receive_one(conn):
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
    if message.get("app") != APP:
        return
    say(f"{message.get('from', 'unknown')}: {message.get('text', '')}")


def send_to(peer, me, text):
    message = {"app": APP, "id": me["id"], "from": me["name"], "text": text}
    with socket.create_connection((peer["ip"], peer["port"]), timeout=3) as conn:
        conn.sendall(json.dumps(message).encode("utf-8"))
        conn.shutdown(socket.SHUT_WR)


def leave_when_idle(chat):
    """Plain mode: exit the process after IDLE_LIMIT_SECONDS without activity."""
    while True:
        time.sleep(10)
        if time.time() - chat.last_activity >= IDLE_LIMIT_SECONDS:
            print("* left: no activity for 10 minutes", flush=True)
            os._exit(0)


def run_plain(chat):
    """Line-by-line chat. Used when prompt_toolkit is missing or no console is available."""
    global _sink
    with _sink_lock:
        _sink = lambda line, style="": print(line, flush=True)
    threading.Thread(target=leave_when_idle, args=(chat,), daemon=True).start()
    while True:
        try:
            text = input("> ")
        except (EOFError, KeyboardInterrupt):
            break
        if not chat.handle(text):
            break
    say("bye")
    return 0


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
    "Send a message",
    "  <text>          send to everyone",
    "",
    "Commands",
    "  /name <new>     change your name",
    "  /lang <code>    random name by language",
    "    codes: ko en zh ja vi fr de es ru id pt",
    "  /peers          list peers seen recently",
    "  /help           show this box",
    "  /quit           leave the chat",
    "",
    "Keys",
    "  Enter           send or run a command",
    "  Backspace       delete one character",
    "  Ctrl+L          clear the message list",
    "  Ctrl+C          quit",
    "",
    "press any key to close",
]
HELP_WIDTH = max(len(line) for line in HELP_LINES)
HELP_BORDER = "+" + "-" * (HELP_WIDTH + 2) + "+"
HELP_BOX = [HELP_BORDER] + [f"| {line.ljust(HELP_WIDTH)} |" for line in HELP_LINES] + [HELP_BORDER]


def run_ansi(chat):
    """Chat with the input line fixed on the bottom row, without prompt_toolkit.
    Every change redraws the screen from our own message list, with absolute cursor positions and no scroll region."""
    global _sink
    enable_vt()
    history = []
    buffer = []
    lock = threading.Lock()
    help_open = False

    def render():
        size = shutil.get_terminal_size((80, 24))
        cols, rows = max(20, size.columns), max(5, size.lines)
        wrapped = []
        for line in history:
            wrapped += wrap_line(line, cols - 1)
        visible = wrapped[-(rows - 1):]
        visible = [""] * (rows - 1 - len(visible)) + visible
        if help_open and len(visible) >= len(HELP_BOX) and cols >= len(HELP_BORDER):
            top = (len(visible) - len(HELP_BOX)) // 2
            visible[top:top + len(HELP_BOX)] = HELP_BOX
        text = "".join(buffer)
        # Plain newlines between rows (no cursor jumps), so the input row stays on the last line.
        sys.stdout.write("\x1b[H\x1b[2J" + "\n".join(visible) + "\n> " + text)
        sys.stdout.flush()

    def add_message(line, style=""):
        with lock:
            history.append(line)
            del history[:-MAX_LINES]
            render()

    with _sink_lock:
        _sink = add_message
    with lock:
        render()
    threading.Thread(target=leave_when_idle, args=(chat,), daemon=True).start()
    try:
        while True:
            ch = read_char()
            if help_open and ch not in ("\x03", "\x04"):   # any key closes the help box
                help_open = False
                with lock:
                    render()
                continue
            if ch in ("\r", "\n"):
                text = "".join(buffer)
                buffer.clear()
                if text.strip() == "/help":
                    help_open = True
                    with lock:
                        render()
                    continue
                # Chat messages come back from chat.handle() as "name (you): text", so echo only commands here.
                if text.startswith("/"):
                    with lock:
                        history.append(f"> {text}")
                        del history[:-MAX_LINES]
                        render()
                if not chat.handle(text):
                    break
                with lock:
                    render()
            elif ch in ("\x08", "\x7f"):
                if buffer:
                    buffer.pop()
                with lock:
                    render()
            elif ch in ("\x03", "\x04"):   # Ctrl+C, Ctrl+D
                break
            elif ch == "\x0c":             # Ctrl+L: clear the messages on screen
                with lock:
                    history.clear()
                    render()
            elif ch and ch.isprintable():
                buffer.append(ch)
                with lock:
                    render()
    finally:
        with _sink_lock:
            _sink = None
        sys.stdout.write("\n")
        sys.stdout.flush()
    say("bye")
    return 0


def run_screen(chat):
    """Full-screen chat: messages above, input line below. Falls back to run_plain() if it cannot start."""
    try:
        from prompt_toolkit.application import Application
        from prompt_toolkit.buffer import Buffer
        from prompt_toolkit.key_binding import KeyBindings
        from prompt_toolkit.layout import HSplit, Layout, VSplit, Window
        from prompt_toolkit.layout.controls import BufferControl, FormattedTextControl
        from prompt_toolkit.styles import Style
    except ImportError:
        return run_ansi(chat)

    def messages_text():
        return [(style, line + "\n") for style, line in chat.lines]

    def scroll_to_bottom(window):
        info = window.render_info
        if info is None:
            return 0
        return max(0, info.content_height - info.window_height)

    def status_text():
        peers = len(chat.fresh_peers())
        return f" herdr-talk  |  {chat.me['name']}  |  peers: {peers}  |  Ctrl+N: rename   /help   Ctrl+C: quit "

    message_window = Window(content=FormattedTextControl(messages_text), wrap_lines=True, always_hide_cursor=True)
    message_window.get_vertical_scroll = scroll_to_bottom
    status_window = Window(content=FormattedTextControl(status_text), height=1, style="class:status")
    divider = Window(height=1, char="-", style="class:divider")

    app = None

    def accept(buffer):
        text = buffer.text
        buffer.text = ""
        if not chat.handle(text) and app is not None:
            app.exit()
        return True

    input_buffer = Buffer(multiline=False, accept_handler=accept)
    input_window = Window(content=BufferControl(buffer=input_buffer), height=1, dont_extend_height=True)
    prompt_window = Window(content=FormattedTextControl("> "), width=2, dont_extend_width=True)

    root = HSplit([status_window, message_window, divider, VSplit([prompt_window, input_window])])
    keys = KeyBindings()

    @keys.add("c-c")
    @keys.add("c-d")
    def _quit(event):
        event.app.exit()

    @keys.add("c-n")
    def _rename(event):
        input_buffer.text = "/name "
        input_buffer.cursor_position = len(input_buffer.text)

    style = Style.from_dict({
        "status": "bg:#3a3a3a #ffffff",
        "divider": "#666666",
        "mine": "bold #58a6ff",
    })
    try:
        app = Application(
            layout=Layout(root, focused_element=input_window),
            key_bindings=keys,
            style=style,
            full_screen=True,
        )
    except Exception:  # no usable console for prompt_toolkit
        return run_ansi(chat)

    async def leave_when_idle_screen():
        while True:
            await asyncio.sleep(10)
            if time.time() - chat.last_activity >= IDLE_LIMIT_SECONDS:
                chat.add_line("* left: no activity for 10 minutes")
                app.exit()
                return

    chat.app = app
    app.create_background_task(leave_when_idle_screen())
    app.run()
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

    say(f"herdr-talk: you are '{me['name']}' (TCP port {me['port']}). Type /help for commands.")
    return run_screen(chat)


def main(argv=None):
    parser = argparse.ArgumentParser(prog="herdr-talk")
    commands = parser.add_subparsers(dest="command", required=True)
    chat = commands.add_parser("run", help="open the chat in this pane")
    chat.add_argument("--name", default=None, help="name shown to others (random if omitted)")
    chat.add_argument("--tcp-port", type=int, default=0, help="TCP port for messages (0 = pick one)")
    chat.add_argument("--udp-port", type=int, default=UDP_PORT, help="discovery port (keep the same on every PC)")
    args = parser.parse_args(argv)
    if args.command == "run":
        return run(args)
    return 1


if __name__ == "__main__":
    sys.exit(main())
