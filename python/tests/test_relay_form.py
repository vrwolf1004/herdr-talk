import os
import re
import shutil
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

import relay_form  # noqa: E402
import relay_key  # noqa: E402
import relay_rest  # noqa: E402
import relay_settings  # noqa: E402


CTRL_N = ""


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


def _plain(frame):
    return re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", frame)


class FirstRunTests(unittest.TestCase):
    def test_keys_choose(self):
        self.assertEqual(relay_form.FirstRun().on_key("1"), "local")
        self.assertEqual(relay_form.FirstRun().on_key("2"), "supabase")
        self.assertEqual(relay_form.FirstRun().on_key("\x1b"), "quit")
        choice = relay_form.FirstRun()
        self.assertIsNone(choice.on_key("\t"))
        self.assertEqual(choice.on_key("\r"), "supabase")
        self.assertEqual(relay_form.FirstRun().on_key("\r"), "local")

    def test_timer_times_out_after_30_seconds_and_counts_down(self):
        clock = Clock()
        choice = relay_form.FirstRun(clock=clock)
        self.assertFalse(choice.timed_out())
        self.assertEqual(choice.remaining(), 30)
        clock.now += 29.2
        self.assertFalse(choice.timed_out())
        self.assertEqual(choice.remaining(), 1)
        clock.now += 1
        self.assertTrue(choice.timed_out())
        self.assertEqual(choice.remaining(), 0)

    def test_any_key_stops_the_timer_for_good(self):
        clock = Clock()
        choice = relay_form.FirstRun(clock=clock)
        choice.on_key("\t")
        clock.now += 1000
        self.assertFalse(choice.timed_out())
        self.assertIsNone(choice.remaining())

    def test_restart_after_the_settings_screen(self):
        clock = Clock()
        choice = relay_form.FirstRun(clock=clock)
        choice.on_key("2")
        choice.restart_timer()
        clock.now += 29
        self.assertFalse(choice.timed_out())
        clock.now += 2
        self.assertTrue(choice.timed_out())

    def test_empty_key_is_ignored_and_keeps_the_timer(self):
        clock = Clock()
        choice = relay_form.FirstRun(clock=clock)
        self.assertIsNone(choice.on_key(""))
        clock.now += 31
        self.assertTrue(choice.timed_out())

    def test_screen_shows_both_choices_and_the_countdown(self):
        text = _plain(relay_form.FirstRun().render(80, 24))
        self.assertIn("1. Local (LAN)", text)
        self.assertIn("2. Supabase", text)
        self.assertIn("Starting in local mode in 30s", text)


class SettingsFormTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-form-")
        self.old_env = {name: os.environ.pop(name, None) for name in ("HERDR_TALK_RELAY_URL", "HERDR_TALK_RELAY_KEY")}
        # The owner row is written before a create: no network and no identity files in these tests.
        patches = [mock.patch.object(relay_key, "noise_public_hex", return_value="bb" * 32),
                   mock.patch.object(relay_key, "load_or_create_short_id", return_value="ABCD2345"),
                   mock.patch.object(relay_rest, "put_status", return_value=201)]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)
        for name, value in self.old_env.items():
            if value is not None:
                os.environ[name] = value

    def _form(self):
        return relay_form.SettingsForm(self.dir, mine=("aa" * 32, "bb" * 32))

    def _type(self, form, text):
        for ch in text:
            form.on_key(ch)

    def test_local_save_keeps_whatever_was_typed(self):
        form = self._form()
        form.on_key("\t")
        self._type(form, "https://x.supabase.co")
        form.focus = 4
        self.assertEqual(form.on_key("\r"), "saved")
        saved = relay_settings.load_saved(self.dir)
        self.assertEqual((saved["mode"], saved["url"]), ("local", "https://x.supabase.co"))
        self.assertIn("local mode", form.message)

    def test_supabase_with_missing_fields_is_refused_and_names_them(self):
        form = self._form()
        form.on_key(" ")
        self.assertEqual(form.mode, "supabase")
        form.focus = 4
        self.assertIsNone(form.on_key("\r"))
        self.assertIn("Not saved. Missing: Project URL", form.message)
        self.assertIn("Room id (8 characters)", form.message)
        self.assertFalse(os.path.exists(os.path.join(self.dir, relay_settings.FILE)))

    def test_a_complete_supabase_save(self):
        form = self._form()
        form.open_supabase()
        self.assertEqual(form.focus, 1)
        for value in ("https://x.supabase.co", "dummy-test-key", "Alice"):
            self._type(form, value)
            form.on_key("\r")
        self._type(form, "ABCD1234")
        self.assertEqual(form.on_key("\r"), "saved")
        self.assertEqual(relay_settings.load_saved(self.dir)["mode"], "supabase")
        self.assertIn("Supabase mode", form.message)

    def test_a_bad_room_code_is_refused(self):
        form = self._form()
        form.open_supabase()
        for value in ("https://x.supabase.co", "dummy-test-key", "Alice"):
            self._type(form, value)
            form.on_key("\r")
        self._type(form, "XY")
        self.assertIsNone(form.on_key("\r"))
        self.assertIn("not saved", form.message)

    def _finish(self, form):
        """Wait for the background create-room request and apply its answer."""
        for _ in range(200):
            if form.poll():
                return
            time.sleep(0.01)
        self.fail("create room did not come back")

    def test_ctrl_n_needs_supabase_keys_and_fills_the_room_field(self):
        form = self._form()
        form.on_key(CTRL_N)
        self.assertIn("Supabase mode first", form.message)
        form.open_supabase()
        form.on_key(CTRL_N)
        self.assertIn("fill in", form.message)
        form.fields[:3] = ["https://x.supabase.co", "dummy-test-key", "Alice"]
        with mock.patch.object(relay_rest, "create_room", return_value="ABCD1234") as create:
            form.on_key(CTRL_N)
            self._finish(form)
        self.assertEqual(form.fields[3], "ABCD1234")
        self.assertIn("room created: ABCD1234", form.message)
        self.assertEqual(create.call_args.args[2], "Alice")  # the signing seed is the second argument; it is not shown
        with mock.patch.object(relay_rest, "create_room", side_effect=OSError("offline")):
            form.on_key(CTRL_N)
            self._finish(form)
        self.assertIn("create room failed: offline", form.message)

    def test_a_failed_row_write_stops_the_create(self):
        form = self._form()
        form.open_supabase()
        form.fields[:3] = ["https://x.supabase.co", "dummy-test-key", "Alice"]
        with mock.patch.object(relay_rest, "put_status", return_value=409),                 mock.patch.object(relay_rest, "create_room") as create:
            form.on_key(CTRL_N)
            self._finish(form)
        create.assert_not_called()
        self.assertIn("409", form.message)
        self.assertEqual(form.fields[3], "")

    def test_the_row_is_written_before_the_create(self):
        form = self._form()
        form.open_supabase()
        form.fields[:3] = ["https://x.supabase.co", "dummy-test-key", "Alice"]
        order = []
        with mock.patch.object(relay_rest, "put_status", side_effect=lambda *a: order.append("row") or 204),                 mock.patch.object(relay_rest, "create_room", side_effect=lambda *a: order.append("create") or "ABCD1234"):
            form.on_key(CTRL_N)
            self._finish(form)
        self.assertEqual(order, ["row", "create"])

    def test_ctrl_n_does_not_wait_for_the_relay(self):
        form = self._form()
        form.open_supabase()
        form.fields[:3] = ["https://x.supabase.co", "dummy-test-key", "Alice"]
        release = threading.Event()

        def slow(config, peer_pk, name):
            release.wait(5)
            return "ABCD1234"

        with mock.patch.object(relay_rest, "create_room", side_effect=slow):
            started = time.monotonic()
            form.on_key(CTRL_N)
            self.assertLess(time.monotonic() - started, 1.0, "the key returns at once")
            self.assertEqual(form.message, "create room: working...")
            self.assertFalse(form.poll(), "no answer yet")
            release.set()
            self._finish(form)
        self.assertEqual(form.fields[3], "ABCD1234")

    def test_escape_leaves_without_saving(self):
        form = self._form()
        form.on_key("\t")
        self._type(form, "abc")
        self.assertEqual(form.on_key("\x1b"), "cancel")
        self.assertFalse(os.path.exists(os.path.join(self.dir, relay_settings.FILE)))

    def test_backspace_edits_and_the_mode_row_takes_no_text(self):
        form = self._form()
        self._type(form, "zzz")
        self.assertEqual(form.fields, ["", "", "", ""])
        form.on_key("\t")
        self._type(form, "ab")
        form.on_key("\x7f")
        self.assertEqual(form.fields[0], "a")

    def test_the_key_is_drawn_as_a_length_never_as_a_value(self):
        form = self._form()
        form.fields[1] = "dummy-test-key"
        text = _plain(form.render(100, 30))
        self.assertIn("(14 chars)", text)
        self.assertNotIn("dummy-test-key", text)
        self.assertIn("aa" * 32, text)

    def test_saved_values_are_loaded_and_the_mode_row_shows_the_mode(self):
        relay_settings.save(self.dir, {"mode": "supabase", "url": "https://x.supabase.co", "key": "k", "name": "Alice", "room_code": "ABCD1234"})
        form = self._form()
        self.assertEqual(form.mode, "supabase")
        self.assertIn("[Supabase]", _plain(form.render(100, 30)))
        form.toggle_mode()
        self.assertIn("[Local (LAN)]", _plain(form.render(100, 30)))

    def test_environment_values_are_marked(self):
        os.environ["HERDR_TALK_RELAY_URL"] = "https://env.supabase.co"
        try:
            text = _plain(self._form().render(100, 30))
        finally:
            del os.environ["HERDR_TALK_RELAY_URL"]
        self.assertIn("Project URL: (environment)", text)


if __name__ == "__main__":
    unittest.main()
