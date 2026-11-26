"""The chat screen: the layout, the message rows and the keys, as pure functions (no output here). The full-screen
mode and the frame writes stay in talk.py. Keys and layout follow the Rust edition."""

import os
import sys
import time

from core.events import Message, Notice, PeerRow
from i18n import can_save_language, cycle_language, t
from extend import settings_form as relay_form


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
            return (KEY_F1 if code == ";" else relay_form.KEY_F2 if code == "<" else relay_form.KEY_F3 if code == "=" else relay_form.KEY_F9 if code == "C"
                    else relay_form.KEY_UP if code == "H" else relay_form.KEY_DOWN if code == "P" else "")
        return ch
    fd = sys.stdin.fileno()
    first = os.read(fd, 1)
    if first == b"":
        # An arrow or function key sends Esc plus more bytes at once: drop them, so only a lone Esc is Esc.
        import select
        if select.select([fd], [], [], 0.02)[0]:
            sequence = os.read(fd, 16)
            return relay_form.KEY_UP if sequence[:2] in (b"[A", b"OA") else relay_form.KEY_DOWN if sequence[:2] in (b"[B", b"OB") else ""
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


def help_box():
    """The help box in the current language (built at each use, so F3 changes it at once)."""
    lines = [
        t("help.title_send"),
        t("help.send"),
        "",
        t("help.commands"),
        t("help.name"),
        t("help.lang"),
        t("help.codes"),
        t("help.peers"),
        t("help.trust"),
        t("help.rejoin"),
        t("help.help"),
        t("help.quit"),
        "",
        t("help.keys"),
        t("help.enter"),
        t("help.backspace"),
        t("help.ctrl_l"),
        t("help.ctrl_c"),
    ]
    width = max(char_width(line) for line in lines)
    border = "+" + "-" * (width + 2) + "+"
    # Pad by display width, so wide characters (Korean, Japanese) keep the right edge straight.
    return [border] + [f"| {line}{' ' * (width - char_width(line))} |" for line in lines] + [border]


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
    if mine:
        sender = f"{sender} {t('chat.you')}"  # "예준 (나) > ...", as the Rust edition
    head = f"[{stamp}] {sender} : "
    room = width - char_width(head)
    if room < 8:
        return [f"{body}{row}" for row in wrap_line(f"{sender} : {text}", width)]
    rows = wrap_line(text, room)
    first = f"{MUTED}[{stamp}] {BOLD}{ACCENT}{sender}{relay_form.RESET}{MUTED} : {body}{rows[0]}"
    return [first] + [f"{' ' * char_width(head)}{body}{row}" for row in rows[1:]]


MAX_NODE_ROWS = 6  # people listed under the CONNECTED NODES title; more become one "…외 N명" row


ERROR_WORDS = ("failed", "could not", "no identity", "incomplete", "invalid", "not valid", "not joined")  # as the Rust edition, plus Python's wording


def status_tag(chat):
    """The tag at the end of the status row: [SYSTEM READY] without a relay, [CONNECTED] once the room is joined, [ERROR] when
    the relay's status text names a failure, otherwise [CONNECTING...]."""
    if not chat.supabase or chat.relay is None:
        return relay_form.TAGS["ready"]
    if any(word in chat.relay_status for word in ERROR_WORDS):
        return relay_form.TAGS["error"]
    return relay_form.TAGS["connected" if chat.relay.joined else "connecting"]


def screen_frame(lines, status, typed, cols, rows, overlay=None, nodes=(), mode="", tag=None):
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

    version = "SYS.VER 1.0"
    head = relay_form.header(cols, version)
    head = head[:-len(version)] + f"{MUTED}{version}" if head.endswith(version) else head  # the app name is green, the version grey
    put(1, f"{ACCENT}{head}")
    put(2, f"{BORDER}{relay_form.divider(cols)}")
    put(3, f"{ACCENT}COMMUNICATION LOG")
    for index in range(area):
        if index < len(visible):
            put(index + 4, visible[index])
        else:
            put(index + 4, "")
    # The overlay rows (the help box), over the bottom rows of the messages. They are not chat lines.
    shown = (overlay or [])[:area]
    for index, text in enumerate(shown):
        text = fit(text, cols - 1)
        put(area + 4 - len(shown) + index, f"{relay_form.OVERLAY}{text}{' ' * (cols - 1 - char_width(text))}")
    top = 4 + area
    put(top, f"{ACCENT}CONNECTED NODES")
    for index, person in enumerate(people):
        put(top + 1 + index, f"{MUTED if index == 0 else TEXT}{fit(person, cols - 1)}")
    put(top + block, f"{BORDER}{relay_form.divider(cols)}")
    shown_typed = tail(typed, cols - 13)
    typed_shown = f"{ACCENT}{shown_typed}_" if typed else f"{MUTED}{t('chat.input_hint')}"
    put(rows - 2, f"{ACCENT}MESSAGE > {typed_shown}")
    # The status row ends with the tag and the key hint for the help box (F1 or /help opens it); the status text is cut to fit.
    hint = t("status.help_hint")
    tag = tag or relay_form.TAG
    tag_color = ACCENT if tag in (relay_form.TAGS["ready"], relay_form.TAGS["connected"]) else relay_form.WARN
    ending = f"{tag}  {hint}"
    left = fit(status, cols - 1 - char_width(ending) - 2)
    gap = " " * max(0, cols - 1 - char_width(left) - char_width(ending))
    put(rows - 1, f"{MUTED}{left}{gap}{tag_color}{tag}  {MUTED}{hint}")
    # The bottom row: made by Python on the right, nothing else.
    footer = fit(relay_form.FOOTER, cols - 1)
    put(rows, f"{' ' * (cols - 1 - char_width(footer))}{MUTED}{footer}")
    # The real cursor goes to the `_` on the input line (the frame's last write is the footer row, so it would sit there).
    cursor_col = 1 + char_width("MESSAGE > ") + char_width(shown_typed)
    return "".join(out) + f"\x1b[{rows - 2};{cursor_col}H\x1b[?25h"


def on_key(ch, chat, typed):
    """Apply one key to the typed text (a one-item list, changed in place). The keys are the Rust edition's.
    Return False to leave the chat."""
    if ch == relay_form.KEY_F3:  # the language is changed at once and the screen is drawn again by the caller
        cycle_language()
        if not can_save_language():
            chat.emit(Notice("lang.not_saved"))
        return True
    if chat.help_open:  # the key that closes the help box is not typed (Ctrl+C still leaves the chat)
        chat.help_open = False
        return ch not in ("\x03", "\x04")
    if ch == KEY_F1:
        chat.help_open = True
        return True
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


def event_rows(event):
    """The chat rows of an event as (text, mine, sender): a notice is its locale text, one row per line; a message
    keeps its sender ("[time] sender > text" on screen); a plain sender is labelled "(plain text)"; a peer line is built
    from its state here, so the core holds no words."""
    if isinstance(event, Notice):
        return [(row, False, None) for row in t(event.code, **event.args).split(chr(10))]
    if isinstance(event, Message):
        return [(event.text, event.mine, f"{event.sender} ({t('chat.plain')})" if event.plain else event.sender)]
    if isinstance(event, PeerRow):
        return [(f"- {event.prefix}{event.name} ({event.ip}:{event.port}) {_peer_state(event)}", False, None)]
    return [(event.text, event.mine, None)]


def _peer_state(row):
    """The words of a /peers line's state."""
    if row.state == "verified":
        return t("peers.verified")
    if row.state == "plain":
        return t("peers.plain")
    if row.state == "unverified":
        return t("peers.unverified", code=row.code)
    # shortcut: the pending text is English-only; move it to the locales with the other peer states.
    return "(code pending)" if row.state == "pending" else ""


def event_texts(event):
    """The rows of an event as plain text, as they are printed when no screen is up."""
    return [text if sender is None else f"{sender} > {text}" for text, _, sender in event_rows(event)]


def print_event(event):
    for text in event_texts(event):
        print(text, flush=True)


def add_event(chat, event):
    """Keep an event's rows as chat lines (what the screen, or the control channel, shows)."""
    for text, mine, sender in event_rows(event):
        chat.add_line(text, mine, sender)
