import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

import noise  # noqa: E402
import relay  # noqa: E402
import relay_chat as rc  # noqa: E402
import relay_replay  # noqa: E402
import relay_rest  # noqa: E402
import websocket  # noqa: E402

NOW = 1_700_000_000
UNUSED_STATE = __import__('tempfile').mkdtemp(prefix='herdr-talk-chat-test-')


def _keypair(seed_byte):
    secret = bytes([seed_byte]) * 32
    return secret, noise.public_key(secret)


class Hooks:
    def __init__(self, trusted=None):
        self.trusted = trusted or {}
        self.lines = []
        self.said = []
        self.status = ""

    def say(self, text):
        self.said.append(text)

    def show(self, sender, text):
        self.lines.append((sender, text))

    def trusted_name(self, key):
        return self.trusted.get(bytes(key))

    def set_status(self, text):
        self.status = text

    def set_relay_candidate(self, name, key):
        self.candidate = (name, key)


class FakeLink:
    def __init__(self):
        self.sent = []

    def send_envelope(self, line):
        self.sent.append(line)


class PayloadTests(unittest.TestCase):
    def test_payload_format_is_fixed(self):
        text = rc.encode_payload('hi "there"', 1700000000, "ab" * 16)
        self.assertEqual(text, '{"v":1,"ts":1700000000,"nonce":"' + "ab" * 16 + '","text":"hi \\"there\\""}')

    def test_parse_payload_accepts_v1_and_refuses_the_rest(self):
        nonce = "ab" * 16
        self.assertEqual(rc.parse_payload(rc.encode_payload("hello", 5, nonce)), (5, nonce, "hello"))
        for bad in (
            "not json",
            "[]",
            '{"v":2,"ts":1,"nonce":"%s","text":"x"}' % nonce,
            '{"v":1,"ts":"1","nonce":"%s","text":"x"}' % nonce,
            '{"v":1,"ts":1,"nonce":"short","text":"x"}',
            '{"v":1,"ts":1,"nonce":"%s","text":5}' % nonce,
            '{"v":true,"ts":1,"nonce":"%s","text":"x"}' % nonce,
        ):
            self.assertIsNone(rc.parse_payload(bad), bad)

    def test_ik_message_round_trip_and_wrong_sender(self):
        a_secret, a_public = _keypair(1)
        b_secret, b_public = _keypair(2)
        _, c_public = _keypair(3)
        message = rc.seal_payload(a_secret, b_public, "안녕")
        self.assertEqual(rc.open_payload(b_secret, a_public, message), "안녕")
        self.assertIsNone(rc.open_payload(b_secret, c_public, message))
        self.assertIsNone(rc.open_payload(b_secret, a_public, message[:-1]))


class ReceiveTests(unittest.TestCase):
    def setUp(self):
        self.a_secret, self.a_public = _keypair(1)
        self.b_secret, self.b_public = _keypair(2)
        self.session_b = rc.Session(b"\x00" * 32, self.b_secret, self.b_public)
        self.hooks = Hooks({self.a_public: "Alice"})
        self.cache = relay_replay.NonceCache()

    def _line(self, text, ts=NOW, nonce="cd" * 16):
        ct = rc.seal_payload(self.a_secret, self.b_public, rc.encode_payload(text, ts, nonce))
        return relay.seal(self.a_public, self.b_public, ct)

    def _receive(self, line, now=NOW):
        return rc.receive(self.hooks, line, self.session_b, self.a_public, self.cache, now)

    def test_a_verified_message_is_shown_under_the_trusted_name(self):
        self.assertTrue(self._receive(self._line("hello")))
        self.assertEqual(self.hooks.lines, [("Alice", "hello")])

    def test_the_same_envelope_twice_is_shown_once(self):
        line = self._line("hello")
        self.assertTrue(self._receive(line))
        self.assertFalse(self._receive(line))
        self.assertEqual(len(self.hooks.lines), 1)

    def test_time_window_edges(self):
        self.assertTrue(self._receive(self._line("a", ts=NOW - 299, nonce="01" * 16)))
        self.assertFalse(self._receive(self._line("b", ts=NOW - 301, nonce="02" * 16)))
        self.assertFalse(self._receive(self._line("c", ts=NOW + 301, nonce="03" * 16)))

    def test_a_message_from_someone_else_is_dropped(self):
        c_secret, c_public = _keypair(3)
        ct = rc.seal_payload(c_secret, self.b_public, rc.encode_payload("x", NOW, "ee" * 16))
        self.assertFalse(self._receive(relay.seal(c_public, self.b_public, ct)))
        self.assertEqual(self.hooks.lines, [])

    def test_control_characters_are_cleaned(self):
        self.assertTrue(self._receive(self._line("a\x1b[31m\nb", nonce="04" * 16)))
        self.assertNotIn("\x1b", self.hooks.lines[0][1])
        self.assertNotIn("\n", self.hooks.lines[0][1])

    def test_an_empty_message_is_dropped(self):
        self.assertFalse(self._receive(self._line("  \n ", nonce="05" * 16)))


class SendTests(unittest.TestCase):
    def setUp(self):
        self.a_secret, self.a_public = _keypair(1)
        self.b_secret, self.b_public = _keypair(2)
        self.session_a = rc.Session(b"\x00" * 32, self.a_secret, self.a_public)
        self.config = relay_rest.Config("https://x.supabase.co", "dummy-test-key")
        self.row = {"pk": "ab12cd34", "name": "Bob", "noise_pk": self.b_public.hex()}

    def _send(self, hooks, link, row):
        with mock.patch.object(relay_rest, "get_peer", return_value=row):
            return rc.send_text(hooks, self.config, "ab12cd34", self.session_a, self.b_public, link, "hello", now=NOW)

    def test_a_verified_peer_gets_one_envelope_that_the_peer_can_read(self):
        hooks = Hooks({self.b_public: "Bob"})
        link = FakeLink()
        self.assertIsNone(self._send(hooks, link, self.row))
        self.assertEqual(len(link.sent), 1)
        envelope = relay.open_envelope(link.sent[0], (self.a_public, self.b_public))
        self.assertEqual(envelope["from"], self.a_public)
        text = rc.open_payload(self.b_secret, self.a_public, envelope["ct"])
        self.assertEqual(rc.parse_payload(text)[0], NOW)
        self.assertEqual(rc.parse_payload(text)[2], "hello")

    def test_an_unverified_peer_gets_nothing(self):
        link = FakeLink()
        self.assertIn("not verified", self._send(Hooks(), link, self.row))
        self.assertEqual(link.sent, [])

    def test_a_row_with_a_changed_key_gets_nothing(self):
        hooks = Hooks({self.b_public: "Bob"})
        link = FakeLink()
        row = {**self.row, "noise_pk": "cd" * 32}
        self.assertIn("not verified", self._send(hooks, link, row))
        self.assertEqual(link.sent, [])

    def test_a_missing_row_gets_nothing(self):
        link = FakeLink()
        self.assertIn("not verified", self._send(Hooks({self.b_public: "Bob"}), link, None))
        self.assertEqual(link.sent, [])

    def test_a_failed_websocket_send_is_reported(self):
        class Broken:
            def send_envelope(self, line):
                raise websocket.WebSocketError("connection closed")

        notice = self._send(Hooks({self.b_public: "Bob"}), Broken(), self.row)
        self.assertIn("connection closed", notice)


class StartTests(unittest.TestCase):
    def _settings(self, **changes):
        base = {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key", "name": "Alice", "peer": "ab12cd34"}
        base.update(changes)
        return base

    def test_local_mode_starts_nothing(self):
        relay_thread, status = rc.start(Hooks(), UNUSED_STATE, self._settings(mode="local"), b"\x01" * 32, b"\x02" * 32)
        self.assertEqual((relay_thread, status), (None, ""))

    def test_empty_settings_start_nothing(self):
        empty = self._settings(url="", key="", peer="")
        self.assertEqual(rc.start(Hooks(), UNUSED_STATE, empty, b"\x01" * 32, b"\x02" * 32), (None, ""))

    def test_bad_settings_say_why_and_never_raise(self):
        for changes, word in (
            ({"url": "http://example.com"}, "https"),
            ({"key": ""}, "incomplete"),
            ({"peer": "zz"}, "peer id"),
        ):
            relay_thread, status = rc.start(Hooks(), UNUSED_STATE, self._settings(**changes), b"\x01" * 32, b"\x02" * 32)
            self.assertIsNone(relay_thread)
            self.assertIn(word, status)


class RelayThreadTests(unittest.TestCase):
    def test_not_connected_texts_are_reported_and_the_thread_stops(self):
        b_secret, b_public = _keypair(2)
        session = rc.Session(b"\x00" * 32, b_secret, b_public)
        hooks = Hooks()
        settings = {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key", "name": "Bob", "peer": "ab12cd34"}
        config = relay_rest.Config(settings["url"], settings["key"])
        relay_thread = rc.Relay(hooks, UNUSED_STATE, config, settings, session)
        with mock.patch.object(relay_rest, "put_peer", return_value=True), mock.patch.object(relay_rest, "get_peer", return_value=None):
            relay_thread.start()
            relay_thread.send("hello")
            import time
            for _ in range(50):
                if hooks.said:
                    break
                time.sleep(0.05)
            relay_thread.stop()
            relay_thread.thread.join(5)
        self.assertEqual(hooks.status, "relay: peer not found on the relay")
        self.assertTrue(any("not connected" in line for line in hooks.said))
        self.assertFalse(relay_thread.thread.is_alive())


class LeaveTest(unittest.TestCase):
    """leave(): one signed delete for a folder with keys and the relay on, nothing otherwise."""

    def setUp(self):
        self.state = __import__('tempfile').mkdtemp(prefix='herdr-talk-leave-test-')
        self.addCleanup(__import__('shutil').rmtree, self.state, True)

    def _leave(self, mode, keys):
        if keys:
            rc.relay_key.load_or_create(self.state)
            rc.relay_key.load_or_create_short_id(self.state)
        settings = {**rc.relay_settings.empty(), "mode": mode, "url": "https://relay.example", "key": "dummy-test-key"}
        with mock.patch.object(rc.relay_settings, "load_saved", return_value=settings), \
                mock.patch.object(relay_rest, "delete_peer", return_value=True) as delete:
            return rc.leave(self.state, timeout=rc.DEAD_LEAVE_SECONDS), delete

    def test_a_folder_with_keys_sends_one_delete_with_its_own_keys(self):
        left, delete = self._leave(rc.relay_settings.SUPABASE, keys=True)
        self.assertTrue(left)
        seed, short = rc.relay_key.load_or_create(self.state), rc.relay_key.load_or_create_short_id(self.state)
        delete.assert_called_once_with(mock.ANY, seed, short, rc.DEAD_LEAVE_SECONDS)

    def test_nothing_is_sent_when_the_relay_is_off(self):
        left, delete = self._leave(rc.relay_settings.LOCAL, keys=True)
        self.assertFalse(left)
        delete.assert_not_called()

    def test_nothing_is_sent_and_no_key_is_made_when_the_keys_are_missing(self):
        left, delete = self._leave(rc.relay_settings.SUPABASE, keys=False)
        self.assertFalse(left)
        delete.assert_not_called()
        self.assertEqual(os.listdir(self.state), [])


if __name__ == "__main__":
    unittest.main()
