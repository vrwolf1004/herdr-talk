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
FIELD_LABELS = ("Setup code", "Trust code (8 digits)", "Display name", "Room id (8 characters)")  # English; row 0 is t()
CODE_FIELD = 0
KEY_F2 = "\x00<"  # F2 on Windows: a 0 code, then <. Saves the setup code (middle save) on the setup-code row.
KEY_F9 = "\x00C"  # F9 on Windows: a 0 code, then C. Hidden: opens the operator passphrase prompt (issue a setup code).
TRUST_FIELD = 1
NAME_FIELD = 2
ROOM_FIELD = 3


def _labels():
    return (t("settings.setup_code"),) + FIELD_LABELS[1:]


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
            *_menu_item(self.selected == 0, "01. BASE STATION", t("mode.hint_local")),
            ("", ""),
            *_menu_item(self.selected == 1, "02. CLOUD SERVICE", t("mode.hint_supabase")),
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
    """The four fields: setup code, display name, room id, trust code. Enter moves down; on the last row it saves. A typed
    setup code is redeemed in a background thread first: the first Enter starts it, the next Enter saves (the same rhythm
    as Ctrl+N). Esc leaves without saving. Ctrl+N asks the relay for a new room and puts its code in the room field.
    The trust code is the owner's choice at create and the value each participant types to join."""

    def __init__(self, state, mine=None):
        self.state = state
        saved = relay_settings.load_saved(state)
        self.mode = saved["mode"]
        self.url = saved["url"]  # saved values; a setup code replaces them on redeem
        self.key = saved["key"]
        self.fields = ["", saved["trust"], saved["name"], saved["room_code"]]
        self.from_env = relay_settings.from_environment()
        self.focus = 0  # index into fields
        self.message = ""
        self.mine = mine if mine is not None else self._own_keys(state)
        self._room_result = None  # a queue of the running create-room request, None when none runs
        self._redeem_result = None  # a queue of the running setup-code redeem, None when none runs
        self._issue_result = None  # a queue of the running operator code issue, None when none runs
        self._operator = None  # the hidden operator passphrase typed so far; None while the prompt is closed
        self._redeem_code = None  # the code the running redeem is for
        self.redeemed_code = None  # the code whose url and key are in self.url and self.key (not saved yet)
        self.left = False  # True on the screen opened after leaving a room: another room may be entered or created

    @staticmethod
    def _own_keys(state):
        """(Ed25519 public key hex, Noise public key hex) of this PC, or None when they cannot be read yet."""
        try:
            from core import ed25519
            return ed25519.public_key(relay_key.load_or_create(state)).hex(), relay_key.noise_public_hex(state)
        except (relay_key.SigningKeyError, OSError, ValueError, AttributeError):
            return None

    def open_supabase(self, left=False):
        """Start on the first field with Supabase selected (the choice made on the first-run screen). left: the screen is
        shown after leaving a room, so the one-room rule does not apply and the saved server info stays in the fields."""
        self.mode = relay_settings.SUPABASE
        self.focus = 0
        self.left = left

    def save(self):
        """Write relay.json. True when saved; otherwise the message says what is missing or wrong. A typed setup code is
        redeemed first (in the background): this call then only starts that and returns False."""
        if self._redeem_result is not None:
            return False
        typed = self.fields[CODE_FIELD].strip()
        if typed:
            code = relay_rest.normalize_setup_code(typed)
            if code is None:
                self.message = t("settings.code_invalid")
                return False
            if code != self.redeemed_code:
                self._redeem(code)
                return False
        saved_room = relay_settings.load_saved(self.state)["room_code"]
        new_room = self.fields[ROOM_FIELD].strip().upper()
        if saved_room and new_room and new_room != saved_room and not self.left:
            self.message = t("room.only_one", code=saved_room)
            return False
        if self.mode == relay_settings.SUPABASE:
            if not (self.url and self.key):
                self.message = t("settings.need_code")
                return False
            missing = [label for label, value in zip(_labels()[1:], self.fields[1:]) if not value.strip()]
            if missing:
                self.message = t("settings.save_refused_missing", fields=", ".join(missing))
                return False
        # The input rule: the room code is saved uppercase; the name and the trust code keep their text, trimmed.
        values = {"mode": self.mode, "url": self.url, "key": self.key, "name": self.fields[NAME_FIELD].strip(),
                  "setup_code": self.redeemed_code or relay_settings.load_saved(self.state)["setup_code"],
                  "room_code": new_room, "trust": self.fields[TRUST_FIELD].strip()}
        reason = relay_settings.save(self.state, values)
        if reason:
            self.message = f"not saved: {reason}"
            return False
        self.fields[CODE_FIELD] = ""
        self.message = t("settings.save_ok_supabase" if values["mode"] == relay_settings.SUPABASE else "settings.save_ok_local")
        return True

    def on_key(self, ch):
        """None while editing, "saved" after a successful save, "cancel" for Esc."""
        if not ch:
            return None
        if self._operator is not None:  # the hidden operator prompt: passphrase, Enter issues, Esc cancels
            if ch == "\x1b":
                self._operator = None
                self.message = ""
            elif ch in ("\r", "\n"):
                self._start_issue(self._operator)
                self._operator = None
            elif ch in ("\x08", "\x7f"):
                self._operator = self._operator[:-1]
            elif ch.isprintable():
                self._operator += ch
            return None
        if ch == KEY_F9:
            self._operator = ""
            self.message = "operator passphrase: Enter issues a setup code, Esc cancels"
            return None
        if ch == "\x1b":
            return "cancel"
        if ch == "\t":
            self.focus = (self.focus + 1) % len(FIELD_LABELS)
        elif ch == KEY_F2:  # F2: the middle save, only on the setup-code row
            if self.focus == CODE_FIELD:
                self._begin_redeem()
            else:
                self.message = t("settings.ctrl_s_only_code")
        elif ch in ("\r", "\n"):
            if self.focus < len(FIELD_LABELS) - 1:
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

    def _begin_redeem(self):
        """F2 on the setup-code row: redeem the typed code now, so the URL and key are saved at once (no room needed)."""
        typed = self.fields[CODE_FIELD].strip()
        if not typed or self._redeem_result is not None:
            return
        code = relay_rest.normalize_setup_code(typed)
        if code is None:
            self.message = t("settings.code_invalid")
        elif code != self.redeemed_code:
            self._redeem(code)

    def _start_issue(self, passphrase):
        """Ask the server for a new setup code in a background thread (the passphrase is checked there)."""
        result = queue.Queue()

        def work():
            try:
                result.put(("ok", relay_rest.issue_setup_code(passphrase)))
            except (OSError, ValueError) as error:
                result.put(("error", str(error)))

        self._issue_result = result
        self.message = "issuing a setup code..."
        threading.Thread(target=work, daemon=True).start()

    def _redeem(self, code):
        """Redeem the setup code in a background thread, so the screen keeps working. poll() applies the answer."""
        result = queue.Queue()

        def work():
            try:
                result.put(("ok", relay_rest.redeem_setup_code(code, relay_rest.setup_url())))
            except (OSError, ValueError) as error:
                result.put(("error", str(error)))

        self._redeem_result = result
        self._redeem_code = code
        threading.Thread(target=work, daemon=True).start()
        self.message = t("settings.code_working")

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
        saved_room = relay_settings.load_saved(self.state)["room_code"]
        if saved_room and not self.left:
            self.message = t("room.only_one", code=saved_room)
            return
        saved = relay_settings.effective(self.state)  # the URL and key the relay uses, environment included
        name = self.fields[NAME_FIELD].strip()
        if not (saved["url"] and saved["key"] and name):
            self.message = "create room: save the setup code and the display name first"
            return
        trust = self.fields[TRUST_FIELD].strip()
        if not relay_rest.valid_trust(trust):
            self.message = "create room: the trust code must be 8 digits"
            return
        try:
            config = relay_rest.config(saved["url"], saved["key"])
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
                result.put(("ok", relay_rest.create_room(config, seed, name, trust)))
            except (OSError, ValueError) as error:
                result.put(("error", str(error)))

        self._room_result = result
        threading.Thread(target=work, daemon=True).start()
        self.message = "create room: working..."

    def poll(self):
        """Apply the redeem and create-room answers when they have come back. True when the screen must be drawn again."""
        if self._redeem_result is not None:
            try:
                kind, value = self._redeem_result.get_nowait()
            except queue.Empty:
                pass
            else:
                self._redeem_result = None
                if kind == "ok":
                    self.url, self.key = value
                    self.redeemed_code = self._redeem_code
                    reason = relay_settings.save_connection(self.state, self.url, self.key, self._redeem_code)
                    self.message = t("settings.code_ok") if reason is None else f"not saved: {reason}"
                else:
                    self.message = t("settings.code_failed", error=value)
                return True
        if self._issue_result is not None:
            try:
                kind, value = self._issue_result.get_nowait()
            except queue.Empty:
                pass
            else:
                self._issue_result = None
                self.message = (f"new setup code: {value} (20 uses, 7 days; write it down now)" if kind == "ok"
                                else f"issue failed: {value}")
                return True
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
        if index == CODE_FIELD:
            if self.fields[CODE_FIELD]:
                return self.fields[CODE_FIELD]
            if self.from_env[0] or self.from_env[1]:
                return "(environment)"
            return t("settings.code_saved", n=len(self.key)) if self.key else "-"
        return self.fields[index]

    def render(self, cols, rows):
        local = self.mode != relay_settings.SUPABASE
        lines = [
            (header(cols, "SYS.VER 1.0"), ACCENT),
            (divider(cols), BORDER),
            ("SUPABASE SETTINGS", ACCENT + BOLD),
            ("", ""),
        ]
        for index, label in enumerate(_labels()):
            value = self._shown(index)
            fill = "_" * max(0, FIELD_WIDTH - char_width(value))  # the blank input, as an underscore line
            selected = self.focus == index
            lines.append((f"{'▶ ' if selected else '  '}{label:<22} : [{value}{fill}]", ACCENT if selected else (MUTED if local else "")))
            if index == CODE_FIELD:
                lines.append(("     Type the setup code, then F2: the URL and key are saved (not shown)", MUTED))
            if index == ROOM_FIELD:
                lines.append(("     Join: type a room ID here, then Enter  ·  New room: press Ctrl+N", MUTED))
        if self._operator is not None:
            lines.append((f"operator passphrase: [{'*' * len(self._operator)}]", ACCENT))
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
