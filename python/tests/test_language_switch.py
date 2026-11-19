import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import i18n  # noqa: E402
import talk  # noqa: E402
from extend import settings_form  # noqa: E402

ORDER = ["en", "ko", "ru", "fr", "es", "de", "ja"]


class LanguageTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-lang-")
        self.saved_language = i18n.language()
        self.env = mock.patch.dict(os.environ, {"HERDR_PLUGIN_CONFIG_DIR": os.path.join(self.dir, "config")})
        self.env.start()
        self.addCleanup(self.env.stop)

    def tearDown(self):
        i18n.set_language(self.saved_language)
        shutil.rmtree(self.dir, ignore_errors=True)

    def config(self):
        return json.loads(Path(self.dir, "config", "config.json").read_text(encoding="utf-8"))

    def test_f3_walks_the_seven_languages_and_wraps(self):
        i18n.set_language("en")
        seen = [i18n.cycle_language() for _ in range(len(ORDER))]
        self.assertEqual(seen, ORDER[1:] + ORDER[:1])
        self.assertEqual(i18n.language(), "en")

    def test_the_choice_is_saved_and_other_keys_stay(self):
        Path(self.dir, "config").mkdir()
        Path(self.dir, "config", "config.json").write_text(json.dumps({"plaintext": True}), encoding="utf-8")
        i18n.set_language("en")
        i18n.cycle_language()
        self.assertEqual(self.config(), {"plaintext": True, "lang": "ko"})

    def test_a_missing_config_folder_is_created(self):
        i18n.set_language("ko")
        i18n.cycle_language()
        self.assertEqual(self.config(), {"lang": "ru"})

    def test_without_a_config_folder_the_language_still_changes(self):
        with mock.patch.dict(os.environ, {"HERDR_PLUGIN_CONFIG_DIR": ""}):
            i18n.set_language("en")
            self.assertEqual(i18n.cycle_language(), "ko")
        self.assertEqual(i18n.language(), "ko")
        self.assertFalse(Path(self.dir, "config").exists())

    def test_an_unreadable_config_is_left_alone(self):
        Path(self.dir, "config").mkdir()
        Path(self.dir, "config", "config.json").write_text("[1, 2]", encoding="utf-8")
        i18n.set_language("en")
        i18n.cycle_language()
        self.assertEqual(i18n.language(), "ko")
        self.assertEqual(Path(self.dir, "config", "config.json").read_text(encoding="utf-8"), "[1, 2]")


    def test_the_settings_form_changes_language_on_f3_and_draws_it(self):
        i18n.set_language("en")
        form = settings_form.SettingsForm(self.dir, mine=("aa" * 32, "bb" * 32))
        form.on_key(settings_form.KEY_F3)
        self.assertEqual(i18n.language(), "ko")
        self.assertIn(i18n.t("settings.setup_code"), form.render(100, 30))

    def test_the_chat_changes_language_on_f3_and_is_not_typed(self):
        i18n.set_language("en")
        chat = mock.MagicMock()
        chat.help_open = False
        typed = [""]
        self.assertTrue(talk.on_key(settings_form.KEY_F3, chat, typed))
        self.assertEqual(i18n.language(), "ko")
        self.assertEqual(typed[0], "")

    def test_f3_is_read_from_the_windows_console_code(self):
        fake = mock.MagicMock()
        fake.getwch.side_effect = ["\x00", "="]
        with mock.patch.dict(sys.modules, {"msvcrt": fake}), mock.patch.object(talk.os, "name", "nt"):
            self.assertEqual(talk.read_char(), settings_form.KEY_F3)

    def test_the_status_row_hint_names_f3_in_every_language(self):
        for language in ORDER:
            text = i18n._load(language).get("status.help_hint", "")
            self.assertIn("F3", text, language)


class ScreenTextTests(unittest.TestCase):
    """The settings and first-run screens draw translated text: a missing key would show its own name (e.g. form.title)."""

    def setUp(self):
        self.saved_language = i18n.language()
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-screen-")
        self.addCleanup(shutil.rmtree, self.dir, True)

    def tearDown(self):
        i18n.set_language(self.saved_language)

    def test_settings_and_first_run_screens_show_no_key_names_in_any_language(self):
        import re
        for language in ORDER:
            i18n.set_language(language)
            form = settings_form.SettingsForm(self.dir, mine=("aa" * 32, "bb" * 32))
            form.open_supabase()
            settings_text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", form.render(120, 40))
            first_text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", settings_form.FirstRun().render(120, 40))
            for text in (settings_text, first_text):
                for prefix in ("form.", "first.", "settings.", "mode."):
                    self.assertNotIn(prefix, text, language)
            self.assertIn(i18n.t("form.title"), settings_text, language)
            self.assertIn(i18n.t("form.identity"), settings_text, language)
            self.assertIn(i18n.t("first.hint"), first_text, language)


class FirstRunLanguageTests(unittest.TestCase):
    """F3 on the first-run screen works like on the other screens, and the mode names stay in English."""

    def setUp(self):
        self.saved_language = i18n.language()
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-first-")
        self.env = mock.patch.dict(os.environ, {"HERDR_PLUGIN_CONFIG_DIR": os.path.join(self.dir, "config")})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.addCleanup(shutil.rmtree, self.dir, True)

    def tearDown(self):
        i18n.set_language(self.saved_language)

    def test_f3_cycles_and_saves_and_the_countdown_keeps_running(self):
        import re
        i18n.set_language("en")
        clock = [1000.0]
        choice = settings_form.FirstRun(clock=lambda: clock[0])
        choice.on_key(settings_form.KEY_F3)
        self.assertEqual(i18n.language(), "ko")
        self.assertEqual(json.loads(Path(self.dir, "config", "config.json").read_text(encoding="utf-8")), {"lang": "ko"})
        self.assertEqual(choice.remaining(), 30)
        text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", choice.render(120, 40))
        self.assertIn(i18n.t("first.hint"), text)

    def test_mode_names_stay_english_and_the_hints_are_translated_in_every_language(self):
        import re
        for language in ORDER:
            i18n.set_language(language)
            text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", settings_form.FirstRun().render(120, 40))
            self.assertIn("BASE STATION", text, language)
            self.assertIn("CLOUD SERVICE", text, language)
            self.assertIn(i18n.t("mode.hint_local"), text, language)
            self.assertIn(i18n.t("mode.hint_supabase"), text, language)
            self.assertIn("F3", text, language)


class FieldAlignmentTests(unittest.TestCase):
    """The four input rows put their ' : [' at the same terminal column in every language (wide letters count as two)."""

    def setUp(self):
        self.saved_language = i18n.language()
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-align-")
        self.addCleanup(shutil.rmtree, self.dir, True)

    def tearDown(self):
        i18n.set_language(self.saved_language)

    def test_the_input_column_is_the_same_in_every_language(self):
        import re
        for language in ORDER:
            i18n.set_language(language)
            form = settings_form.SettingsForm(self.dir, mine=("aa" * 32, "bb" * 32))
            form.open_supabase()
            columns = []
            for row in re.split(r"\x1b\[\d+;1H", form.render(120, 40)):
                plain = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", row)
                if " : [" in plain:
                    columns.append(settings_form.char_width(plain.split(" : [", 1)[0]))
            self.assertEqual(len(columns), 4, language)
            self.assertEqual(len(set(columns)), 1, (language, columns))


if __name__ == "__main__":
    unittest.main()
