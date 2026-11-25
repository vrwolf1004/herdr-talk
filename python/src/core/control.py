"""The control channel: a line-by-line JSON socket on 127.0.0.1 for the GUI and the Rust edition.

Moved out of talk.py (no screen code here). The first line is {"token": ...}; then one request line, one answer line."""

import hmac
import json
import os
import secrets
import socket
import threading

from core import relay_settings
from core import sessions
from core.chat import LOOP_POLL_SECONDS
from core.net import STOP


# ---------------------------------------------------------------- 제어 채널
# Rust 판 (control.rs) 과 같은 형식: 127.0.0.1 위의 줄 단위 JSON. 첫 줄은 {"token": ...}, 이후 요청마다 한 줄 답.
# 요청: {"cmd": "lines"|"peers"|"send"|"shutdown"}. port 와 token 은 상태 폴더의 control-<pid>.json 에 적는다.

CONTROL_MAX_REQUEST_BYTES = 64 * 1024
CONTROL_MAX_CLIENTS = 4
CONTROL_IDLE_SECONDS = 300
CONTROL_LINES_SHOWN = 300



def control_answer(chat, request, allow_shutdown):
    cmd = request.get("cmd")
    if cmd == "lines":
        with chat.lines_lock:
            rows = list(chat.lines[-CONTROL_LINES_SHOWN:])
        return {"ok": True, "lines": [{"text": f"{sender} > {text}" if sender else text, "mine": mine} for text, mine, sender, _ in rows]}
    if cmd == "peers":
        peers = [{"name": info["name"], "ip": info["ip"], "port": info["port"]} for info in chat.fresh_peers().values()]
        mode = relay_settings.SUPABASE if chat.supabase else relay_settings.LOCAL
        return {"ok": True, "peers": peers, "room": list(chat.room_names), "me": {"name": chat.me["name"], "port": chat.me["port"]}, "mode": mode, "relay": chat.relay_status}
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


