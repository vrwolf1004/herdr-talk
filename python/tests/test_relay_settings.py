import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

import relay_settings as rs  # noqa: E402


def _sample():
    return {"mode": rs.SUPABASE, "url": "https://example.supabase.co", "key": "dummy-test-key", "name": "Alice", "peer": "ab12cd34"}


class SettingsTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-relay-settings-")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_saved_settings_come_back_as_they_were(self):
        self.assertIsNone(rs.save(self.dir, _sample()))
        self.assertEqual(rs.load_saved(self.dir), _sample())

    def test_missing_or_damaged_file_means_no_settings(self):
        self.assertEqual(rs.load_saved(self.dir), rs.empty())
        with open(os.path.join(self.dir, rs.FILE), "w") as handle:
            handle.write("{not json")
        self.assertEqual(rs.load_saved(self.dir), rs.empty())
        self.assertTrue(rs.needs_first_run(self.dir))

    def test_mode_missing_or_unknown_is_local(self):
        with open(os.path.join(self.dir, rs.FILE), "w") as handle:
            handle.write('{"url":"x"}')
        self.assertEqual(rs.load_saved(self.dir)["mode"], rs.LOCAL)
        self.assertTrue(rs.needs_first_run(self.dir))
        with open(os.path.join(self.dir, rs.FILE), "w") as handle:
            handle.write('{"mode":"Supabase"}')
        self.assertEqual(rs.load_saved(self.dir)["mode"], rs.LOCAL)
        self.assertFalse(rs.needs_first_run(self.dir))

    def test_a_byte_order_mark_is_read_and_never_written(self):
        with open(os.path.join(self.dir, rs.FILE), "wb") as handle:
            handle.write(b"\xef\xbb\xbf" + b'{"mode":"supabase","name":"Bob"}')
        self.assertEqual(rs.load_saved(self.dir)["name"], "Bob")
        self.assertEqual(rs.load_saved(self.dir)["mode"], rs.SUPABASE)
        self.assertIsNone(rs.save(self.dir, _sample()))
        with open(os.path.join(self.dir, rs.FILE), "rb") as handle:
            self.assertNotEqual(handle.read(3), b"\xef\xbb\xbf")

    def test_supabase_needs_all_four_and_valid_values(self):
        for name in ("url", "key", "name", "peer"):
            incomplete = {**_sample(), name: ""}
            self.assertIsNotNone(rs.validate(incomplete), name)
        self.assertIsNotNone(rs.validate({**_sample(), "url": "http://example.com"}))
        self.assertIsNotNone(rs.validate({**_sample(), "peer": "AB" * 32}))
        self.assertIsNotNone(rs.validate({**_sample(), "name": "x" * 33}))
        self.assertIsNone(rs.validate(_sample()))

    def test_local_mode_keeps_partial_values(self):
        partial = {"mode": rs.LOCAL, "url": "https://example.supabase.co", "key": "", "name": "", "peer": ""}
        self.assertIsNone(rs.save(self.dir, partial))
        self.assertEqual(rs.load_saved(self.dir), partial)

    def test_set_mode_keeps_the_other_values(self):
        rs.save(self.dir, _sample())
        self.assertIsNone(rs.set_mode(self.dir, rs.LOCAL))
        saved = rs.load_saved(self.dir)
        self.assertEqual(saved["mode"], rs.LOCAL)
        self.assertEqual(saved["name"], "Alice")

    def test_environment_wins_for_url_and_key(self):
        rs.save(self.dir, _sample())
        settings = rs.effective(self.dir, env_url="https://env.supabase.co", env_key="env-key")
        self.assertEqual((settings["url"], settings["key"]), ("https://env.supabase.co", "env-key"))
        self.assertEqual(settings["name"], "Alice")

    def test_key_label_shows_the_length_only(self):
        self.assertEqual(rs.key_label("abcd"), "(4 chars)")
        self.assertEqual(rs.key_label(""), "-")


if __name__ == "__main__":
    unittest.main()
