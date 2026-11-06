"""The phone's address and token, owned by one node per PC: the host (방장). Same files and rules as the Rust edition
(phone.rs), so Rust and Python nodes in one state folder elect one host between them.

The host is the node that holds `phone.lock` in the state folder. The lock is an OS file lock, taken atomically and given back
by the OS when the process ends, even when it crashes. So of nodes that start together only one wins.
The host writes `phone.json` (port, token, URL candidates, its own pid and chat port) and every node on this PC reads it.
When the host ends, a node that finds the lock free takes over and keeps the token (the phone need not be set up again).
The last node to leave removes `phone.json`. Nothing here is sent over the network.
"""

import ipaddress
import json
import os
import secrets
import socket

DEFAULT_PORT = 47860
LOCK_FILE = "phone.lock"
PHONE_FILE = "phone.json"
PROBE_TIMEOUT = 0.3


def phone_file(state):
    return os.path.join(state, PHONE_FILE)


def read(state):
    """{"port", "token", "urls", "owner_pid", "owner_port"} or None."""
    try:
        with open(phone_file(state), encoding="utf-8") as f:
            value = json.load(f)
        port, token, urls = value["port"], value["token"], value["urls"]
    except (OSError, ValueError, KeyError, TypeError):
        return None
    if not isinstance(port, int) or not isinstance(token, str) or not isinstance(urls, list):
        return None
    return {"port": port, "token": token, "urls": [u for u in urls if isinstance(u, str)],
            "owner_pid": value.get("owner_pid", 0), "owner_port": value.get("owner_port", 0)}


def _write(state, phone):
    # A new file each time, so the token is never half written; only this user may read it (0o600; on Windows the profile ACL applies).
    temp = os.path.join(state, f"{PHONE_FILE}.{os.getpid()}.tmp")
    with os.fdopen(os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w", encoding="utf-8") as out:
        json.dump(phone, out)
    os.replace(temp, phone_file(state))


def new_token():
    """Six digits, 000000 to 999999, from the OS random source."""
    return f"{secrets.randbelow(1_000_000):06d}"


def regenerate_token(state):
    """A new token in phone.json, the other fields kept. False when there is no phone.json."""
    phone = read(state)
    if phone is None:
        return False
    phone["token"] = new_token()
    _write(state, phone)
    return True


def bridge_port():
    """TALK_BRIDGE_PORT as the bridge reads it, else the default."""
    try:
        return int(os.environ["TALK_BRIDGE_PORT"])
    except (KeyError, ValueError):
        return DEFAULT_PORT


def _is_virtual(ip):
    o = ip.packed
    return ip.is_link_local or (o[0] == 192 and o[1] == 168 and o[2] == 56) or (o[0] == 172 and o[1] == 17)


def rank(addresses, default, port):
    """Addresses the phone can try, best first: the default-route address, then other private ones, virtual adapters
    (VirtualBox host-only 192.168.56.x, link-local, Docker) last."""
    found = []
    for text in addresses:
        ip = ipaddress.IPv4Address(text)
        if not ip.is_loopback and not ip.is_unspecified and ip not in found:
            found.append(ip)
    default = ipaddress.IPv4Address(default) if default else None
    found.sort(key=lambda ip: (_is_virtual(ip), ip != default, not ip.is_private))
    return [f"http://{ip}:{port}" for ip in found]


def url_candidates(port):
    default = None
    probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        probe.connect(("192.0.2.1", 9))  # sends nothing: it only picks the address this PC would send from
        default = probe.getsockname()[0]
    except OSError:
        pass
    finally:
        probe.close()
    addresses = [default] if default else []
    try:
        addresses += socket.gethostbyname_ex(socket.gethostname())[2]
    except OSError:
        pass
    return rank(addresses, default, port)


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
        # Others still alive and a file left by the old host: this is a take-over, so the phone keeps its token.
        kept = read(state) if other_live_nodes(state) > 0 else None
        port = kept["port"] if kept else bridge_port()
        token = kept["token"] if kept else new_token()
        _write(state, {"port": port, "token": token, "urls": url_candidates(port), "owner_pid": os.getpid(), "owner_port": chat_port})
        return cls(fd, state)

    def release(self):
        # The file stays for a successor's token while another node is alive.
        if other_live_nodes(self.state) == 0:
            try:
                os.remove(phone_file(self.state))
            except OSError:
                pass
        if self.fd is not None:
            os.close(self.fd)  # closing the file gives the lock back
            self.fd = None
