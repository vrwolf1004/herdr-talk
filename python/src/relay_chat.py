"""Room relay of the chat (Python edition, same rules as rust/crates/herdr-talk and talk-core relay_chat).

Supabase mode only. The chat joins the room in the settings (join-room) and publishes this PC's row first: the room
needs its owner row. Sending to room members is not implemented yet: a typed text is not sent and a notice says so.
Nothing is stored.

This module does not import talk.py. The chat gives it a `hooks` object with: say(text), set_status(text).
"""

import os
import queue
import threading
import time

import relay_key
import relay_rest
import relay_settings

SLICE_SECONDS = 0.2
WAIT_SECONDS = 30
DEAD_LEAVE_SECONDS = 3  # shorter than a normal request: several dead folders may wait in line


class Session:
    """The Ed25519 seed of this PC (row signing and the room join)."""

    def __init__(self, seed):
        self.seed = seed


def make_session(state):
    return Session(relay_key.load_or_create(state))


class Relay:
    """The relay thread of one chat. start() replaces nothing: the chat stops the old one first."""

    def __init__(self, hooks, state, config, settings, session):
        self.hooks = hooks
        self.state = state
        self.config = config
        self.settings = settings
        self.session = session
        self.outbox = queue.Queue()
        self.stop_event = threading.Event()
        self.thread = None

    def start(self):
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def stop(self):
        self.stop_event.set()

    def send(self, text):
        """Queue a typed text. Room messages are not sent yet, so the relay thread only tells the user."""
        self.outbox.put(text)

    def _wait(self, seconds):
        """Sleep in slices so a stop is seen. Texts typed meanwhile are not sent: a notice says so."""
        until = time.monotonic() + seconds
        while time.monotonic() < until and not self.stop_event.is_set():
            while True:
                try:
                    self.outbox.get_nowait()
                except queue.Empty:
                    break
                self.hooks.say("relay: room messages are not sent yet, the message was not sent")
            time.sleep(SLICE_SECONDS)

    def _join_room(self):
        """Join the room once (join-room). Returns the status text."""
        settings = self.settings
        try:
            info = relay_rest.join_room(self.config, self.session.seed, settings["room_code"], settings["name"])
        except (OSError, ValueError) as error:
            return f"relay: room join failed: {error}"
        # The member count comes from the join answer (read_join_room checks the list). A malformed answer is a failure above.
        return f"relay: 방 참가 (멤버 {len(info['members'])}명)"

    def _publish(self):
        """Write this PC's row. A 409 means another key owns the short id: make a new one and try again (up to 5)."""
        for _ in range(5):
            short = relay_key.load_or_create_short_id(self.state)
            status = relay_rest.put_status(self.config, self.session.seed, relay_key.noise_public_hex(self.state), short, self.settings["name"])
            if status is not None and relay_rest.put_succeeded(status):
                return True
            if status != 409:
                return False
            relay_key.replace_short_id(self.state, relay_key.new_short_id())
        return False

    def _run(self):
        if relay_rest.valid_name(self.settings["name"]) and not self._publish():
            self.hooks.set_status("relay: could not publish this PC's row")
        self.hooks.set_status(self._join_room())
        while not self.stop_event.is_set():
            self._wait(WAIT_SECONDS)


def start(hooks, state, settings):
    """Start the room relay for `settings`. Returns (Relay or None, status text). None: the relay is off (local mode, or
    the settings are empty/incomplete). Never raises for a bad setting: the status says why."""
    if settings["mode"] != relay_settings.SUPABASE:
        return None, ""
    if not settings["url"] and not settings["key"] and not settings.get("room_code"):
        return None, ""
    try:
        config = relay_rest.config(settings["url"], settings["key"])
    except ValueError as error:
        return None, f"relay: {error}"
    if config is None:
        return None, "relay: settings incomplete"
    if not relay_rest.valid_room_code(settings.get("room_code")):
        return None, "relay: room code missing or invalid (8 uppercase letters or digits)"
    try:
        session = make_session(state)
    except (relay_key.SigningKeyError, OSError) as error:
        return None, f"relay: {error}"
    relay_thread = Relay(hooks, state, config, settings, session)
    relay_thread.start()
    return relay_thread, ""


def leave(state, timeout=relay_rest.TIMEOUT_SECONDS):
    """On exit: delete this node's own relay row. Quiet and best effort: no relay, no keys or no short id yet means nothing
    to do; a failed request is ignored (the row then stays for the owner to clear). Returns True when the row was deleted.
    Also for the folder of a node that ended without cleaning up, then with the shorter DEAD_LEAVE_SECONDS."""
    settings = relay_settings.load_saved(state)
    if settings["mode"] != relay_settings.SUPABASE:
        return False
    try:
        config = relay_rest.config(settings["url"], settings["key"])
    except ValueError:
        return False
    seed_path = os.path.join(state, relay_key.KEY_FILE)
    short_path = os.path.join(state, relay_key.SHORT_FILE)
    if config is None or not os.path.exists(seed_path) or not os.path.exists(short_path):
        return False
    seed = relay_key.load_or_create(state)
    short = relay_key.load_or_create_short_id(state)
    return relay_rest.delete_peer(config, seed, short, timeout)
