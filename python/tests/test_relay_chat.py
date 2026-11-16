import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import relay_chat as rc  # noqa: E402
from core import relay_rest  # noqa: E402
from core.events import Notice, Status  # noqa: E402

ROOM = "ABCD1234"
SEED = bytes(range(32))


class Hooks:
    def __init__(self):
        self.said = []
        self.status = ""

    def emit(self, event):
        if isinstance(event, Status):
            self.status = event.text
        else:
            self.said.append(event)


def settings(**changes):
    values = {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key", "name": "Alice", "room_code": ROOM}
    values.update(changes)
    return values


class StartTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-room-start-")

    def test_local_mode_and_empty_settings_start_nothing(self):
        self.assertEqual(rc.start(Hooks(), self.dir, settings(mode="local")), (None, ""))
        self.assertEqual(rc.start(Hooks(), self.dir, settings(url="", key="", room_code="", name="")), (None, ""))

    def test_a_bad_room_code_or_url_is_reported(self):
        relay, status = rc.start(Hooks(), self.dir, settings(room_code="abcd1234"))
        self.assertIsNone(relay)
        self.assertIn("room code", status)
        relay, status = rc.start(Hooks(), self.dir, settings(url="http://example.com"))
        self.assertIsNone(relay)
        self.assertIn("https", status)

    def test_a_room_only_start_is_accepted_and_stopped(self):
        with mock.patch.object(rc.Relay, "_run", lambda self: self.stop_event.wait(2)):
            relay, status = rc.start(Hooks(), self.dir, settings())
            self.assertEqual(status, "")
            relay.stop()
            relay.thread.join(5)
            self.assertFalse(relay.thread.is_alive())


class RoomThreadTests(unittest.TestCase):
    def setUp(self):
        self.config = relay_rest.Config("https://x.supabase.co", "dummy-test-key")
        self.session = rc.Session(SEED)

    def test_the_room_join_status_names_the_room_and_its_size(self):
        hooks = Hooks()
        relay = rc.Relay(hooks, None, self.config, settings(), self.session)
        info = {"room_code": ROOM, "owner_pk": "01" * 32, "members": ["01" * 32, "02" * 32]}
        with mock.patch.object(relay_rest, "join_room", return_value=info):
            self.assertEqual(relay._join_room(), "relay: 방 참가 (멤버 2명)")
        with mock.patch.object(relay_rest, "join_room", side_effect=OSError("offline")):
            self.assertEqual(relay._join_room(), "relay: room join failed: offline")

    def test_the_join_status_counts_the_members_in_the_answer(self):
        relay = rc.Relay(Hooks(), None, self.config, settings(), self.session)
        empty = {"room_code": ROOM, "owner_pk": "01" * 32, "members": []}
        with mock.patch.object(relay_rest, "join_room", return_value=empty):
            self.assertEqual(relay._join_room(), "relay: 방 참가 (멤버 0명)")
        # No members list in the answer: read_join_room refuses it, and the existing failure text is shown.
        with mock.patch.object(relay_rest, "_send", return_value=(200, '{"room_code":"ABCD1234","owner_pk":"%s"}' % ("01" * 32))):
            self.assertEqual(relay._join_room(), "relay: room join failed: answer has no valid members list")

    def test_an_expired_room_says_so(self):
        relay = rc.Relay(Hooks(), None, self.config, settings(), self.session)
        with mock.patch.object(relay_rest, "_send", return_value=(410, '{"error":"x"}')):
            self.assertEqual(relay._join_room(), "relay: room join failed: 방 기한 만료")

    def test_a_failed_row_write_stops_the_join_and_says_so(self):
        hooks = Hooks()
        relay = rc.Relay(hooks, None, self.config, settings(), self.session)
        # The first write fails: no join is tried, and the status says why. The retry (after the wait) stops the thread.
        with mock.patch.object(relay, "_publish", return_value=False), \
                mock.patch.object(relay, "_wait", side_effect=lambda seconds: relay.stop_event.set()), \
                mock.patch.object(relay_rest, "join_room") as join:
            relay._run()
        join.assert_not_called()
        self.assertEqual(hooks.status, "relay: could not publish this PC's row; not joined yet")

    def test_a_bad_name_is_not_published_or_joined_and_the_reason_is_shown(self):
        hooks = Hooks()
        relay = rc.Relay(hooks, None, self.config, settings(name="a​b"), self.session)
        with mock.patch.object(relay, "_publish") as publish, mock.patch.object(relay_rest, "join_room") as join:
            relay._run()
        publish.assert_not_called()
        join.assert_not_called()
        self.assertEqual(hooks.status, "relay: name is not valid; not joined yet")

    def test_typed_texts_are_not_sent_and_the_user_is_told(self):
        hooks = Hooks()
        relay = rc.Relay(hooks, None, self.config, settings(), self.session)
        relay.send("hello")
        relay._wait(0.3)
        self.assertEqual(hooks.said, [Notice("relay.room_not_sent")])


if __name__ == "__main__":
    unittest.main()
