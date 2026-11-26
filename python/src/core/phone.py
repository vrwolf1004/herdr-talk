"""The host (방장) of one PC: the node that holds `phone.lock` in the state folder. The lock is an OS file lock, taken atomically
and given back by the OS when the process ends, even when it crashes. So of nodes that start together only one wins.
The host writes `phone.json` (its own pid and chat port) and every node on this PC reads it. When the host ends, a node
that finds the lock free takes over. The last node to leave removes `phone.json`. Nothing here is sent over the network.
"""

import json
import os
import socket

LOCK_FILE = "phone.lock"
PHONE_FILE = "phone.json"
PROBE_TIMEOUT = 0.3


def phone_file(state):
    return os.path.join(state, PHONE_FILE)


def read(state):
    """{"owner_pid", "owner_port"} of the host, or None when there is no phone.json or it is damaged."""
    try:
        with open(phone_file(state), encoding="utf-8") as f:
            value = json.load(f)
        owner_port = value["owner_port"]
    except (OSError, ValueError, KeyError, TypeError):
        return None
    if not isinstance(owner_port, int):
        return None
    return {"owner_pid": value.get("owner_pid", 0), "owner_port": owner_port}


def _write(state, info):
    # A new file each time, so it is never half written.
    temp = os.path.join(state, f"{PHONE_FILE}.{os.getpid()}.tmp")
    with os.fdopen(os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w", encoding="utf-8") as out:
        json.dump(info, out)
    os.replace(temp, phone_file(state))


def other_live_nodes(state):
    """How many other nodes in this state folder are live: their control port accepts a connection."""
    count = 0
    try:
        names = os.listdir(state)
    except OSError:
        return 0
    for name in names:
        if not (name.startswith("control-") and name.endswith(".json")):
            continue
        try:
            pid = int(name[len("control-"):-len(".json")])
            with open(os.path.join(state, name), encoding="utf-8") as f:
                port = json.load(f)["port"]
        except (OSError, ValueError, KeyError, TypeError):
            continue
        if pid == os.getpid():
            continue
        try:
            socket.create_connection(("127.0.0.1", port), timeout=PROBE_TIMEOUT).close()
            count += 1
        except OSError:
            pass
    return count


def _try_lock(fd):
    if os.name == "nt":
        import msvcrt
        try:
            os.lseek(fd, 0, os.SEEK_SET)
            msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
            return True
        except OSError:
            return False
    import fcntl
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except OSError:
        return False


class Host:
    """Holds the host lock. release() gives it back and, when no other node is left, removes phone.json."""

    def __init__(self, fd, state):
        self.fd = fd
        self.state = state

    @classmethod
    def take(cls, state, chat_port):
        """The host if the lock is free, else None. chat_port is this node's chat port."""
        os.makedirs(state, exist_ok=True)
        fd = os.open(os.path.join(state, LOCK_FILE), os.O_RDWR | os.O_CREAT, 0o600)
        if not _try_lock(fd):
            os.close(fd)
            return None
        _write(state, {"owner_pid": os.getpid(), "owner_port": chat_port})
        return cls(fd, state)

    def release(self):
        # The file stays while another node is alive, so a successor can read the host's chat port.
        if other_live_nodes(self.state) == 0:
            try:
                os.remove(phone_file(self.state))
            except OSError:
                pass
        if self.fd is not None:
            os.close(self.fd)  # closing the file gives the lock back
            self.fd = None
