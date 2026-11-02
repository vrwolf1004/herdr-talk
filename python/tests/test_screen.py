"""The chat screen: the same layout and keys as the Rust edition (rust/crates/herdr-talk/src/screen.rs)."""
import os
import sys
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import talk  # noqa: E402


def make_chat():
    return talk.Chat({"id": "me", "name": "Haruto", "port": 52311})


class FrameTest(unittest.TestCase):
    def frame(self, lines, typed="", cols=30, rows=8):
        return talk.screen_frame(lines, "STATUS", typed, cols, rows)

    def test_status_row_is_grey_and_padded_to_the_width(self):
        frame = self.frame([])
        self.assertIn("\x1b[1;1H\x1b[100m\x1b[37mSTATUS" + " " * 24 + "\x1b[0m", frame)

    def test_messages_start_under_the_status_row_and_stop_above_the_divider(self):
        frame = self.frame([("a", False), ("b", False)], rows=8)
        self.assertIn("\x1b[2;1H\x1b[2Ka", frame)
        self.assertIn("\x1b[3;1H\x1b[2Kb", frame)
        # rows=8: status row 1, messages rows 2..5, divider row 7, input row 8.
        self.assertIn("\x1b[7;1H\x1b[90m" + "-" * 30 + "\x1b[0m", frame)
        self.assertIn("\x1b[8;1H\x1b[2K> ", frame)

    def test_only_the_newest_lines_fit(self):
        lines = [(f"line{i}", False) for i in range(10)]
        frame = self.frame(lines, rows=8)   # 8 - 3 = 5 message rows
        self.assertIn("line5", frame)
        self.assertIn("line9", frame)
        self.assertNotIn("line4", frame)

    def test_my_lines_are_bold_blue(self):
        frame = self.frame([("Haruto (you): hi", True), ("Liam: yo", False)])
        self.assertIn("\x1b[1m\x1b[34mHaruto (you): hi\x1b[0m", frame)
        self.assertIn("Liam: yo", frame)
        self.assertNotIn("\x1b[34mLiam", frame)

    def test_input_row_shows_the_typed_text_at_the_end(self):
        frame = self.frame([], typed="hello")
        self.assertTrue(frame.endswith("\x1b[8;1H\x1b[2K> hello\x1b[?25h"))

    def test_long_input_keeps_its_end_visible(self):
        frame = self.frame([], typed="a" * 40 + "END", cols=30)
        self.assertIn("END", frame)
        self.assertNotIn("a" * 40, frame)

    def test_wide_characters_are_padded_by_display_width(self):
        frame = talk.screen_frame([], "가나", "", 20, 8)
        # 가나 is 4 columns, so 16 spaces pad the status row to 20.
        self.assertIn("가나" + " " * 16 + "\x1b[0m", frame)


class KeyTest(unittest.TestCase):
    def setUp(self):
        self.chat = make_chat()
        self.saved_sink = talk._sink
        talk._sink = self.chat.add_line
        self.saved_send = talk.send_to
        self.sent = []
        talk.send_to = lambda peer, me, text: self.sent.append(text)

    def tearDown(self):
        talk._sink = self.saved_sink
        talk.send_to = self.saved_send

    def press(self, typed, *keys):
        for key in keys:
            if not talk.on_key(key, self.chat, typed):
                return False
        return True

    def texts(self):
        return [line for line, _ in self.chat.lines]

    def test_typing_and_backspace(self):
        typed = [""]
        self.press(typed, "a", "b", "\x7f")
        self.assertEqual(typed[0], "a")
        self.press(typed, "\x08", "\x08")
        self.assertEqual(typed[0], "")

    def test_ctrl_n_starts_a_rename(self):
        typed = ["old"]
        self.press(typed, "\x0e")
        self.assertEqual(typed[0], "/name ")

    def test_ctrl_l_clears_the_messages(self):
        self.chat.add_line("one")
        self.press([""], "\x0c")
        self.assertEqual(self.chat.lines, [])

    def test_ctrl_c_and_ctrl_d_leave(self):
        self.assertFalse(talk.on_key("\x03", self.chat, [""]))
        self.assertFalse(talk.on_key("\x04", self.chat, [""]))

    def test_enter_runs_a_command_and_echoes_it(self):
        typed = ["/peers"]
        self.assertTrue(talk.on_key("\r", self.chat, typed))
        self.assertEqual(typed[0], "")
        self.assertEqual(self.texts()[0], "> /peers")

    def test_enter_on_quit_says_leave(self):
        self.assertFalse(talk.on_key("\r", self.chat, ["/quit"]))

    def test_a_chat_message_is_sent_and_shown_as_mine(self):
        self.chat.add_line("Liam: hi")
        self.chat.peers = {"p": {"name": "Liam", "ip": "10.0.0.2", "port": 52311, "seen": time.time()}}
        typed = ["hello"]
        talk.on_key("\r", self.chat, typed)
        self.assertEqual(self.sent, ["hello"])
        self.assertEqual(self.chat.lines[-1], ("Haruto (you): hello", True))

    def test_help_puts_the_box_in_the_messages(self):
        typed = ["/help"]
        talk.on_key("\r", self.chat, typed)
        self.assertEqual(self.texts(), ["> /help"] + talk.HELP_BOX)


class FitTest(unittest.TestCase):
    def test_fit_and_tail_keep_to_the_width(self):
        self.assertEqual(talk.fit("가나다", 5), "가나")
        self.assertEqual(talk.tail("가나다", 5), "나다")
        self.assertEqual(talk.fit("abc", 10), "abc")


if __name__ == "__main__":
    unittest.main()
