import json
import os
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

from core import relay_chat as rc  # noqa: E402
from core import relay_rest  # noqa: E402
from core.events import Notice, Status  # noqa: E402

ROOM = "ABCD1234"
SEED = bytes(range(32))


class Hooks:
    def __init__(self, name="Alice"):
        self.said = []
        self.status = ""
        self.name = name
        self.shown_members = None

    def my_name(self):
        return self.name

    def members(self, names):
        self.shown_members = list(names)

    def emit(self, event):
        if isinstance(event, Status):
            self.status = event.text
        else:
            self.said.append(event)


def settings(**changes):
    values = {"mode": "supabase", "url": "https://x.supabase.co", "key": "dummy-test-key", "name": "Alice", "room_code": ROOM, "trust": "12345678"}
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

    def test_the_join_reads_the_members_and_an_expired_room_raises_its_reason(self):
        relay = rc.Relay(Hooks(), None, self.config, settings(), self.session)
        info = {"room_code": ROOM, "owner_pk": "01" * 32, "members": ["01" * 32, "02" * 32]}
        with mock.patch.object(relay_rest, "join_room", return_value=info),                 mock.patch.object(relay_rest, "_send", return_value=(200, '[]')):
            self.assertEqual(relay._load_members(), info)
        with mock.patch.object(relay_rest, "_send", return_value=(410, '{"error":"x"}')):
            with self.assertRaisesRegex(ValueError, "방 기한 만료"):
                relay._load_members()

    def test_the_names_of_the_people_in_the_room_are_shown_and_a_bad_name_falls_back_to_the_key(self):
        hooks = Hooks()
        relay = rc.Relay(hooks, None, self.config, settings(), self.session)
        first, second = "02" * 32, "03" * 32
        info = {"room_code": ROOM, "owner_pk": "01" * 32, "members": ["01" * 32, first, second]}
        answer = json.dumps([{"pk": first, "noise_pk": "ab" * 32, "name": "서연"}, {"pk": second, "noise_pk": "cd" * 32, "name": "a​b"}])
        with mock.patch.object(relay_rest, "join_room", return_value=info), mock.patch.object(relay_rest, "_send", return_value=(200, answer)) as send:
            relay._load_members()
        self.assertEqual(hooks.shown_members, ["서연", second[:4]])
        self.assertIn("select=pk,noise_pk,name", send.call_args.args[0]["url"], "the names are asked for with the keys")

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
        hooks = Hooks(name="a​b")
        relay = rc.Relay(hooks, None, self.config, settings(), self.session)
        with mock.patch.object(relay, "_publish") as publish, mock.patch.object(relay_rest, "join_room") as join:
            relay._run()
        publish.assert_not_called()
        join.assert_not_called()
        self.assertEqual(hooks.status, "relay: name is not valid; not joined yet")

    def test_a_text_with_no_other_member_is_not_sent_and_the_user_is_told(self):
        hooks = Hooks()
        relay = rc.Relay(hooks, None, self.config, settings(), self.session)
        relay.own_pk = "aa" * 32
        relay.send("hello")
        with mock.patch.object(relay_rest, "_send") as send:
            relay._wait(0.3)
        send.assert_not_called()
        self.assertEqual(hooks.said, [Notice("room.send_failed", {"error": "no other member in the room"})])

    def test_a_text_goes_to_every_other_member(self):
        from core import noise as noise_mod
        hooks = Hooks()
        hooks.my_name = lambda: "Alice"
        relay = rc.Relay(hooks, None, self.config, settings(), self.session)
        relay.own_pk = "aa" * 32
        relay.members = {"bb" * 32: b"" * 32, "cc" * 32: b"" * 32, relay.own_pk: b"" * 32}
        with mock.patch("core.room.post_text", return_value=True) as post:
            relay._send_text("hello")
        recipients = post.call_args.args[4]
        self.assertEqual(sorted(recipients), [("bb" * 32, b"" * 32), ("cc" * 32, b"\x02" * 32)])
        self.assertEqual(hooks.said[0].text, "hello")
        self.assertTrue(hooks.said[0].mine)
        self.assertEqual(hooks.said[0].sender, "Alice")
        self.assertEqual(len(hooks.said), 1)


class JoinRetryTests(unittest.TestCase):
    def test_a_failed_join_is_tried_once_more_after_the_wait_and_then_not_again(self):
        import threading
        hooks = Hooks()
        config = relay_rest.Config("https://x.supabase.co", "dummy-test-key")
        relay = rc.Relay(hooks, None, config, settings(), rc.Session(SEED, b"" * 32))
        with mock.patch.object(rc, "MEMBERS_SECONDS", 0.05), mock.patch.object(relay, "_publish", return_value=True),                 mock.patch.object(relay, "_load_members", side_effect=OSError("offline")) as load:
            thread = threading.Thread(target=relay._run, daemon=True)
            thread.start()
            time.sleep(0.6)
            relay.stop_event.set()
            thread.join(2)
        self.assertEqual(load.call_count, 2, "one try, one retry, then no more")


class InboxTests(unittest.TestCase):
    def test_only_rows_that_check_out_are_shown_and_the_cursor_moves_on(self):
        hooks = Hooks()
        relay = rc.Relay(hooks, None, None, settings(), rc.Session(SEED, b"" * 32))
        relay.members = {"bb" * 32: b"" * 32}
        rows = [{"sender_pk": "bb" * 32, "created_at": "t1", "ciphertext": "00", "ts": 1, "nonce": "11" * 16},
                {"sender_pk": "bb" * 32, "created_at": "t2", "ciphertext": "00", "ts": 2, "nonce": "22" * 16}]
        with mock.patch("core.room.fetch_rows", return_value=rows),                 mock.patch("core.room.check_row", side_effect=[None, ("Liam", "hi")]):
            relay._fetch()
        self.assertEqual(relay.since, "t2")
        self.assertEqual(hooks.said, [talk_message("Liam", "hi")])


def talk_message(sender, text):
    from core.events import Message
    return Message(sender, text)


if __name__ == "__main__":
    unittest.main()
