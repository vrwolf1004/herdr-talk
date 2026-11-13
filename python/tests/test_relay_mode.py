import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / "src"
sys.path.insert(0, str(SRC))
os.environ.setdefault("HERDR_TALK_LANG", "en")

import relay_chat  # noqa: E402
import relay_settings  # noqa: E402
import store  # noqa: E402
import talk  # noqa: E402


class FakeRelay:
    def __init__(self):
        self.sent = []
        self.stopped = False

    def send(self, text):
        self.sent.append(text)

    def stop(self):
        self.stopped = True


class SettingsScreenTest(unittest.TestCase):
    def test_the_settings_screen_polls_for_a_room_answer_between_keys(self):
        state = tempfile.mkdtemp(prefix="herdr-talk-settings-screen-")
        self.addCleanup(shutil.rmtree, state, True)
        with mock.patch.object(talk, "read_char_timeout", side_effect=[None, None, ""]) as read,                 mock.patch.object(talk, "_draw"),                 mock.patch("relay_form.SettingsForm.poll", return_value=False) as poll:
            outcome, _message = talk.settings_screen(state)
        self.assertEqual(outcome, "cancel")
        self.assertEqual(poll.call_count, 2, "polled on each empty wait")
        self.assertEqual(read.call_count, 3)


class ModeTest(unittest.TestCase):
    def setUp(self):
        self.lines = []
        self.saved_sink = talk._sink
        talk._sink = lambda line, mine=False: self.lines.append((line, mine))
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-mode-")
        self.public, self.secret = store.load_or_create_identity(self.dir)
        self.chat = talk.Chat({"id": "me", "name": "Me", "port": 1, "key": self.public, "secret": self.secret})
        self.chat.state_dir = self.dir

    def tearDown(self):
        talk._sink = self.saved_sink
        shutil.rmtree(self.dir, ignore_errors=True)

    def _peer(self):
        self.chat.peers["p"] = {"name": "Bob", "ip": "10.0.0.2", "port": 5000, "seen": 1e18, "id": "p"}

    def _env(self):
        return mock.patch.dict(os.environ, {"HERDR_TALK_RELAY_URL": "", "HERDR_TALK_RELAY_KEY": ""})

    def test_local_mode_is_the_default_and_sends_over_the_lan(self):
        self._peer()
        self.chat.peers["p"]["key"] = None
        self.chat.plaintext = True
        with self._env(), mock.patch.object(talk, "send_to") as send_to:
            self.assertEqual(self.chat.start_relay(), "")
            self.chat.handle("hello")
        self.assertFalse(self.chat.supabase)
        self.assertIsNone(self.chat.relay)
        send_to.assert_called_once()

    def test_supabase_mode_sends_over_the_relay_and_never_over_the_lan(self):
        self._peer()
        self.chat.supabase = True
        self.chat.relay = FakeRelay()
        with mock.patch.object(talk, "send_to") as send_to:
            self.assertTrue(self.chat.handle("hello"))
        send_to.assert_not_called()
        self.assertEqual(self.chat.relay.sent, ["hello"])
        self.assertTrue(any(mine and "hello" in line for line, mine in self.lines))

    def test_supabase_mode_without_a_running_relay_says_so(self):
        self.chat.supabase = True
        self.assertTrue(self.chat.handle("hello"))
        self.assertTrue(any("relay" in line.lower() for line, _ in self.lines))

    def test_commands_still_work_in_supabase_mode(self):
        self.chat.supabase = True
        self.chat.relay = FakeRelay()
        self.chat.handle("/peers")
        self.assertEqual(self.chat.relay.sent, [])

    def test_a_lan_chat_message_is_dropped_in_supabase_mode_and_shown_in_local_mode(self):
        data = json.dumps({"app": talk.APP, "from": "Bob", "text": "hi", "id": "x" * 10}).encode()
        talk.deliver(data, None, self.chat)
        self.assertEqual(len(self.lines), 1)
        self.chat.supabase = True
        talk.deliver(data, None, self.chat)
        self.assertEqual(len(self.lines), 1)

    def test_start_relay_in_supabase_mode_starts_it_and_a_restart_stops_the_old_one(self):
        settings = {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key", "name": "Me", "room_code": "ABCD1234"}
        self.assertIsNone(relay_settings.save(self.dir, settings))
        first, second = FakeRelay(), FakeRelay()
        with self._env(), mock.patch.object(relay_chat, "start", side_effect=[(first, ""), (second, "")]):
            self.chat.start_relay()
            self.assertTrue(self.chat.supabase)
            self.assertIs(self.chat.relay, first)
            self.chat.start_relay()
        self.assertTrue(first.stopped)
        self.assertIs(self.chat.relay, second)

    def test_switching_back_to_local_stops_the_relay(self):
        settings = {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key", "name": "Me", "room_code": "ABCD1234"}
        relay_settings.save(self.dir, settings)
        fake = FakeRelay()
        with self._env(), mock.patch.object(relay_chat, "start", return_value=(fake, "")):
            self.chat.start_relay()
            relay_settings.set_mode(self.dir, relay_settings.LOCAL)
            self.chat.start_relay()
        self.assertTrue(fake.stopped)
        self.assertFalse(self.chat.supabase)
        self.assertIsNone(self.chat.relay)

    def test_a_bad_setting_is_shown_and_does_not_stop_the_chat(self):
        settings = {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key", "name": "Me", "room_code": "ABCD1234"}
        relay_settings.save(self.dir, settings)
        with self._env(), mock.patch.object(relay_chat, "start", return_value=(None, "relay: room code missing")):
            status = self.chat.start_relay()
        self.assertEqual(status, "relay: room code missing")
        self.assertTrue(any("room code missing" in line for line, _ in self.lines))

    def test_control_peers_tells_the_mode(self):
        answer = talk.control_answer(self.chat, {"cmd": "peers"}, False)
        self.assertEqual((answer["mode"], answer["relay"]), ("local", ""))
        self.chat.supabase = True
        self.chat.relay_status = "relay: connected"
        answer = talk.control_answer(self.chat, {"cmd": "peers"}, False)
        self.assertEqual((answer["mode"], answer["relay"]), ("supabase", "relay: connected"))


if __name__ == "__main__":
    unittest.main()
