import io
import json
import re
import os
import shutil
import socket
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / "src"
sys.path.insert(0, str(SRC))
os.environ.setdefault("HERDR_TALK_LANG", "en")

import talk  # noqa: E402


class CollectSayTest(unittest.TestCase):
    """Events go to Chat.on_event; collect the lines instead of printing."""

    def setUp(self):
        self.lines = []
        self.saved = talk.Chat.on_event
        talk.Chat.on_event = staticmethod(lambda event: self.lines.extend(talk.event_texts(event)))

    def tearDown(self):
        talk.Chat.on_event = staticmethod(self.saved)


class WrapAndWidthTest(unittest.TestCase):
    def test_breaks_at_spaces(self):
        self.assertEqual(talk.wrap_line("one two three", 7), ["one two", "three"])

    def test_cuts_long_word_by_character(self):
        rows = talk.wrap_line("abcdefghij", 4)
        self.assertTrue(all(talk.char_width(row) <= 4 for row in rows))
        self.assertEqual("".join(rows), "abcdefghij")

    def test_wide_characters_count_as_two(self):
        self.assertEqual(talk.char_width("가나"), 4)
        self.assertEqual(talk.char_width("abc"), 3)
        for row in talk.wrap_line("가나다라마바사", 4):
            self.assertLessEqual(talk.char_width(row), 4)


class NameTest(unittest.TestCase):
    def test_pick_name_avoids_taken(self):
        names = ["a", "b", "c"]
        for _ in range(20):
            self.assertIn(talk.pick_name(names, taken={"a", "b"}), ["c"])

    def test_pick_name_falls_back_when_all_taken(self):
        names = ["a", "b"]
        self.assertIn(talk.pick_name(names, taken={"a", "b"}), names)


class ChatHandleTest(CollectSayTest):
    def make_chat(self):
        return talk.Chat({"id": "me", "name": "Me", "port": 1})

    def test_help_opens_the_box_over_the_messages(self):
        chat = self.make_chat()
        self.assertTrue(chat.handle("/help"))
        # The command is echoed first; the box is an overlay (chat.help_open), not chat lines.
        self.assertEqual(self.lines, ["> /help"])
        self.assertTrue(chat.help_open)

    def test_name_change_is_announced(self):
        chat = self.make_chat()
        chat.handle("/name Alice")
        self.assertEqual(chat.me["name"], "Alice")

    def test_quit_returns_false(self):
        self.assertFalse(self.make_chat().handle("/quit"))

    def test_lang_avoids_names_of_fresh_peers(self):
        names = list(talk.NAMES_BY_LANGUAGE["ko"][1])
        keep = names[0]
        chat = self.make_chat()
        chat.peers = {
            str(i): {"name": name, "ip": "10.0.0.1", "port": 1, "seen": time.time()}
            for i, name in enumerate(names[1:])
        }
        chat.handle("/lang ko")
        self.assertEqual(chat.me["name"], keep)


class IdleTimerTest(unittest.TestCase):
    def test_touch_restarts_idle_timer(self):
        chat = talk.Chat({"id": "me", "name": "Me", "port": 1})
        chat.last_activity = 0.0
        chat.touch()
        self.assertGreater(chat.last_activity, time.time() - 5)


class ReceiveTest(CollectSayTest):
    def deliver(self, payload):
        left, right = socket.socketpair()
        try:
            left.sendall(payload)
            left.shutdown(socket.SHUT_WR)
            talk.receive_one(right, talk.Chat({"id": "me", "name": "Me", "port": 1}))
        finally:
            left.close()

    def message(self, **fields):
        base = {"app": talk.APP, "from": "Bob", "text": "hi"}
        base.update(fields)
        return json.dumps(base).encode("utf-8")

    def test_valid_message_is_shown(self):
        self.deliver(self.message())
        self.assertEqual(self.lines, ["Bob (plain text) > hi"])

    def test_wrong_types_are_dropped(self):
        self.deliver(self.message(text=123))
        self.deliver(self.message(**{"from": ["x"]}))
        self.deliver(b"[1, 2]")
        self.assertEqual(self.lines, [])

    def test_control_characters_are_replaced(self):
        self.deliver(self.message(text="a\x1b[31mred\x07"))
        self.assertEqual(len(self.lines), 1)
        self.assertNotIn("\x1b", self.lines[0])
        self.assertNotIn("\x07", self.lines[0])

    def test_blank_text_is_dropped(self):
        self.deliver(self.message(text="   "))
        self.assertEqual(self.lines, [])


class LocaleKeysTest(unittest.TestCase):
    def test_every_locale_has_the_same_keys_as_english(self):
        locales = Path(talk.__file__).resolve().parent / "locales"
        english = set(json.loads((locales / "en.json").read_text(encoding="utf-8")))
        for path in sorted(locales.glob("*.json")):
            keys = set(json.loads(path.read_text(encoding="utf-8")))
            self.assertEqual(keys, english, path.name)

    def test_every_locale_has_the_chat_guide_and_input_hint(self):
        locales = Path(talk.__file__).resolve().parent / "locales"
        for path in sorted(locales.glob("*.json")):
            messages = json.loads(path.read_text(encoding="utf-8"))
            for key in ("status.help_hint", "chat.input_hint"):
                self.assertTrue(messages.get(key), f"{path.name} {key}")
            self.assertIn("F1", messages["status.help_hint"], path.name)


class ShutdownTest(unittest.TestCase):
    def setUp(self):
        talk.STOP.clear()

    def tearDown(self):
        talk.STOP.clear()

    def test_tcp_listener_stops_after_signal(self):
        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        server.bind(("127.0.0.1", 0))
        server.listen()
        worker = threading.Thread(target=talk.receive_messages, args=(server, talk.Chat({"id": "me", "name": "Me", "port": 1})), daemon=True)
        worker.start()
        talk.STOP.set()
        worker.join(3)
        server.close()
        self.assertFalse(worker.is_alive())

    def test_announce_stops_after_signal(self):
        worker = threading.Thread(target=talk.announce, args=(0, {"id": "x", "name": "x", "port": 1}), daemon=True)
        worker.start()
        time.sleep(0.1)
        talk.STOP.set()
        worker.join(3)
        self.assertFalse(worker.is_alive())


class SharedLinesTest(unittest.TestCase):
    def test_concurrent_adds_keep_the_list_consistent(self):
        chat = talk.Chat({"id": "me", "name": "Me", "port": 1})

        def add_many(prefix):
            for i in range(300):
                chat.add_line(f"{prefix}{i}")

        workers = [threading.Thread(target=add_many, args=(f"t{n}-",)) for n in range(4)]
        for worker in workers:
            worker.start()
        for worker in workers:
            worker.join()
        self.assertEqual(len(chat.lines), min(1200, talk.MAX_LINES))


class AnnounceTest(unittest.TestCase):
    def test_the_announcement_always_carries_commits_even_when_empty(self):
        sent = []
        sock = unittest.mock.MagicMock()
        sock.sendto.side_effect = lambda data, addr: (sent.append(json.loads(data.decode("utf-8"))), talk.STOP.set())
        talk.STOP.clear()
        try:
            with unittest.mock.patch.object(talk.socket, "socket", return_value=sock):
                talk.announce(47850, {"id": "me", "name": "Me", "port": 1, "key": bytes(32)})
        finally:
            talk.STOP.clear()
        self.assertEqual(sent[0]["commits"], {})
        self.assertIn("key", sent[0])


class StatusLineTest(unittest.TestCase):
    def test_every_status_line_shows_only_peers(self):
        """The key hints are on the chat guide row (chat.guide), and the node name is not on the status line."""
        locales = Path(talk.__file__).resolve().parent / "locales"
        for path in sorted(locales.glob("*.json")):
            line = json.loads(path.read_text(encoding="utf-8")).get("status.line", "")
            self.assertIn("{peers}", line, path.name)
            self.assertNotIn("{name}", line, path.name)
            for key in ("Ctrl+", "/help"):
                self.assertNotIn(key, line, path.name)


class RoomGateTests(unittest.TestCase):
    """A wrong trust code keeps the chat from opening; a network failure does not (the chat starts and shows it)."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-gate-")
        talk.relay_settings.save(self.dir, {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key",
                                            "name": "Alice", "room_code": "ABCD1234", "trust": "12345678", "setup_code": ""})

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_a_wrong_trust_refuses_and_a_network_failure_does_not(self):
        with mock.patch.object(talk.relay_rest, "join_room", side_effect=ValueError(talk.relay_rest.TRUST_WRONG)):
            self.assertTrue(talk.relay_chat.room_refuses(self.dir))
        with mock.patch.object(talk.relay_rest, "join_room", side_effect=OSError("offline")):
            self.assertFalse(talk.relay_chat.room_refuses(self.dir))
        with mock.patch.object(talk.relay_rest, "join_room", return_value={"members": []}):
            self.assertFalse(talk.relay_chat.room_refuses(self.dir))

    def test_a_403_from_the_relay_refuses_and_a_network_failure_does_not(self):
        with mock.patch.object(talk.relay_rest, "_send", return_value=(403, "")):
            self.assertTrue(talk.relay_chat.room_refuses(self.dir))
        with mock.patch.object(talk.relay_rest, "_send", side_effect=OSError("offline")):
            self.assertFalse(talk.relay_chat.room_refuses(self.dir))

class ChatStatusRowTest(unittest.TestCase):
    """The chat status row: peers, mode, room ID and setup code; a relay error is added at the end, success adds nothing."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-status-")
        talk.relay_settings.save(self.dir, {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key",
                                            "name": "Alice", "room_code": "ABCD1234", "trust": "12345678", "setup_code": "ABCD2345"})

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def _row(self, relay_status):
        chat = mock.MagicMock()
        chat.me = {"name": "Alice"}
        chat.supabase = True
        chat.state_dir = self.dir
        chat.is_host.return_value = False
        chat.fresh_peers.return_value = {"a": {"name": "Bob", "ip": "10.0.0.2", "key": "k1"},
                                         "b": {"name": "Cy", "ip": "10.0.0.3", "key": "k2"}}
        chat.link_of.return_value = "encrypted"
        chat.numbers = {}
        chat.lines = []
        chat.lines_lock = threading.Lock()
        chat.help_open = False
        chat.phone_overlay.return_value = ""
        chat.relay_status = relay_status
        frames = []
        with mock.patch.object(talk, "_enter_screen", return_value=lambda: None), mock.patch.object(talk, "_draw", side_effect=frames.append), \
                mock.patch.object(talk, "leave_when_idle"), mock.patch.object(talk, "read_char", side_effect=KeyboardInterrupt), \
                mock.patch.object(talk.shutil, "get_terminal_size", return_value=os.terminal_size((200, 30))):
            with self.assertRaises(KeyboardInterrupt):
                talk.run_ansi(chat)
        return re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", frames[0])

    def test_status_row_has_peers_mode_room_and_setup_code(self):
        from i18n import t
        text = self._row("")
        expected = f"{t('status.line', peers=2)} · {t('mode.supabase')} · room ID: ABCD1234 · setup code: ABCD2345"
        self.assertIn(expected, text)
        self.assertNotIn("방 참가", text)
        self.assertNotIn("joined", text)

    def test_only_a_relay_error_is_added_to_the_status_row(self):
        from i18n import t
        text = self._row("relay: room join failed: offline")
        self.assertIn(f"setup code: ABCD2345 · relay: room join failed: offline", text)
        self.assertIn(t("status.line", peers=2), text)


class HerdrCliTimeoutTest(unittest.TestCase):
    """A herdr that never answers must stop at the CLI time limit, not hang the pane split."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-fake-")
        if os.name == "nt":
            self.fake = os.path.join(self.dir, "herdr.bat")
            Path(self.fake).write_text("@echo off\r\nping -n 6 127.0.0.1 >nul\r\n", encoding="ascii")
        else:
            self.fake = os.path.join(self.dir, "herdr")
            Path(self.fake).write_text("#!/bin/sh\nsleep 5\n", encoding="ascii")
            os.chmod(self.fake, 0o755)
        self.env = mock.patch.dict(os.environ, {"HERDR_BIN_PATH": self.fake, "HERDR_CLI_TIMEOUT_SECONDS": "0.5",
                                                "HERDR_PANE_ID": "pane-1"})
        self.env.start()

    def tearDown(self):
        self.env.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_a_silent_herdr_raises_after_the_time_limit(self):
        started = time.monotonic()
        with self.assertRaises(talk.herdr_api.HerdrError) as caught:
            talk.herdr_cli("pane", "split", "pane-1")
        self.assertLess(time.monotonic() - started, 4, "the call returned near the 0.5 s limit, not after the fake's sleep")
        self.assertIn("no response", str(caught.exception))

    def test_open_chat_reports_the_timeout_and_returns_1(self):
        started = time.monotonic()
        with mock.patch("sys.stderr", new_callable=io.StringIO) as err:
            code = talk.open_chat(None)
        self.assertEqual(code, 1)
        self.assertIn("no response", err.getvalue())
        self.assertLess(time.monotonic() - started, 4)


if __name__ == "__main__":
    unittest.main()
