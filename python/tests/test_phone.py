import json
import os
import shutil
import socket
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))
os.environ.setdefault("HERDR_TALK_LANG", "en")

from core import noise  # noqa: E402
from core import phone, relay_chat, relay_rest  # noqa: E402
from core.events import Status  # noqa: E402
from extend import screen  # noqa: E402
from core import store  # noqa: E402
import talk  # noqa: E402


class Base(unittest.TestCase):
    def setUp(self):
        talk.STOP.clear()
        self.lines = []
        self.saved = talk.Chat.on_event
        talk.Chat.on_event = staticmethod(lambda event: self.lines.extend(talk.event_texts(event)))
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-phone-")
        self.hosts = []

    def tearDown(self):
        for host in self.hosts:
            host.release()
        talk.Chat.on_event = staticmethod(self.saved)
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


class RoomHostTest(Base):
    """[host] in Supabase mode is the room's owner only; in LAN mode it is still the PC's phone host."""

    def test_in_supabase_only_the_room_owner_is_host_even_when_another_node_holds_the_pc_lock(self):
        chat = self.chat(1)
        chat.supabase = True
        chat.host = object()  # another node on this PC holds the phone host lock
        chat.relay = SimpleNamespace(owner_pk="bb" * 32, own_pk="aa" * 32)
        self.assertFalse(chat.is_host())
        chat.relay.owner_pk = "aa" * 32
        self.assertTrue(chat.is_host(), "the room's creator is the host")
        chat.relay = None
        self.assertFalse(chat.is_host(), "no relay running: no room host")

    def test_lan_mode_keeps_the_pc_host(self):
        chat = self.chat(1)
        chat.host = object()
        self.assertTrue(chat.is_host())

    def test_the_relay_takes_the_owner_from_the_join_answer(self):
        config = relay_rest.Config("https://x.supabase.co", "dummy-test-key")
        settings = {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key", "name": "Alice",
                    "room_code": "ABCD1234", "trust": "12345678"}
        relay = relay_chat.Relay(SimpleNamespace(emit=lambda event: None, my_name=lambda: "Alice"), None, config, settings, relay_chat.Session(bytes(range(32))))
        info = {"room_code": "ABCD1234", "owner_pk": "cc" * 32, "members": []}
        with mock.patch.object(relay_rest, "join_room", return_value=info):
            relay._load_members()
        self.assertEqual(relay.owner_pk, "cc" * 32)


class HostLockTest(Base):
    """One host per PC: the node that holds phone.lock. phone.json keeps only the host's pid and chat port."""

    def test_one_host_wins_and_a_second_node_changes_nothing(self):
        first = self.take(1111)
        self.assertIsNotNone(first)
        info = phone.read(self.dir)
        self.assertEqual((info["owner_pid"], info["owner_port"]), (os.getpid(), 1111))
        self.assertIsNone(self.take(2222))
        self.assertEqual(phone.read(self.dir), info)

    def test_phone_json_has_only_the_host_fields(self):
        self.take(7)
        with open(phone.phone_file(self.dir), encoding="utf-8") as f:
            value = json.load(f)
        self.assertEqual(set(value), {"owner_pid", "owner_port"})

    def test_the_last_node_to_leave_removes_the_file_and_the_lock_is_free_again(self):
        first = self.take(1)
        first.release()
        self.assertIsNone(phone.read(self.dir))
        self.assertIsNotNone(self.take(2))


class HostNameTest(Base):

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
            self.assertIn(first.me["name"], [n for _, names in talk.NAMES_BY_LANGUAGE.values() for n in names], "the host gets a random country name too")
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



class OverlayRowsTest(unittest.TestCase):

    def test_screen_frame_draws_the_overlay_over_the_bottom_rows_only(self):
        # rows=12: the message rows are 4..6, so the two overlay rows are 5 and 6.
        frame = talk.screen_frame([("hello", False)], "status", "", 40, 12, ["ROW-1", "ROW-2"])
        self.assertIn(talk.relay_form.OVERLAY + "ROW-1", frame)
        self.assertIn(talk.relay_form.OVERLAY + "ROW-2", frame)
        self.assertNotIn("ROW", talk.screen_frame([("hello", False)], "status", "", 40, 12))



if __name__ == "__main__":
    unittest.main()
