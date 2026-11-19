"""Room relay of the chat (Python edition, same rules as rust/crates/herdr-talk and talk-core relay_chat).

Supabase mode only. The chat joins the room in the settings (join-room) and publishes this PC's row first: the room
needs its owner row. Sending to room members is not implemented yet: a typed text is not sent and a notice says so.
Nothing is stored.

This module does not import talk.py. The chat gives it a `hooks` object with: emit(event).
"""

import os
import queue
import threading
import time

from core import ed25519, relay_key, relay_rest, room, store
from core.events import Message, Notice, Status
import relay_settings

SLICE_SECONDS = 0.2
WAIT_SECONDS = 30
POLL_SECONDS = 2         # how often the inbox is asked (Rust room_relay.rs POLL)
MEMBERS_SECONDS = 10     # how often the room is joined again for its members (Rust MEMBERS_EVERY)
DEAD_LEAVE_SECONDS = 3  # shorter than a normal request: several dead folders may wait in line


class Session:
    """This PC's keys: the Ed25519 seed (row signing, the room join) and the Noise secret (room messages)."""

    def __init__(self, seed, noise_secret=None):
        self.seed = seed
        self.noise_secret = noise_secret


def make_session(state):
    return Session(relay_key.load_or_create(state), store.load_or_create_identity(state)[1])


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
        self.members = {}  # pk hex -> Noise public key bytes, from the last join
        self.names = {}    # pk hex -> the 대화명 it was shown under (a clash gets the key's first 4 characters)
        self.since = None
        self.own_pk = None  # this PC's Ed25519 key hex, set when the relay starts
        self.started = 0    # seconds when this node's relay started (memory only): older room rows are not shown
        self.cache = room.NonceCache()

    def start(self):
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def stop(self):
        self.stop_event.set()

    def send(self, text):
        """Queue a typed text for the room (the relay thread sends it)."""
        self.outbox.put(text)

    def _send_text(self, text):
        """One text to every other member of the room, one Noise message each in one post (no /trust step in rooms;
        the Rust edition still sends to trusted members only)."""
        others = [(pk, noise) for pk, noise in self.members.items() if pk != self.own_pk]
        if not others:
            self.hooks.emit(Notice("room.send_failed", {"error": "no other member in the room"}))
            return
        try:
            ok = room.post_text(self.config, self.session.seed, self.settings["room_code"], self.session.noise_secret,
                                others, self.hooks.my_name(), text)
        except (OSError, ValueError) as error:
            self.hooks.emit(Notice("room.send_failed", {"error": str(error)}))
            return
        if not ok:
            self.hooks.emit(Notice("room.send_failed", {"error": "server"}))
            return
        self.hooks.emit(Message(self.hooks.my_name(), text, mine=True))

    def _fetch(self):
        """Show the inbox rows that check out (Rust fetch_round + show_message)."""
        rows = room.fetch_rows(self.config, self.session.seed, self.settings["room_code"], self.since)
        for row in rows:
            self.since = row["created_at"]
            shown = room.check_row(row, self.members, self.session.noise_secret, self.cache, int(time.time()),
                                   not_before=self.started)
            if shown is None:
                continue
            name, text = shown
            self.names[row["sender_pk"]] = name
            self.hooks.emit(Message(room.display_name(self.names, row["sender_pk"], name), text))

    def _load_members(self):
        """Read the room's member keys and their Noise keys (the peers table)."""
        info = relay_rest.join_room(self.config, self.session.seed, self.settings["room_code"], self.settings["name"], self.settings["trust"])
        pks = [pk for pk in info["members"] if pk != self.own_pk]
        noise_keys = {}
        if pks:
            status, body = relay_rest._send(relay_rest.members_noise_request(self.config, pks))
            noise_keys = {pk: bytes.fromhex(noise) for pk, noise in relay_rest.read_members_noise(status, body)}
        self.members = {pk: noise_keys[pk] for pk in pks if pk in noise_keys}
        return info

    def _wait(self, seconds):
        """Sleep in slices so a stop is seen. Sends queued meanwhile are sent in the slice."""
        until = time.monotonic() + seconds
        while time.monotonic() < until and not self.stop_event.is_set():
            while True:
                try:
                    text = self.outbox.get_nowait()
                except queue.Empty:
                    break
                self._send_text(text)
            time.sleep(SLICE_SECONDS)

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
        self.started = int(time.time())  # rows older than this node's start are not shown
        # The row is written before the join (the room's member rows point at it): until it is, the join is not tried.
        name = self.settings["name"]
        if name and not relay_rest.valid_name(name):
            # A bad name would never get its row, so the join would fail on the missing row: stop here with the reason.
            self.hooks.emit(Status("relay: name is not valid; not joined yet"))
            return
        if relay_rest.valid_name(name):
            while not self._publish():
                self.hooks.emit(Status("relay: could not publish this PC's row; not joined yet"))
                self._wait(WAIT_SECONDS)
                if self.stop_event.is_set():
                    return
        self.own_pk = ed25519.public_key(self.session.seed).hex()
        joined = False
        retries = 1  # a failed join is tried once more, after MEMBERS_SECONDS; then no more tries
        next_members = next_fetch = 0.0
        while not self.stop_event.is_set():
            now = time.monotonic()
            if now >= next_members:
                next_members = now + MEMBERS_SECONDS
                try:
                    info = self._load_members()
                    joined = True
                    retries = 1
                    self.hooks.emit(Status(""))  # a successful join says nothing on the status line; failures still do
                except (OSError, ValueError) as error:
                    joined = False
                    self.hooks.emit(Status(f"relay: room join failed: {error}"))
                    if not retries:
                        next_members = float("inf")  # shortcut: a second failure stops the joins until the relay restarts
                    retries -= 1
            if joined and now >= next_fetch:
                next_fetch = now + POLL_SECONDS
                try:
                    self._fetch()
                except (OSError, ValueError) as error:
                    if joined:
                        self.hooks.emit(Status(f"relay: {error}"))
            self._wait(SLICE_SECONDS)

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
    if not relay_rest.valid_trust(settings.get("trust")):
        return None, "relay: trust code missing or invalid (8 digits)"
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


def room_refuses(state):
    """True when the saved room refuses this trust code (join-room answers 403): the chat must not open the room then. Any
    other failure (offline, no keys yet) is not a refusal: the chat starts as before and shows it. Same rule as talk-core
    relay_chat::room_refuses."""
    settings = relay_settings.effective(state)
    if settings["mode"] != relay_settings.SUPABASE or not settings["room_code"]:
        return False
    try:
        config = relay_rest.config(settings["url"], settings["key"])
        seed = relay_key.load_or_create(state)
        relay_rest.join_room(config, seed, settings["room_code"], settings["name"], settings["trust"], timeout=3)  # shortcut: 3 seconds fixed; a timeout is not a refusal
    except ValueError as error:
        return str(error) == relay_rest.TRUST_WRONG
    except OSError:
        return False
    return False
