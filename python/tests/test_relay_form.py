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

from extend import settings_form as relay_form  # noqa: E402
from core import relay_key  # noqa: E402
from core import relay_rest  # noqa: E402
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
        self.assertIn("01. BASE STATION", text)
        self.assertIn("02. CLOUD SERVICE", text)
        self.assertIn("Starting in local mode in 30s", text)


class SettingsFormTests(unittest.TestCase):
    URL = "https://x.supabase.co"
    KEY = "dummy-test-key"

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-form-")
        self.old_env = {name: os.environ.pop(name, None) for name in ("HERDR_TALK_RELAY_URL", "HERDR_TALK_RELAY_KEY")}
        # The owner row is written before a create: no network and no identity files in these tests.
        patches = [mock.patch.object(relay_key, "noise_public_hex", return_value="bb" * 32),
                   mock.patch.object(relay_key, "load_or_create_short_id", return_value="ABCD2345"),
                   mock.patch.object(relay_rest, "put_status", return_value=201),
                   mock.patch.object(relay_rest, "redeem_setup_code", return_value=(self.URL, self.KEY))]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        self.redeem = relay_rest.redeem_setup_code  # the mock, patched above

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

    def _fill(self, form, code="", name="Alice", room="", trust=""):
        """Type each of the four rows and press Enter on each; the last Enter is the save."""
        for value in (code, trust, name, room):
            self._type(form, value)
            result = form.on_key("\r")
        return result

    def _finish(self, form):
        """Wait for the background redeem or create-room request and apply its answer."""
        for _ in range(200):
            if form.poll():
                return
            time.sleep(0.01)
        self.fail("the background request did not come back")

    def _save_keys(self):
        """Saved URL and key with no room yet: local mode, because Supabase mode needs a room in the file."""
        relay_settings.save(self.dir, {"mode": "local", "url": self.URL, "key": self.KEY, "name": "Alice", "room_code": "", "trust": ""})

    def _save_supabase(self, room="ABCD1234"):
        relay_settings.save(self.dir, {"mode": "supabase", "url": self.URL, "key": self.KEY,
                                       "name": "Alice", "room_code": room, "trust": "12345678"})

    def test_four_rows_setup_code_name_room_trust(self):
        form = self._form()
        form.open_supabase()
        text = _plain(form.render(100, 30))
        self.assertIn("▶ Setup code", text)
        self.assertIn("Display name", text)
        self.assertIn("Room id (8 characters)", text)
        self.assertIn("Trust code (8 digits)", text)
        self.assertNotIn("Project URL", text)
        self.assertNotIn("Publishable key", text)
        for _ in range(4):
            form.on_key("\t")
        self.assertEqual(form.focus, 0, "Tab wraps after the four rows")

    def test_local_save_without_a_code_needs_no_keys(self):
        form = self._form()
        self.assertEqual(self._fill(form, name="", room="", trust=""), "saved")
        saved = relay_settings.load_saved(self.dir)
        self.assertEqual((saved["mode"], saved["url"], saved["key"]), ("local", "", ""))
        self.assertIn("local mode", form.message)

    def test_supabase_without_a_code_or_saved_keys_asks_for_the_setup_code(self):
        form = self._form()
        form.open_supabase()
        self.assertIsNone(self._fill(form, name="Alice", room="ABCD1234", trust="12345678"))
        self.assertIn("Type the setup code first", form.message)
        self.redeem.assert_not_called()
        self.assertFalse(os.path.exists(os.path.join(self.dir, relay_settings.FILE)))

    def test_supabase_missing_fields_are_named(self):
        form = self._form()
        form.open_supabase()
        form.url, form.key = self.URL, self.KEY  # saved keys, as after an earlier setup code
        self.assertIsNone(self._fill(form, name="", room="", trust=""))
        self.assertIn("Not saved. Missing: Trust code (8 digits), Display name, Room id (8 characters)", form.message)
        self.assertFalse(os.path.exists(os.path.join(self.dir, relay_settings.FILE)))

    def test_enter_on_the_code_row_saves_the_url_and_key_at_once_then_the_final_enter_saves_the_rest(self):
        form = self._form()
        form.open_supabase()
        self._type(form, "abcd-2345")
        form.on_key(relay_form.KEY_F2)  # F2 on the setup-code row: redeem, save URL and key at once
        self.assertIn("checking", form.message)
        self._finish(form)
        self.assertEqual(self.redeem.call_args.args[0], "ABCD2345", "the code is sent normalized")
        saved = relay_settings.load_saved(self.dir)
        self.assertEqual((saved["mode"], saved["url"], saved["key"]), ("supabase", self.URL, self.KEY))
        self.assertEqual(saved["name"], "", "the name is not saved yet")
        self.assertIn("saved", form.message)
        form.on_key("\r")  # Enter moves down to the display name row
        self._type(form, "12345678")
        form.on_key("\r")
        self._type(form, "Alice")
        form.on_key("\r")
        self._type(form, "ABCD1234")
        self.assertEqual(form.on_key("\r"), "saved")
        self.assertEqual(self.redeem.call_count, 1, "the final Enter does not redeem again")
        saved = relay_settings.load_saved(self.dir)
        self.assertEqual((saved["url"], saved["key"]), (self.URL, self.KEY))
        self.assertEqual((saved["name"], saved["room_code"], saved["trust"]), ("Alice", "ABCD1234", "12345678"))
        self.assertEqual(form.fields[relay_form.CODE_FIELD], "", "the typed code is cleared after the save")

    def test_enter_on_the_code_row_only_moves_down_and_ctrl_s_elsewhere_is_refused(self):
        form = self._form()
        form.open_supabase()
        self._type(form, "abcd-2345")
        form.on_key("\r")
        self.assertEqual(form.focus, 1, "Enter only moves down")
        self.redeem.assert_not_called()
        form.on_key(relay_form.KEY_F2)
        self.assertIn("F2", form.message)
        self.redeem.assert_not_called()

    def test_f9_asks_for_the_passphrase_and_shows_the_issued_code(self):
        form = self._form()
        form.open_supabase()
        form.on_key(relay_form.KEY_F9)
        self.assertIn("operator passphrase", form.message)
        for ch in "pw":
            form.on_key(ch)
        self.assertIn("operator passphrase: [**]", _plain(form.render(100, 30)))  # the passphrase is masked, never drawn
        with mock.patch.object(relay_rest, "issue_setup_code", return_value="T5N9Q1ZB") as issue:
            form.on_key("\r")
            self._finish(form)
        issue.assert_called_once_with("pw")
        self.assertIn("T5N9Q1ZB", form.message)

    def test_a_bad_code_is_refused_without_a_request(self):
        form = self._form()
        form.open_supabase()
        self.assertIsNone(self._fill(form, code="ABC", name="Alice", room="ABCD1234", trust="12345678"))
        self.assertIn("8 letters or digits", form.message)
        self.redeem.assert_not_called()

    def test_redeem_answers_are_shown_and_nothing_is_saved(self):
        for error in (ValueError("the setup code is wrong, used or expired"), ValueError("too many wrong codes; try again later"),
                      OSError("offline")):
            with self.subTest(error=str(error)):
                form = self._form()
                form.open_supabase()
                self.redeem.side_effect = error
                self._fill(form, code="ABCD2345", name="Alice", room="ABCD1234", trust="12345678")
                self._finish(form)
                self.assertIn(f"setup code failed: {error}", form.message)
                self.assertFalse(os.path.exists(os.path.join(self.dir, relay_settings.FILE)))
                self.redeem.side_effect = None
        self.redeem.side_effect = None

    def test_a_save_without_a_code_keeps_the_saved_keys(self):
        self._save_supabase()
        form = self._form()
        self.assertEqual((form.url, form.key), (self.URL, self.KEY))
        form.focus = relay_form.ROOM_FIELD
        self.assertEqual(form.on_key("\r"), "saved")
        self.redeem.assert_not_called()
        saved = relay_settings.load_saved(self.dir)
        self.assertEqual((saved["url"], saved["key"]), (self.URL, self.KEY))

    def test_the_redeem_does_not_freeze_the_screen(self):
        form = self._form()
        form.open_supabase()
        release = threading.Event()
        self.redeem.side_effect = lambda code, url=None: release.wait(5) and (self.URL, self.KEY)
        self._type(form, "ABCD2345")
        form.focus = relay_form.ROOM_FIELD
        started = time.monotonic()
        form.on_key("\r")
        self.assertLess(time.monotonic() - started, 1.0, "the key returns at once")
        self.assertFalse(form.poll(), "no answer yet")
        self.assertIsNone(form.on_key("\r"), "a second Enter while redeeming does not start another")
        release.set()
        self._finish(form)
        self.redeem.side_effect = None

    def test_ctrl_n_needs_saved_keys_and_a_name_and_fills_the_room_field(self):
        form = self._form()
        form.on_key(CTRL_N)
        self.assertIn("Supabase mode first", form.message)
        form.open_supabase()
        form.on_key(CTRL_N)
        self.assertIn("save the setup code", form.message)
        self._save_keys()
        form.fields[relay_form.NAME_FIELD] = "Alice"
        form.fields[relay_form.TRUST_FIELD] = "12345678"
        with mock.patch.object(relay_rest, "create_room", return_value="ABCD1234") as create:
            form.on_key(CTRL_N)
            self._finish(form)
        self.assertEqual(form.fields[relay_form.ROOM_FIELD], "ABCD1234")
        self.assertIn("room created: ABCD1234", form.message)
        self.assertEqual(create.call_args.args[2:], ("Alice", "12345678"))  # the signing seed is the second argument; it is not shown
        with mock.patch.object(relay_rest, "create_room", side_effect=OSError("offline")):
            form.on_key(CTRL_N)
            self._finish(form)
        self.assertIn("create room failed: offline", form.message)

    def test_a_failed_row_write_stops_the_create(self):
        self._save_keys()
        form = self._form()
        form.open_supabase()
        form.fields[relay_form.NAME_FIELD] = "Alice"
        form.fields[relay_form.TRUST_FIELD] = "12345678"
        with mock.patch.object(relay_rest, "put_status", return_value=409), \
                mock.patch.object(relay_rest, "create_room") as create:
            form.on_key(CTRL_N)
            self._finish(form)
        create.assert_not_called()
        self.assertIn("409", form.message)
        self.assertEqual(form.fields[relay_form.ROOM_FIELD], "")

    def test_the_row_is_written_before_the_create(self):
        self._save_keys()
        form = self._form()
        form.open_supabase()
        form.fields[relay_form.NAME_FIELD] = "Alice"
        form.fields[relay_form.TRUST_FIELD] = "12345678"
        order = []
        with mock.patch.object(relay_rest, "put_status", side_effect=lambda *a: order.append("row") or 204), \
                mock.patch.object(relay_rest, "create_room", side_effect=lambda *a: order.append("create") or "ABCD1234"):
            form.on_key(CTRL_N)
            self._finish(form)
        self.assertEqual(order, ["row", "create"])

    def test_ctrl_n_does_not_wait_for_the_relay(self):
        self._save_keys()
        form = self._form()
        form.open_supabase()
        form.fields[relay_form.NAME_FIELD] = "Alice"
        form.fields[relay_form.TRUST_FIELD] = "12345678"
        release = threading.Event()

        def slow(config, seed, name, trust):
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
        self.assertEqual(form.fields[relay_form.ROOM_FIELD], "ABCD1234")

    def test_only_one_room_is_allowed_and_the_same_room_again_is_fine(self):
        self._save_supabase(room="ABCD1234")
        form = self._form()
        form.fields[relay_form.ROOM_FIELD] = "EFGH5678"
        self.assertFalse(form.save())
        self.assertIn("ABCD1234", form.message)
        form.fields[relay_form.ROOM_FIELD] = "abcd1234"
        self.assertTrue(form.save(), "the same room, typed in another case, is allowed")

    def test_ctrl_n_is_refused_while_a_room_is_saved(self):
        self._save_supabase(room="ABCD1234")
        form = self._form()
        form.create_room()
        self.assertIn("ABCD1234", form.message)
        self.assertIsNone(form._room_result, "no create request is sent")

    def test_after_leaving_another_room_can_be_entered_or_created(self):
        self._save_supabase(room="ABCD1234")
        form = self._form()
        form.open_supabase(left=True)
        self.assertEqual((form.url, form.key), (self.URL, self.KEY), "the server info is kept")
        form.fields[relay_form.ROOM_FIELD] = "EFGH5678"
        self.assertTrue(form.save(), "another room is allowed after leaving")
        self.assertEqual(relay_settings.load_saved(self.dir)["room_code"], "EFGH5678")
        with mock.patch.object(relay_form.threading, "Thread") as thread:
            form.create_room()
        self.assertIsNotNone(form._room_result, "a create request starts instead of the one-room refusal")
        thread.assert_called_once()

    def test_escape_leaves_without_saving(self):
        form = self._form()
        form.on_key("\t")
        self._type(form, "abc")
        self.assertEqual(form.on_key("\x1b"), "cancel")
        self.assertFalse(os.path.exists(os.path.join(self.dir, relay_settings.FILE)))

    def test_backspace_edits_the_focused_field_and_tab_cycles_the_four(self):
        form = self._form()
        self._type(form, "abc")
        form.on_key("\x7f")
        self.assertEqual(form.fields, ["ab", "", "", ""])
        for _ in range(4):
            form.on_key("\t")
        self.assertEqual(form.focus, 0)
        form.on_key("\t")
        self._type(form, "xy")
        form.on_key("\x7f")
        self.assertEqual(form.fields, ["ab", "x", "", ""])

    def test_the_key_is_shown_as_a_length_never_as_a_value(self):
        self._save_supabase()
        form = self._form()
        text = _plain(form.render(100, 30))
        self.assertIn("(saved, key 14 chars)", text)
        self.assertNotIn(self.KEY, text)
        self.assertIn("aa" * 32, text)

    def test_saved_values_are_loaded_and_no_mode_row_is_drawn(self):
        self._save_supabase()
        form = self._form()
        self.assertEqual(form.mode, "supabase")
        text = _plain(form.render(100, 30))
        self.assertNotIn("MODE", text)
        self.assertIn("▶ Setup code", text)
        self.assertIn("Alice", text)

    def test_a_frame_overwrites_rows_in_place_without_clearing_the_screen(self):
        frame = self._form().render(100, 30)
        self.assertNotIn("\x1b[2J", frame)
        self.assertIn(relay_form.place_row(3, "SUPABASE SETTINGS", relay_form.ACCENT + relay_form.BOLD), frame)
        self.assertIn(relay_form.place_row(29, ""), frame)   # an unused row is blanked, so nothing from an older frame stays
        self.assertIn("\x1b[30;1H", frame)   # the status line is the bottom row
        self.assertIn("[SYSTEM READY]", frame)
        self.assertIn("made by Python" + relay_form.RESET + relay_form.BACKGROUND + "\x1b[K", frame)

    def test_environment_values_are_marked(self):
        os.environ["HERDR_TALK_RELAY_URL"] = "https://env.supabase.co"
        try:
            text = _plain(self._form().render(100, 30))
        finally:
            del os.environ["HERDR_TALK_RELAY_URL"]
        self.assertIn("[(environment)", text)


if __name__ == "__main__":
    unittest.main()
