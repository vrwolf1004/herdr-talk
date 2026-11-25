import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))
os.environ.setdefault("HERDR_TALK_LANG", "en")

from core import sessions  # noqa: E402
import talk  # noqa: E402
from extend.screen import add_event  # noqa: E402


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
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-control-")
        self.chat = talk.Chat({"id": "me", "name": "Me", "port": 1234})
        self.chat.lan_listening = True  # as after start_relay
        self.chat.on_event = lambda event: add_event(self.chat, event)
        self.control = talk.start_control(self.chat, self.dir, allow_shutdown=True)
        with open(self.control.file, encoding="utf-8") as f:
            self.info = json.load(f)

    def tearDown(self):
        self.control.close()
        talk.STOP.clear()

    def test_file_has_port_token_pid_and_goes_away(self):
        self.assertEqual(os.path.basename(self.control.file), f"control-{os.getpid()}.json")
        self.assertEqual(set(self.info), {"port", "token", "pid", "started"})
        self.assertEqual(self.info["started"], sessions.process_start_time(os.getpid()))
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
        self.assertEqual(answer, {"ok": True, "peers": [{"name": "Bob", "ip": "10.0.0.2", "port": 5000}], "room": [], "me": {"name": "Me", "port": 1234}, "mode": "local", "relay": ""})
        self.chat.room_names = ["서연"]  # the people in the room (CLOUD SERVICE), for the GUI's list
        client = Client(self.info)
        self.assertEqual(client.call({"cmd": "peers"})["room"], ["서연"])
        client.close()

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

    def test_cloud_service_ignores_lan_announcements(self):
        packet = json.dumps({"app": talk.APP, "id": "shared", "name": "RustA", "port": 5001}).encode("utf-8")

        class Sock:
            sent = False

            def recvfrom(self, _size):
                if not Sock.sent:
                    Sock.sent = True
                    return packet, ("10.0.0.9", 1)
                talk.STOP.set()
                raise OSError("closed")

        self.chat.supabase = True
        talk._listen_loop(Sock(), self.chat)
        self.assertEqual(self.chat.peers, {}, "no LAN peer is remembered, so none is asked to be verified or kicked")
        self.assertEqual(self.chat.waiting_since, {})

    def test_lan_announcements_are_ignored_while_the_first_run_screen_is_open(self):
        packet = json.dumps({"app": talk.APP, "id": "shared", "name": "RustA", "port": 5001}).encode("utf-8")

        class Sock:
            sent = False

            def recvfrom(self, _size):
                if not Sock.sent:
                    Sock.sent = True
                    return packet, ("10.0.0.9", 1)
                talk.STOP.set()
                raise OSError("closed")

        self.chat.lan_listening = False
        talk._listen_loop(Sock(), self.chat)
        self.assertEqual(self.chat.peers, {})

    def test_a_lan_peer_heard_before_the_cloud_mode_is_dropped_and_never_kicked(self):
        self.chat.peers["k"] = {"name": "Early", "ip": "10.0.0.9", "port": 1}
        self.chat.waiting_since["k"] = 0  # first announced long ago, never joined
        self.chat.supabase = True
        self.assertFalse(self.chat.is_kicked("k", "Early", "10.0.0.9"), "no kick in CLOUD SERVICE")
        self.chat.supabase = False
        self.assertTrue(self.chat.is_kicked("k", "Early", "10.0.0.9"), "the LAN mode still kicks")

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


class StaleControlFilesTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()

    def tearDown(self):
        for name in os.listdir(self.dir):
            os.remove(os.path.join(self.dir, name))
        os.rmdir(self.dir)

    def touch(self, pid):
        with open(os.path.join(self.dir, f"control-{pid}.json"), "w", encoding="utf-8") as f:
            json.dump({"port": 1, "token": "x", "pid": pid}, f)

    def test_ended_node_removed_live_and_own_kept(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        try:
            self.touch(4000000)  # no such process
            self.touch(child.pid)
            self.touch(os.getpid())
            talk.remove_stale_control_files(self.dir)
            self.assertEqual(sorted(os.listdir(self.dir)), sorted([f"control-{child.pid}.json", f"control-{os.getpid()}.json"]))
        finally:
            child.kill()
            child.wait()

    def test_unrelated_names_and_missing_folder_do_not_raise(self):
        self.touch(4000000)
        open(os.path.join(self.dir, "control-x.json"), "w").close()
        open(os.path.join(self.dir, "notes.txt"), "w").close()
        talk.remove_stale_control_files(self.dir)
        self.assertEqual(sorted(os.listdir(self.dir)), ["control-x.json", "notes.txt"])
        talk.remove_stale_control_files(os.path.join(self.dir, "missing"))



if __name__ == "__main__":
    unittest.main()
