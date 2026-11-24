"""The full-screen chat: the alternate screen, the redraw loop, the first-run and settings screens, and the idle exit.
Drawing and keys are in extend/screen.py; the chat state and the network are in core/. talk.py only assembles them."""

import os
import shutil
import sys
import threading
import time

import relay_chat
import relay_settings
from core import relay_rest
from core.chat import Chat
from core.events import Line, Notice
from core.net import STOP
from extend import settings_form as relay_form
from extend.screen import enable_vt, help_box, on_key, read_char, read_char_timeout, screen_frame, status_tag
from i18n import t


IDLE_LIMIT_SECONDS = 10 * 60   # leave automatically after this long without any chat activity

_restore_screen = None  # undoes the full-screen mode while the chat runs; leave_when_idle() calls it


def leave_when_idle(chat):
    """Exit after IDLE_LIMIT_SECONDS without activity. The full-screen mode is undone first, so the shell is left clean."""
    while True:
        time.sleep(10)
        if time.time() - chat.last_activity >= IDLE_LIMIT_SECONDS:
            if _restore_screen is not None:
                _restore_screen()
            print(t("idle.left", minutes=IDLE_LIMIT_SECONDS // 60), flush=True)
            os._exit(0)


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
        # "Peers" are the people in the room in CLOUD SERVICE, the peers found on the network in the LAN mode.
        status = t("status.line", name=chat.me["name"], peers=len(chat.room_names) if chat.supabase else len(chat.fresh_peers()))
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
        nodes = [", ".join([f"{'HOST/YOU' if chat.is_host() else 'YOU'} {chat.me['name']}", *chat.room_names])]  # CLOUD SERVICE: the people in the room follow on the same row
        for info in chat.fresh_peers().values():
            link = chat.link_of(info)
            number = chat.numbers.get(info.get("key")) if link == "unverified" else None
            prefix = f"[{number}] " if number is not None else ""
            state = t("nodes.verified") if link == "encrypted" else t("nodes.waiting")
            nodes.append(f"{prefix}{info['name']} ({info['ip']})  {state}")
        mode = "CLOUD SERVICE" if chat.supabase else "BASE STATION"
        overlay = help_box() if chat.help_open else chat.phone_overlay()
        frame = screen_frame(lines, status, typed[0], size.columns, size.lines, overlay, nodes, mode, status_tag(chat))
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


