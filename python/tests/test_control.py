import json
import os
import socket
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))
os.environ.setdefault("HERDR_TALK_LANG", "en")

import talk  # noqa: E402


class Client:
    """A control client, as the bridge is: one token line, then a request line and an answer line each."""

    def __init__(self, info, token=None):
        self.sock = socket.create_connection(("127.0.0.1", info["port"]), timeout=5)
        self.file = self.sock.makefile("rwb")
        self.first = self.call({"token": info["token"] if token is None else token})

    def call(self, request):
        self.file.write((json.dumps(request, ensure_ascii=False) + "\n").encode("utf-8"))
        self.file.flush()
        line = self.file.readline()
        return json.loads(line.decode("utf-8")) if line else None

    def close(self):
        self.file.close()
        self.sock.close()


class ControlTest(unittest.TestCase):
    def setUp(self):
        talk.STOP.clear()
        self.saved_sink = talk._sink
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-control-")
        self.chat = talk.Chat({"id": "me", "name": "Me", "port": 1234})
        talk._sink = self.chat.add_line
        self.control = talk.start_control(self.chat, self.dir, allow_shutdown=True)
        with open(self.control.file, encoding="utf-8") as f:
            self.info = json.load(f)

    def tearDown(self):
        self.control.close()
        talk.STOP.clear()
        talk._sink = self.saved_sink

    def test_file_has_port_token_pid_and_goes_away(self):
        self.assertEqual(os.path.basename(self.control.file), f"control-{os.getpid()}.json")
        self.assertEqual(set(self.info), {"port", "token", "pid"})
        self.assertEqual(len(self.info["token"]), 64)
        self.control.close()
        self.assertFalse(os.path.exists(self.control.file))

    def test_bad_token_is_refused_and_closed(self):
        client = Client(self.info, token="wrong")
        self.assertEqual(client.first, {"ok": False, "error": "bad token"})
        try:
            self.assertIsNone(client.call({"cmd": "lines"}))
        except OSError:
            pass  # Windows reports the closed connection as an error
        client.close()

    def test_lines_have_text_and_mine(self):
        self.chat.add_line("Bob: 안녕", False)
        self.chat.add_line("Me (you): hi", True)
        client = Client(self.info)
        self.assertEqual(client.first, {"ok": True})
        answer = client.call({"cmd": "lines"})
        client.close()
        self.assertEqual(answer, {"ok": True, "lines": [{"text": "Bob: 안녕", "mine": False}, {"text": "Me (you): hi", "mine": True}]})

    def test_peers_and_me(self):
        with self.chat.peers_lock:
            self.chat.peers["x"] = {"name": "Bob", "ip": "10.0.0.2", "port": 5000, "seen": time.time()}
            self.chat.peers["old"] = {"name": "Gone", "ip": "10.0.0.3", "port": 5001, "seen": 0}
        client = Client(self.info)
        answer = client.call({"cmd": "peers"})
        client.close()
        self.assertEqual(answer, {"ok": True, "peers": [{"name": "Bob", "ip": "10.0.0.2", "port": 5000}], "me": {"name": "Me", "port": 1234}})

    def test_send_goes_to_the_peer(self):
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen()
        with self.chat.peers_lock:
            self.chat.peers["x"] = {"name": "Bob", "ip": "127.0.0.1", "port": server.getsockname()[1], "seen": time.time()}
        client = Client(self.info)
        self.assertEqual(client.call({"cmd": "send", "text": "안녕 폰"}), {"ok": True})
        client.close()
        conn, _ = server.accept()
        with conn:
            conn.settimeout(5)
            data = b""
            while chunk := conn.recv(4096):
                data += chunk
        server.close()
        message = json.loads(data.decode("utf-8"))
        self.assertEqual((message["from"], message["text"]), ("Me", "안녕 폰"))
        self.assertTrue(self.chat.lines[-1][1])

    def test_send_without_text_and_unknown_cmd(self):
        client = Client(self.info)
        self.assertEqual(client.call({"cmd": "send"}), {"ok": False, "error": "text missing"})
        self.assertEqual(client.call({"cmd": "nope"}), {"ok": False, "error": "unknown cmd"})
        client.close()

    def test_too_long_request_closes_the_connection(self):
        client = Client(self.info)
        client.file.write(b"x" * (talk.CONTROL_MAX_REQUEST_BYTES + 10) + b"\n")
        client.file.flush()
        self.assertEqual(client.file.readline(), b"")
        client.close()

    def test_shutdown_sets_stop_only_when_allowed(self):
        client = Client(self.info)
        self.assertEqual(client.call({"cmd": "shutdown"}), {"ok": True})
        client.close()
        self.assertTrue(talk.STOP.is_set())
        self.assertEqual(talk.control_answer(self.chat, {"cmd": "shutdown"}, False), {"ok": False, "error": "unknown cmd"})

    def test_same_id_on_two_ports_is_two_peers(self):
        packets = [json.dumps({"app": talk.APP, "id": "shared", "name": name, "port": port}).encode("utf-8")
                   for name, port in (("RustA", 5001), ("RustB", 5002), ("RustA", 5001))]

        class Sock:
            def recvfrom(self, _size):
                if not packets:
                    talk.STOP.set()
                    raise OSError("closed")
                return packets.pop(0), ("10.0.0.9", 1)

        talk._listen_loop(Sock(), self.chat)
        self.assertEqual(sorted(info["name"] for info in self.chat.peers.values()), ["RustA", "RustB"])

    def test_state_dir_order(self):
        saved = {key: os.environ.get(key) for key in ("HERDR_PLUGIN_STATE_DIR", "LOCALAPPDATA")}
        try:
            os.environ["HERDR_PLUGIN_STATE_DIR"] = "/a"
            self.assertEqual(talk.state_dir(), "/a")
            del os.environ["HERDR_PLUGIN_STATE_DIR"]
            os.environ["LOCALAPPDATA"] = "/b"
            self.assertEqual(talk.state_dir(), os.path.join("/b", "herdr-talk"))
        finally:
            for key, value in saved.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value


if __name__ == "__main__":
    unittest.main()
