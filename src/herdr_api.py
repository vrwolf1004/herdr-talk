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
import tempfile
import time

IS_WINDOWS = sys.platform == "win32"
SOCKET_TIMEOUT_SECONDS = 3
CLI_TIMEOUT_SECONDS = 10  # herdr CLI 한 번의 최대 대기 시간. HERDR_CLI_TIMEOUT_SECONDS 로 바꿀 수 있다.


class HerdrError(Exception):
    """herdr 호출 실패."""


def cli_timeout():
    """CLI 제한 시간(초). 환경 변수가 숫자가 아니거나 0 이하면 기본값을 쓴다."""
    try:
        value = float(os.environ.get("HERDR_CLI_TIMEOUT_SECONDS", ""))
    except ValueError:
        return CLI_TIMEOUT_SECONDS
    return value if value > 0 else CLI_TIMEOUT_SECONDS


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
    """herdr CLI 를 실행하고 JSON 응답의 result 를 돌려준다. check=False 면 실패 시 None.

    출력은 파이프 대신 임시 파일로 받는다. 제한 시간이 지나 부모 프로세스를 죽여도, 그 자식이 파이프를
    잡고 있으면 파이프 쪽이 끝나기를 기다리며 멈추기 때문이다 (Windows 의 .cmd 래퍼 등).
    """
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        try:
            completed = subprocess.run(
                [herdr_bin(), *[str(arg) for arg in args]],
                stdin=subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                timeout=cli_timeout(),
            )
        except subprocess.TimeoutExpired:
            # 응답하지 않는 herdr 에 매달리지 않는다. 실패와 같은 방식으로 처리한다 (check=False 면 None).
            if not check:
                return None
            raise HerdrError(f"herdr {' '.join(map(str, args))}: no response within {cli_timeout():g} s")
        out.seek(0)  # 자식이 같은 파일 위치를 쓰므로 처음부터 읽는다
        err.seek(0)
        stdout = out.read().decode("utf-8", errors="replace")
        stderr = err.read().decode("utf-8", errors="replace")
    if completed.returncode != 0:
        if not check:
            return None
        detail = (stdout + stderr).strip()
        raise HerdrError(f"herdr {' '.join(map(str, args))}: {detail}")
    output = stdout.strip().lstrip("﻿")
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
