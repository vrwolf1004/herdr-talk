"""The two relay screens of the Python edition as plain state machines (the terminal loop is in talk.py):

  FirstRun      the connection mode choice shown before the chat when no mode was ever chosen (30-second default)
  SettingsForm  Ctrl+R: the connection mode and the four Supabase settings on one screen

Same rules as rust/crates/herdr-talk/src/first_run.rs and settings_form.rs. A key value is never drawn: only its length.
Keys arrive as single characters from read_char(): arrow keys are not available, so Tab moves, Space changes the mode.
"""

import queue
import threading
import time

import relay_key
import relay_rest
import relay_settings
from i18n import t

TIMEOUT_SECONDS = 30
FIELD_LABELS = ("Project URL", "Publishable key", "Display name", "Room id (8 characters)")
FIELD_NAMES = ("url", "key", "name", "room_code")
KEY_FIELD = 1
ROOM_FIELD = 3


def _row(row, text, style=""):
    return f"\x1b[{row};1H\x1b[2K{style}{text}\x1b[0m"


def _cut(text, cols):
    return text if len(text) <= cols else text[: max(0, cols - 1)] + "…"


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
        mark = [">", " "] if self.selected == 0 else [" ", ">"]
        lines = [
            (t("mode.first_run_title"), "\x1b[1m"),
            ("", ""),
            (f"{mark[0]} 1. {t('mode.local')}", "\x1b[1m" if self.selected == 0 else ""),
            (f"      {t('mode.hint_local')}", "\x1b[90m"),
            (f"{mark[1]} 2. {t('mode.supabase')}", "\x1b[1m" if self.selected == 1 else ""),
            (f"      {t('mode.hint_supabase')}", "\x1b[90m"),
            ("", ""),
            ("1 / 2, or Tab then Enter · Esc: quit", "\x1b[90m"),
        ]
        left = self.remaining()
        if left is not None:
            lines.append((t("mode.timeout_hint", n=left), ""))
        out = ["\x1b[?25l\x1b[2J"]
        out += [_row(index + 1, _cut(text, cols - 1), style) for index, (text, style) in enumerate(lines[: rows - 1])]
        return "".join(out)


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
    """Mode row, then the four fields. Enter moves down and saves on the last row; Esc leaves without saving.
    Ctrl+N asks the relay for a new room and puts its code in the room field (saved with Enter)."""

    def __init__(self, state, mine=None):
        self.state = state
        saved = relay_settings.load_saved(state)
        self.mode = saved["mode"]
        self.fields = [saved[name] for name in FIELD_NAMES]
        self.from_env = relay_settings.from_environment()
        self.focus = 0
        self.message = ""
        self.mine = mine if mine is not None else self._own_keys(state)
        self._room_result = None  # a queue of the running create-room request, None when none runs

    @staticmethod
    def _own_keys(state):
        """(Ed25519 public key hex, Noise public key hex) of this PC, or None when they cannot be read yet."""
        try:
            import ed25519
            return ed25519.public_key(relay_key.load_or_create(state)).hex(), relay_key.noise_public_hex(state)
        except (relay_key.SigningKeyError, OSError, ValueError, AttributeError):
            return None

    def open_supabase(self):
        """Start on the first field with Supabase selected (the choice made on the first-run screen)."""
        self.mode = relay_settings.SUPABASE
        self.focus = 1

    def settings(self):
        return {"mode": self.mode, **dict(zip(FIELD_NAMES, self.fields))}

    def toggle_mode(self):
        self.mode = relay_settings.LOCAL if self.mode == relay_settings.SUPABASE else relay_settings.SUPABASE

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
        on_mode = self.focus == 0
        if ch == "\x1b":
            return "cancel"
        if ch == "\t":
            self.focus = (self.focus + 1) % (len(FIELD_NAMES) + 1)
        elif ch == " " and on_mode:
            self.toggle_mode()
        elif ch in ("\r", "\n"):
            if self.focus < len(FIELD_NAMES):
                self.focus += 1
            elif self.save():
                return "saved"
        elif ch == "\x0e":
            self.create_room()
        elif ch in ("\x08", "\x7f"):
            if not on_mode:
                self.fields[self.focus - 1] = self.fields[self.focus - 1][:-1]
        elif ch.isprintable() and not on_mode:
            self.fields[self.focus - 1] += ch
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
        choice = f"[{t('mode.local')}]  {t('mode.supabase')}" if local else f"{t('mode.local')}  [{t('mode.supabase')}]"
        lines = [
            (f"{t('settings.mode_row')}", "\x1b[1m"),
            (f"{'>' if self.focus == 0 else ' '} {choice}", ""),
            (f"    {t('mode.hint_local') if local else t('mode.hint_supabase')}", "\x1b[90m"),
            ("", ""),
        ]
        for index, label in enumerate(FIELD_LABELS):
            marker = ">" if self.focus == index + 1 else " "
            lines.append((f"{marker} {label}: {self._shown(index)}", "\x1b[90m" if local else ""))
        lines.append(("", ""))
        if self.mine:
            lines.append((f"This PC, Ed25519 key: {self.mine[0]}", ""))
            lines.append((f"This PC, Noise key:   {self.mine[1]}", ""))
        lines.append((self.message, ""))
        lines.append(("", ""))
        lines.append(("Tab: next · Space: change mode · Enter: next / save · Ctrl+N: create room · Esc: leave without saving", "\x1b[90m"))
        out = ["\x1b[?25l\x1b[2J"]
        out += [_row(index + 1, _cut(text, cols - 1), style) for index, (text, style) in enumerate(lines[: rows - 1])]
        return "".join(out)
