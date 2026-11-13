"""Relay settings and the connection mode (same rules as rust/crates/talk-core/src/relay_settings.rs).

Saved in relay.json in the state folder. HERDR_TALK_RELAY_URL and HERDR_TALK_RELAY_KEY win over the saved URL and key
when set and not empty. The key is never printed: screens show its length only.
Mode: "local" (LAN, the default) or "supabase" (messages only through the relay). Only the exact word "supabase"
selects Supabase; a missing or unknown value means local.
"""

import json
import os

import relay_rest

FILE = "relay.json"
LOCAL = "local"
SUPABASE = "supabase"
_FIELDS = ("url", "key", "name", "room_code")


def empty():
    return {"mode": LOCAL, "url": "", "key": "", "name": "", "room_code": ""}


def _read(state):
    """The parsed relay.json (a dict), or None when missing or damaged. A UTF-8 byte order mark is allowed."""
    try:
        with open(os.path.join(state, FILE), encoding="utf-8-sig") as handle:
            value = json.load(handle)
    except (OSError, ValueError):
        return None
    return value if isinstance(value, dict) else None


def needs_first_run(state):
    """True before the mode has been chosen: no file, a damaged one, or one without a `mode` key."""
    value = _read(state)
    return value is None or "mode" not in value


def load_saved(state):
    """The settings as saved. Missing or damaged file: empty settings (local, relay off)."""
    value = _read(state)
    if value is None:
        return empty()
    def field(name):
        item = value.get(name)
        return item if isinstance(item, str) else ""
    settings = {name: field(name) for name in _FIELDS}
    settings["mode"] = SUPABASE if value.get("mode") == SUPABASE else LOCAL
    return settings


def _env(name):
    value = os.environ.get(name, "").strip()
    return value or None


def effective(state, env_url=None, env_key=None):
    """The settings the relay uses: a non-empty environment value replaces the saved URL or key.
    env_url and env_key (tests) replace reading the environment."""
    settings = load_saved(state)
    # A room code saved by an older version may be lowercase: the relay uses it uppercase (the input rule).
    settings["room_code"] = settings["room_code"].upper()
    url = env_url if env_url is not None else _env(relay_rest.URL_VARIABLE)
    key = env_key if env_key is not None else _env(relay_rest.KEY_VARIABLE)
    if url:
        settings["url"] = url
    if key:
        settings["key"] = key
    return settings


def from_environment():
    """(url from the environment, key from the environment): the screen says so."""
    return _env(relay_rest.URL_VARIABLE) is not None, _env(relay_rest.KEY_VARIABLE) is not None


def validate(settings):
    """None when the settings can be saved, else the reason. Empty fields are allowed in local mode."""
    if settings["mode"] == SUPABASE and any(not settings.get(name) for name in _FIELDS):
        return "Supabase mode needs the URL, the key, the display name and the room code"
    if settings["name"] and not relay_rest.valid_name(settings["name"]):
        return "name must be 1 to 32 characters without control characters"
    if settings.get("room_code") and not relay_rest.valid_room_code(settings["room_code"]):
        return "room code must be 8 uppercase letters or digits"
    if settings["mode"] == SUPABASE:
        try:
            if relay_rest.config(settings["url"], settings["key"]) is None:
                return "URL and key are both needed"
        except ValueError as error:
            return str(error)
    return None


def save(state, settings):
    """Write relay.json whole through a temporary file (without a byte order mark). Returns None or the reason."""
    # The input rule: the room code is saved uppercase.
    settings = {**settings, "room_code": settings.get("room_code", "").upper()}
    reason = validate(settings)
    if reason:
        return reason
    try:
        os.makedirs(state, exist_ok=True)
        text = json.dumps({"mode": settings["mode"], **{name: settings.get(name, "") for name in _FIELDS}}, ensure_ascii=False)
        temp = os.path.join(state, f"{FILE}.{os.getpid()}.tmp")
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(text)
        os.replace(temp, os.path.join(state, FILE))
    except OSError as error:
        return str(error)
    return None


def set_mode(state, mode):
    """Change only the mode and keep every other saved value. Returns None or the reason."""
    settings = load_saved(state)
    settings["mode"] = SUPABASE if mode == SUPABASE else LOCAL
    return save(state, settings)


def key_label(key):
    """What the screen shows for the key: its length only."""
    return "(%d chars)" % len(key) if key else "-"
