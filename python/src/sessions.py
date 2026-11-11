"""Node folders (sessions) of the Python edition. Same rules as rust/crates/talk-core/src/store.rs.

Every run or serve starts a new node: a new folder %LOCALAPPDATA%\\herdr-talk\\sessions\\<id> (or under
HERDR_PLUGIN_STATE_DIR), with its own keys, short ID, trusted list and relay.json. --session <id> reuses an existing
folder; "default" is the base folder itself (the state of earlier versions, never changed here).
"""

import os
import shutil

import relay_key
import relay_rest

DEFAULT_SESSION = "default"

_chosen = None  # (id, folder) once choose_session has run; the first choice counts


def base_dir():
    """HERDR_PLUGIN_STATE_DIR, else LOCALAPPDATA/herdr-talk, else ~/.local/state/herdr-talk."""
    folder = os.environ.get("HERDR_PLUGIN_STATE_DIR")
    if folder:
        return folder
    base = os.environ.get("LOCALAPPDATA") or os.path.join(os.environ.get("HOME", ""), ".local", "state")
    return os.path.join(base, "herdr-talk")


def sessions_dir(base):
    return os.path.join(base, "sessions")


def new_session(base):
    """A fresh id and its new folder (created now). Returns (id, folder)."""
    while True:
        session_id = relay_key.new_short_id()
        folder = os.path.join(sessions_dir(base), session_id)
        if not os.path.exists(folder):
            os.makedirs(folder)
            return session_id, folder


def existing_session(base, session_id):
    """An existing session by id; "default" is the base folder. ValueError for a bad id or a missing folder."""
    if session_id == DEFAULT_SESSION:
        return session_id, base
    if not relay_rest.valid_short(session_id):
        raise ValueError("session id is 8 lowercase letters or digits, or default")
    folder = os.path.join(sessions_dir(base), session_id)
    if not os.path.isdir(folder):
        raise ValueError(f"no session {session_id} (see herdr-talk sessions)")
    return session_id, folder


def choose_session(requested=None):
    """Use the session given by --session, or a new one when none was given. Call it once, first. Returns (id, folder)."""
    global _chosen
    if _chosen is None:
        base = base_dir()
        _chosen = new_session(base) if requested is None else existing_session(base, requested)
    return _chosen


def process_alive(pid):
    """Whether a process with this pid exists. Windows asks the OS; elsewhere os.kill(pid, 0) (which only probes on POSIX)."""
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel32.OpenProcess.restype = wintypes.HANDLE
        handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not handle:
            return ctypes.get_last_error() == 5  # ERROR_ACCESS_DENIED: it exists, but belongs to someone else
        try:
            code = wintypes.DWORD()
            if not kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
                return True
            return code.value == 259  # STILL_ACTIVE
        finally:
            kernel32.CloseHandle(handle)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except OSError:
        return True
    return True


def remove_dead_sessions(base, alive=None, remove=None, leave=None):
    """Remove the node folders under base whose node has ended without cleaning up: each has control files, and every one
    of them is for a process that has ended. A folder without a control file is kept (its node may be starting).
    This process's own session is kept: with --session it may be such a folder, about to be used again.
    Before a folder goes, leave(folder) is called when its key files are there, to delete that node's relay row (best
    effort: the folder is removed whatever leave does). Returns the ids that could not be removed (they are skipped)."""
    alive = process_alive if alive is None else alive
    remove = shutil.rmtree if remove is None else remove
    own = _chosen[0] if _chosen else None
    failed = []
    for session_id, folder in list_sessions(base):
        if session_id != own and _control_pids(folder) and not _has_live_node(folder, alive, None):
            if leave is not None and all(os.path.isfile(os.path.join(folder, name)) for name in (relay_key.KEY_FILE, relay_key.SHORT_FILE)):
                try:  # the keys are in the folder, so the row goes first
                    leave(folder)
                except Exception:  # noqa: BLE001 - a failed delete never keeps the folder
                    pass
            try:
                remove(folder)
            except OSError:
                failed.append(session_id)
    return failed


def remove_own_session(alive=None, remove=None):
    """This node's own folder, removed when the node stops. The default folder is never removed. A folder that another
    live node still uses is kept. Returns the id when the removal failed, else None."""
    if _chosen is None:
        return None
    session_id, folder = _chosen
    return _remove_session_folder(session_id, folder, os.getpid(), alive, remove)


def _remove_session_folder(session_id, folder, own_pid, alive=None, remove=None):
    alive = process_alive if alive is None else alive
    remove = shutil.rmtree if remove is None else remove
    if session_id == DEFAULT_SESSION or _has_live_node(folder, alive, own_pid):
        return None
    try:
        remove(folder)
    except OSError:
        return session_id
    return None


def _control_pids(folder):
    """The pids of the control-<pid>.json files in a folder; None when the folder cannot be read."""
    try:
        names = os.listdir(folder)
    except OSError:
        return None
    pids = []
    for name in names:
        if not (name.startswith("control-") and name.endswith(".json")):
            continue
        try:
            pids.append(int(name[len("control-"):-len(".json")]))
        except ValueError:
            continue
    return pids


def _has_live_node(folder, alive, except_pid):
    """A control file of a live process, other than except_pid. An unreadable folder counts as live, so it is kept."""
    pids = _control_pids(folder)
    if pids is None:
        return True
    return any(pid != except_pid and alive(pid) for pid in pids)


def state_dir():
    """The folder this process keeps its state in: the chosen session, else the base folder."""
    return _chosen[1] if _chosen else base_dir()


def session_id():
    return _chosen[0] if _chosen else None


def list_sessions(base):
    """The session folders under base (not the default folder), sorted by id: (id, folder) pairs."""
    root = sessions_dir(base)
    try:
        names = os.listdir(root)
    except OSError:
        return []
    found = [(name, os.path.join(root, name)) for name in names
             if relay_rest.valid_short(name) and os.path.isdir(os.path.join(root, name))]
    return sorted(found)
