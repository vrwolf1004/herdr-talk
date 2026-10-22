"""herdr 호출. CLI(herdr 바이너리)와 socket API 를 함께 쓴다.

workspace.move 처럼 CLI 가 없는 요청, 그리고 애니메이션처럼 1초마다 보내는 요청은
프로세스를 띄우지 않도록 socket 으로 직접 보낸다.
socket 은 Unix 에서는 Unix domain socket, Windows 에서는 같은 경로 이름의 named pipe 다.
"""

import json
import os
import re
import socket
import subprocess
import sys
import time

IS_WINDOWS = sys.platform == "win32"
SOCKET_TIMEOUT_SECONDS = 3


class HerdrError(Exception):
    """herdr 호출 실패."""


def herdr_bin():
    return os.environ.get("HERDR_BIN_PATH") or "herdr"


def socket_path():
    path = os.environ.get("HERDR_SOCKET_PATH")
    if path:
        return path
    if IS_WINDOWS:
        appdata = os.environ.get("APPDATA")
        if not appdata:
            raise OSError("APPDATA is not set; set HERDR_SOCKET_PATH to the herdr socket")
        return os.path.join(appdata, "herdr", "herdr.sock")
    return os.path.expanduser("~/.config/herdr/herdr.sock")


def cli(*args, check=True):
    """herdr CLI 를 실행하고 JSON 응답의 result 를 돌려준다. check=False 면 실패 시 None."""
    completed = subprocess.run(
        [herdr_bin(), *[str(arg) for arg in args]],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode != 0:
        if not check:
            return None
        detail = (completed.stdout + completed.stderr).strip()
        raise HerdrError(f"herdr {' '.join(map(str, args))}: {detail}")
    output = completed.stdout.strip().lstrip("﻿")
    if not output:
        return {}
    return json.loads(output).get("result", {})


# ---------------------------------------------------------------- socket


def _read_line(read):
    buffer = b""
    while not buffer.endswith(b"\n"):
        chunk = read(65536)
        if not chunk:
            break
        buffer += chunk
    return buffer


def _exchange_unix(path, payload):
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(SOCKET_TIMEOUT_SECONDS)
    try:
        client.connect(path)
        client.sendall(payload)
        return _read_line(client.recv)
    finally:
        client.close()


def _exchange_pipe(path, payload):
    # Windows named pipe 이름에 ':' 가 들어 있어 open() 으로는 열 수 없다. CreateFileW 로 연다.
    import ctypes
    import msvcrt
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.CreateFileW.argtypes = [
        wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
        wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE,
    ]
    generic_read_write = 0xC0000000
    open_existing = 3
    handle = kernel32.CreateFileW("\\\\.\\pipe\\" + path, generic_read_write, 0, None, open_existing, 0, None)
    if handle is None or handle == wintypes.HANDLE(-1).value:
        raise OSError(f"named pipe open failed ({ctypes.get_last_error()})")
    fd = msvcrt.open_osfhandle(handle, os.O_RDWR | os.O_BINARY)
    try:
        pipe = os.fdopen(fd, "r+b", buffering=0)
    except OSError:
        os.close(fd)  # fd 가 handle 을 가지고 있으므로 닫으면 handle 도 닫힌다
        raise
    with pipe:
        pipe.write(payload)
        return _read_line(pipe.read)


def request(method, params):
    """socket API 요청을 보내고 result 를 돌려준다."""
    message = {"id": f"timekit:{time.time_ns()}", "method": method, "params": params}
    payload = (json.dumps(message, ensure_ascii=False) + "\n").encode("utf-8")
    exchange = _exchange_pipe if IS_WINDOWS else _exchange_unix
    try:
        raw = exchange(socket_path(), payload)
    except OSError as error:
        raise HerdrError(f"{method}: {error}") from error
    response = json.loads(raw.decode("utf-8")) if raw else {}
    if "error" in response:
        raise HerdrError(f"{method}: {response['error']}")
    return response.get("result", {})


def try_request(method, params):
    """실패해도 루프를 멈추지 않을 요청. 성공 여부만 돌려준다."""
    try:
        request(method, params)
        return True
    except (HerdrError, ValueError):
        return False


def installed_plugin_path(plugin_id):
    """herdr plugin list 에서 활성화된 local plugin 의 폴더. 없거나 비활성이거나 GitHub 설치본이면 None."""
    try:
        completed = subprocess.run(
            [herdr_bin(), "plugin", "list"],
            stdin=subprocess.DEVNULL, capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
    except OSError:
        return None
    pattern = re.compile(r"^- " + re.escape(plugin_id) + r" \(.*\) enabled \[local:(.+)\]$")
    for line in completed.stdout.splitlines():
        match = pattern.match(line.strip())
        if match:
            return match.group(1)
    return None
