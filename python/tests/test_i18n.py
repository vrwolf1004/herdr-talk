import os
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

import i18n  # noqa: E402


class ActiveLanguageOnlyTest(unittest.TestCase):
    def setUp(self):
        self.saved = (i18n._language, i18n._messages, i18n._english)

    def tearDown(self):
        i18n._language, i18n._messages, i18n._english = self.saved

    def test_a_key_the_active_language_has_does_not_read_english(self):
        i18n.set_language("ko")
        i18n._english = None
        loaded = []
        real = i18n._load
        with mock.patch.object(i18n, "_load", side_effect=lambda lang: loaded.append(lang) or real(lang)):
            text = i18n.t("chat.guide")
        self.assertIn("보내기", text)
        self.assertEqual(loaded, [], "the key is in ko: no other language file is read")
        self.assertIsNone(i18n._english)

    def test_a_key_missing_in_the_active_language_comes_from_english(self):
        i18n.set_language("ko")
        i18n._messages = {}  # simulate a missing key in the active language
        self.assertEqual(i18n.t("chat.guide"), i18n._load("en")["chat.guide"])

    def test_an_unknown_key_is_shown_as_the_key(self):
        i18n.set_language("en")
        self.assertEqual(i18n.t("no.such.key"), "no.such.key")


if __name__ == "__main__":
    unittest.main()
