import json
import os
import shutil
import socket
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))
os.environ.setdefault("HERDR_TALK_LANG", "en")

import noise  # noqa: E402
import phone  # noqa: E402
import store  # noqa: E402
import talk  # noqa: E402


class Base(unittest.TestCase):
    def setUp(self):
        talk.STOP.clear()
        self.lines = []
        self.saved = talk._sink
        talk._sink = lambda line, mine=False: self.lines.append(line)
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-phone-")
        self.hosts = []

    def tearDown(self):
        for host in self.hosts:
            host.release()
        talk._sink = self.saved
        talk.STOP.clear()
        shutil.rmtree(self.dir, ignore_errors=True)

    def take(self, port):
        host = phone.Host.take(self.dir, port)
        if host:
            self.hosts.append(host)
        return host

    def chat(self, port):
        chat = talk.Chat({"id": f"id{port}", "name": f"N{port}", "port": port})
        chat.state_dir = self.dir
        return chat


class TokenTest(unittest.TestCase):
    def test_six_digits_with_leading_zeros_kept(self):
        for _ in range(200):
            token = phone.new_token()
            self.assertRegex(token, r"^\d{6}$")
        with mock.patch.object(phone.secrets, "randbelow", return_value=42):
            self.assertEqual(phone.new_token(), "000042")


class RankTest(unittest.TestCase):
    def test_addresses_are_ranked_for_a_phone(self):
        ranked = phone.rank(
            ["192.168.56.1", "169.254.1.2", "10.0.0.5", "192.168.0.181", "127.0.0.1", "192.168.0.181", "203.0.113.9"],
            "192.168.0.181", 47860)
        self.assertEqual(ranked, ["http://192.168.0.181:47860", "http://10.0.0.5:47860", "http://203.0.113.9:47860",
                                  "http://192.168.56.1:47860", "http://169.254.1.2:47860"])


class HostTest(Base):
    def test_one_host_wins_and_a_second_node_changes_nothing(self):
        first = self.take(1111)
        self.assertIsNotNone(first)
        info = phone.read(self.dir)
        self.assertEqual((info["owner_pid"], info["owner_port"], info["port"]), (os.getpid(), 1111, phone.bridge_port()))
        self.assertIsNone(self.take(2222))
        self.assertEqual(phone.read(self.dir), info)

    def test_phone_json_has_the_agreed_fields(self):
        self.take(7)
        with open(phone.phone_file(self.dir), encoding="utf-8") as f:
            value = json.load(f)
        self.assertEqual(set(value), {"port", "token", "urls", "owner_pid", "owner_port"})
        self.assertRegex(value["token"], r"^\d{6}$")
        self.assertIsInstance(value["urls"], list)

    def test_the_last_node_to_leave_removes_the_file_and_the_lock_is_free_again(self):
        first = self.take(1)
        first.release()
        self.assertIsNone(phone.read(self.dir))
        self.assertIsNotNone(self.take(2))

    def test_a_successor_keeps_the_token_while_another_node_is_alive(self):
        alive = socket.socket()
        alive.bind(("127.0.0.1", 0))
        alive.listen()
        with open(os.path.join(self.dir, "control-1.json"), "w", encoding="utf-8") as f:
            json.dump({"port": alive.getsockname()[1], "token": "x"}, f)
        try:
            first = self.take(1)
            token = phone.read(self.dir)["token"]
            first.release()
            self.assertIsNotNone(phone.read(self.dir))
            self.assertIsNotNone(self.take(2))
            info = phone.read(self.dir)
            self.assertEqual((info["token"], info["owner_port"]), (token, 2))
        finally:
            alive.close()

    def test_regenerate_keeps_the_other_fields(self):
        self.take(1)
        before = phone.read(self.dir)
        self.assertTrue(phone.regenerate_token(self.dir))
        after = phone.read(self.dir)
        self.assertEqual({k: v for k, v in after.items() if k != "token"}, {k: v for k, v in before.items() if k != "token"})
        os.remove(phone.phone_file(self.dir))
        self.assertFalse(phone.regenerate_token(self.dir))


class ChatPhoneTest(Base):
    def test_first_chat_is_host_and_shows_the_phone_once_without_leaking_the_token(self):
        first, second = self.chat(1), self.chat(2)
        self.assertTrue(first.try_become_host())
        self.assertFalse(second.try_become_host())
        token = phone.read(self.dir)["token"]
        self.assertIn(token, "\n".join(first.phone_overlay()))
        self.assertIsNone(second.phone_overlay(), "a node that is not the host shows nothing by itself")
        second.handle("/phone")
        self.assertIn(token, "\n".join(second.phone_overlay()))  # but it reads the host's phone.json when asked
        self.assertFalse([line for line in self.lines if token in line])
        self.assertFalse([line for chat in (first, second) for line, _ in chat.lines if token in line])
        first.release_host()

    def test_phone_new_changes_the_token_and_says_nothing_secret(self):
        chat = self.chat(1)
        chat.try_become_host()
        before = phone.read(self.dir)["token"]
        chat.handle("/phone new")
        after = phone.read(self.dir)["token"]
        self.assertNotEqual(before, after)
        self.assertIn(talk.t("phone.new_done"), self.lines)
        self.assertFalse([line for line in self.lines if before in line or after in line])
        chat.handle("/phone what")
        self.assertEqual(self.lines[-1], talk.t("usage.phone"))
        chat.release_host()

    def test_ctrl_p_toggles_and_any_other_key_hides(self):
        chat = self.chat(1)
        typed = [""]
        talk.on_key("\x10", chat, typed)
        self.assertTrue(chat.phone_visible())
        self.assertEqual(typed[0], "")
        talk.on_key("\x10", chat, typed)
        self.assertFalse(chat.phone_visible())
        talk.on_key("\x10", chat, typed)
        talk.on_key("a", chat, typed)
        self.assertFalse(chat.phone_visible())
        self.assertEqual(typed[0], "a")

    def test_it_hides_when_its_time_is_up_and_asks_for_a_redraw(self):
        chat = self.chat(1)
        redraws = []
        chat.on_phone_change = lambda: redraws.append(1)
        chat.show_phone()
        chat.phone_until = time.monotonic() - 1
        self.assertFalse(chat.phone_visible())
        self.assertEqual(len(redraws), 2)  # once for show, once for the hide

    def test_no_phone_json_says_so(self):
        chat = self.chat(1)
        chat.show_phone()
        self.assertEqual(chat.phone_overlay(), [talk.t("phone.none")])

    def test_the_host_joining_shows_the_phone_here_but_a_remote_one_does_not(self):
        host = self.chat(5000)
        host.try_become_host()
        info = phone.read(self.dir)
        info["owner_pid"] = os.getpid() + 1  # as if the host were another process
        with open(phone.phone_file(self.dir), "w", encoding="utf-8") as f:
            json.dump(info, f)
        public, secret = noise.generate_keypair()
        mine = talk.Chat({"id": store.peer_id(public), "name": "Me", "port": 1, "key": public, "secret": secret})
        mine.state_dir = self.dir
        announce = {"id": "other-node", "name": "Host", "port": 5000}
        talk.remember_peer(mine, announce, "203.0.113.7")   # another PC: its token is not ours
        self.assertFalse(mine.phone_visible())
        talk.remember_peer(mine, {**announce, "id": "other-node2", "port": 5000}, "127.0.0.1")
        self.assertTrue(mine.phone_visible())
        host.release_host()

    def test_screen_frame_draws_the_overlay_over_the_bottom_rows_only(self):
        frame = talk.screen_frame([("hello", False)], "status", "", 40, 10, ["PHONE-ROW-1", "PHONE-ROW-2"])
        self.assertIn("PHONE-ROW-1", frame)
        self.assertIn("PHONE-ROW-2", frame)
        self.assertNotIn("PHONE-ROW", talk.screen_frame([("hello", False)], "status", "", 40, 10))

    def test_the_first_node_is_the_host_named_so_unless_it_has_a_name(self):
        import types
        args = lambda name: types.SimpleNamespace(name=name, tcp_port=0, udp_port=0)
        started = []
        try:
            for name, folder in ((None, self.dir), (None, self.dir), ("Zed", tempfile.mkdtemp(prefix="herdr-talk-phone-"))):
                with mock.patch.object(talk, "announce"), mock.patch.object(talk, "listen_for_peers"):
                    server, chat = talk.start_node(args(name), state=folder, plaintext=False)
                started.append((server, chat, folder))
            (_, first, _), (_, second, _), (_, named, other) = started
            self.assertTrue(first.is_host() and not second.is_host() and named.is_host())
            self.assertEqual(first.me["name"], talk.t("host.name"))
            self.assertNotEqual(second.me["name"], talk.t("host.name"))
            self.assertEqual(named.me["name"], "Zed")
            kept = second.me["name"]
            first.release_host()
            self.assertTrue(second.try_become_host())
            self.assertEqual(second.me["name"], kept)  # a successor keeps its name; only the badge appears
        finally:
            for server, chat, folder in started:
                chat.release_host()
                server.close()
            shutil.rmtree(started[-1][2], ignore_errors=True) if started else None

    def test_help_mentions_the_command_and_the_key(self):
        box = "\n".join(talk.HELP_BOX)
        self.assertIn("/phone", box)
        self.assertIn("Ctrl+P", box)


if __name__ == "__main__":
    unittest.main()
