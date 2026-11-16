"""The two relay screens of the Python edition as plain state machines (the terminal loop is in talk.py):

  FirstRun      the connection mode choice shown before the chat when no mode was ever chosen (30-second default)
  SettingsForm  Ctrl+R: the four Supabase settings on one screen (the mode comes from the first-run choice)

Same rules as rust/crates/herdr-talk/src/first_run.rs and settings_form.rs. A key value is never drawn: only its length.
Keys arrive as single characters from read_char(): arrow keys are not available, so Tab moves between the fields.
"""

import queue
import threading
import time
import unicodedata

from core import relay_key
from core import relay_rest
import relay_settings
from i18n import t

TIMEOUT_SECONDS = 30
FIELD_LABELS = ("Project URL", "Publishable key", "Display name", "Room id (8 characters)")
FIELD_NAMES = ("url", "key", "name", "room_code")
KEY_FIELD = 1
ROOM_FIELD = 3


# The classic terminal palette of docs/ui-style.md section 3, as 24-bit colours (the same values as the Rust edition).
RESET = "\x1b[0m"
BACKGROUND = "\x1b[48;2;5;10;8m"      # #050A08 every row
TEXT = "\x1b[38;2;180;220;190m"       # #B4DCBE body text
ACCENT = "\x1b[38;2;70;255;130m"      # #46FF82 titles, my lines
BORDER = "\x1b[38;2;65;125;85m"       # #417D55 dividers
MUTED = "\x1b[38;2;125;154;132m"      # #7D9A84 hints (lifted from #526B59 for readability, as in Rust)
WARN = "\x1b[38;2;255;184;77m"        # #FFB84D messages
ALERT = "\x1b[38;2;255;85;85m"        # #FF5555 errors
OVERLAY = "\x1b[38;2;5;10;8m\x1b[48;2;65;125;85m"  # the phone address: background text on the border green
BOLD = "\x1b[1m"
APP_NAME = "▣ HERDR-TALK"


def place_row(row, text, style=""):
    """Place the cursor, paint the row's background, write the text, clear the rest of the row with the background.
    A reset inside the text puts the background and body colour back. No whole-screen clear: that is what flickered."""
    body = text.replace(RESET, RESET + BACKGROUND + TEXT)
    return f"\x1b[{row};1H{BACKGROUND}{style or TEXT}{body}{RESET}{BACKGROUND}\x1b[K"


FOOTER = "made by Python"
TAG = "[SYSTEM READY]"
FIELD_WIDTH = 24  # the width of a blank input line


def _frame(lines, cols, rows):
    """The whole screen as one string: body rows overwrite the old ones and the bottom row carries the footer.
    Rows left over from the last frame are blanked, so nothing stale stays."""
    shown = [(_cut(text, cols - 1), style) for text, style in lines[: rows - 1]]
    shown += [("", "")] * (rows - 1 - len(shown))
    footer = _cut(FOOTER, cols - 1 - char_width(TAG))
    gap = " " * max(0, cols - 1 - char_width(TAG) - char_width(footer))
    shown.append((f"{ACCENT}{TAG}\x1b[0m{gap}{MUTED}{footer}", ""))  # status tag left, footer right, on the bottom row
    return "".join(place_row(index + 1, text, style) for index, (text, style) in enumerate(shown))


def char_width(text):
    """Terminal columns used by text. East Asian wide characters count as two."""
    return sum(2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1 for ch in text)


def _cut(text, cols):
    """The start of text that fits in cols terminal columns, ending with … when it was cut."""
    if char_width(text) <= cols:
        return text
    out, width = "", 0
    for ch in text:
        width += char_width(ch)
        if width > cols - 1:
            break
        out += ch
    return out + "…"


def header(cols, right):
    """The top row: the app name on the left, `right` (version and state) on the right, cut to the width."""
    gap = cols - 1 - len(APP_NAME) - len(right)
    return _cut(APP_NAME + " " * max(2, gap) + right, cols - 1)


def divider(cols):
    return "─" * max(0, cols - 1)


class FirstRun:
    """Choose the connection mode. Any key stops the 30-second timer; if nothing is pressed the chat starts in local
    mode and nothing is saved (the question comes again at the next start)."""

    def __init__(self, clock=time.monotonic, timeout=TIMEOUT_SECONDS):
        self._clock = clock
        self._deadline = clock() + timeout
        self.selected = 0  # 0 local, 1 supabase

    def remaining(self):
        """Whole seconds left (rounded up), or None once a key has stopped the timer."""
        if self._deadline is None:
            return None
        return max(0, int(self._deadline - self._clock() + 0.999))

    def timed_out(self):
        return self._deadline is not None and self._clock() >= self._deadline

    def restart_timer(self, timeout=TIMEOUT_SECONDS):
        """Back from the settings screen: the timer starts again."""
        self._deadline = self._clock() + timeout

    def on_key(self, ch):
        """None while choosing; "local", "supabase" or "quit" when the choice is made."""
        if not ch:
            return None
        self._deadline = None
        if ch == "1":
            return "local"
        if ch == "2":
            return "supabase"
        if ch == "\x1b" or ch in ("\x03", "\x04"):
            return "quit"
        if ch in ("\t", " "):
            self.selected = 1 - self.selected
        elif ch in ("\r", "\n"):
            return "supabase" if self.selected else "local"
        return None

    def render(self, cols, rows):
        lines = [
            (header(cols, "SYS.VER 1.0"), ACCENT),
            (divider(cols), BORDER),
            (t("mode.first_run_title"), ACCENT + BOLD),
            ("", ""),
            *_menu_item(self.selected == 0, "01. LOCAL NETWORK", t("mode.hint_local")),
            ("", ""),
            *_menu_item(self.selected == 1, "02. SUPABASE CLOUD", t("mode.hint_supabase")),
            ("", ""),
        ]
        left = self.remaining()
        if left is not None:
            lines.append((t("mode.timeout_hint", n=left), MUTED))
        lines += [(divider(cols), BORDER), ("1 / 2, or Tab then Enter · Esc: quit", MUTED)]
        return _frame(lines, cols, rows)


def _menu_item(selected, title, hint):
    """A text-menu entry: ▶ and the highlight on the selected one only, the hint under it in muted text."""
    return [
        (f"{'▶ ' if selected else '  '}{title}", ACCENT + BOLD if selected else ""),
        (f"     {hint}", MUTED),
    ]


def publish_failure(status):
    """Why the row write did not go through (the create is not sent), or None when it did."""
    if status is None:
        return "this PC's row could not be published (network or server error)"
    if status == 409:
        return "this PC's short id is already used by another key (409). Try again"
    if not relay_rest.put_succeeded(status):
        return f"this PC's row could not be published (status {status})"
    return None


class SettingsForm:
    """The four fields. Enter moves down and saves on the last row; Esc leaves without saving.
    Ctrl+N asks the relay for a new room and puts its code in the room field (saved with Enter)."""

    def __init__(self, state, mine=None):
        self.state = state
        saved = relay_settings.load_saved(state)
        self.mode = saved["mode"]
        self.fields = [saved[name] for name in FIELD_NAMES]
        self.from_env = relay_settings.from_environment()
        self.focus = 0  # index into fields
        self.message = ""
        self.mine = mine if mine is not None else self._own_keys(state)
        self._room_result = None  # a queue of the running create-room request, None when none runs

    @staticmethod
    def _own_keys(state):
        """(Ed25519 public key hex, Noise public key hex) of this PC, or None when they cannot be read yet."""
        try:
            from core import ed25519
            return ed25519.public_key(relay_key.load_or_create(state)).hex(), relay_key.noise_public_hex(state)
        except (relay_key.SigningKeyError, OSError, ValueError, AttributeError):
            return None

    def open_supabase(self):
        """Start on the first field with Supabase selected (the choice made on the first-run screen)."""
        self.mode = relay_settings.SUPABASE
        self.focus = 0

    def settings(self):
        return {"mode": self.mode, **dict(zip(FIELD_NAMES, self.fields))}

    def save(self):
        """Write relay.json. True when saved; otherwise the message says what is missing or wrong."""
        values = self.settings()
        if values["mode"] == relay_settings.SUPABASE:
            missing = [label for label, value in zip(FIELD_LABELS, self.fields) if not value.strip()]
            if missing:
                self.message = t("settings.save_refused_missing", fields=", ".join(missing))
                return False
        # The input rule: the room code is saved uppercase; the key and the name keep their text.
        values.update({name: value.strip().upper() if name == "room_code" else value.strip() for name, value in zip(FIELD_NAMES, self.fields)})
        reason = relay_settings.save(self.state, values)
        if reason:
            self.message = f"not saved: {reason}"
            return False
        self.message = t("settings.save_ok_supabase" if values["mode"] == relay_settings.SUPABASE else "settings.save_ok_local")
        return True

    def on_key(self, ch):
        """None while editing, "saved" after a successful save, "cancel" for Esc."""
        if not ch:
            return None
        if ch == "\x1b":
            return "cancel"
        if ch == "\t":
            self.focus = (self.focus + 1) % len(FIELD_NAMES)
        elif ch in ("\r", "\n"):
            if self.focus < len(FIELD_NAMES) - 1:
                self.focus += 1
            elif self.save():
                return "saved"
        elif ch == "\x0e":
            self.create_room()
        elif ch in ("\x08", "\x7f"):
            self.fields[self.focus] = self.fields[self.focus][:-1]
        elif ch.isprintable():
            self.fields[self.focus] += ch
        return None

    def create_room(self):
        """Ctrl+N: create-room in a background thread, so the screen keeps working. poll() applies the answer: the code goes
        into the room field (saved with Enter)."""
        if self._room_result is not None:
            return
        if self.mode != relay_settings.SUPABASE:
            self.message = "create room: switch to Supabase mode first"
            return
        if self.mine is None:
            self.message = "create room: this PC has no keys yet (start a chat first)"
            return
        url, key, name = (value.strip() for value in self.fields[:3])
        if not (url and key and name):
            self.message = "create room: fill in the URL, the key and the display name first"
            return
        try:
            config = relay_rest.config(url, key)
            seed = relay_key.load_or_create(self.state)
            noise_pk = relay_key.noise_public_hex(self.state)
            short_id = relay_key.load_or_create_short_id(self.state)
        except (ValueError, relay_key.SigningKeyError, OSError) as error:
            self.message = f"create room failed: {error}"
            return
        result = queue.Queue()

        def work():
            try:
                # The room's owner row (peers) must exist before the room is made (rooms.owner_pk references it).
                failure = publish_failure(relay_rest.put_status(config, seed, noise_pk, short_id, name))
                if failure:
                    result.put(("error", failure))
                    return
                result.put(("ok", relay_rest.create_room(config, seed, name)))
            except (OSError, ValueError) as error:
                result.put(("error", str(error)))

        self._room_result = result
        threading.Thread(target=work, daemon=True).start()
        self.message = "create room: working..."

    def poll(self):
        """Apply the create-room answer when it has come back. True when the screen must be drawn again."""
        if self._room_result is None:
            return False
        try:
            kind, value = self._room_result.get_nowait()
        except queue.Empty:
            return False
        self._room_result = None
        if kind == "ok":
            self.fields[ROOM_FIELD] = value
            self.message = f"room created: {value} (save with Enter to apply)"
        else:
            self.message = f"create room failed: {value}"
        return True

    def _shown(self, index):
        env = (index == 0 and self.from_env[0]) or (index == KEY_FIELD and self.from_env[1])
        if env:
            return "(environment)"
        value = self.fields[index]
        return relay_settings.key_label(value) if index == KEY_FIELD else value

    def render(self, cols, rows):
        local = self.mode != relay_settings.SUPABASE
        lines = [
            (header(cols, "SYS.VER 1.0"), ACCENT),
            (divider(cols), BORDER),
            ("SUPABASE SETTINGS", ACCENT + BOLD),
            ("", ""),
        ]
        for index, label in enumerate(FIELD_LABELS):
            value = self._shown(index)
            fill = "_" * max(0, FIELD_WIDTH - char_width(value))  # the blank input, as an underscore line
            selected = self.focus == index
            lines.append((f"{'▶ ' if selected else '  '}{label:<22} : [{value}{fill}]", ACCENT if selected else (MUTED if local else "")))
        lines.append(("", ""))
        lines.append(("DEVICE IDENTITY", ACCENT))
        if self.mine:
            lines.append((f"  Ed25519 public key: {self.mine[0]}", MUTED))
            lines.append((f"  Noise public key:   {self.mine[1]}", MUTED))
        else:
            lines.append(("  (no keys yet)", MUTED))
        lines.append(("", ""))
        lines.append((self.message, WARN))
        lines.append((divider(cols), BORDER))
        lines.append(("Tab: next · Enter: next / save · Ctrl+N: create room · Esc: CANCEL", MUTED))
        return _frame(lines, cols, rows)
