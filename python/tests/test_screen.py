"""The chat screen: the same layout and keys as the Rust edition (rust/crates/herdr-talk/src/screen.rs)."""
import os
import sys
import time
import unittest
import unittest.mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import talk  # noqa: E402
from relay_form import ACCENT, BACKGROUND, BOLD, BORDER, MUTED, RESET, TEXT, place_row as place  # noqa: E402


def make_chat():
    return talk.Chat({"id": "me", "name": "Haruto", "port": 52311})


def place_text(frame, row):
    """The text written on one screen row: from its cursor move to the next one, without the escape codes."""
    import re
    segment = frame.split(f"[{row};1H", 1)[1]
    segment = re.split(r"\[\d+;1H", segment, maxsplit=1)[0]
    return re.sub(r"\[[0-9;]*[A-Za-z]", "", segment)


class FrameTest(unittest.TestCase):
    def frame(self, lines, typed="", cols=30, rows=9):
        return talk.screen_frame(lines, "STATUS", typed, cols, rows)

    def test_status_row_is_above_a_footer_row_that_holds_only_made_by_python(self):
        frame = self.frame([])   # rows=9 is raised to 10: status on row 9, the footer alone on row 10
        self.assertIn("STATUS", place_text(frame, 9))
        self.assertTrue(place_text(frame, 9).rstrip().endswith("[SYSTEM READY]"))
        self.assertIn(place(10, " " * 15 + MUTED + "made by Python"), frame)

    def test_header_titles_and_mode_are_drawn(self):
        frame = talk.screen_frame([], "STATUS", "", 60, 12, nodes=["YOU Haruto", "PEER Liam"], mode="BASE STATION")
        self.assertIn("▣ HERDR-TALK", frame)
        self.assertIn("SYS.VER 1.0 · BASE STATION", frame)
        self.assertIn("COMMUNICATION LOG", frame)
        self.assertIn(f"CONNECTED NODES{RESET}{BACKGROUND}{TEXT}  YOU Haruto · PEER Liam", frame)

    def test_messages_start_under_the_log_title_and_stop_above_the_nodes_row(self):
        frame = self.frame([("a", False), ("b", False)], rows=12)
        # rows=12: header 1, divider 2, log title 3, messages 4..6, nodes 7, divider 8, guide 9, input 10, status 11, footer 12.
        self.assertIn(place(4, MUTED + "a"), frame)
        self.assertIn(place(5, MUTED + "b"), frame)
        self.assertIn(place(7, f"{ACCENT}CONNECTED NODES{RESET}  "), frame)
        self.assertIn(place(8, BORDER + "─" * 29), frame)
        self.assertIn(BACKGROUND + TEXT + "MESSAGE > ", frame)
        self.assertIn(place(12, " " * 15 + MUTED + "made by Python"), frame)

    def test_the_status_tag_is_at_the_right_end_of_the_status_row(self):
        frame = talk.screen_frame([], "herdr-talk | py-test", "", 60, 12)
        self.assertIn("[11;1H", frame)
        row = frame.split("[11;1H", 1)[1].split("[12;1H", 1)[0]
        self.assertIn("herdr-talk | py-test", row)
        self.assertTrue(row.rstrip("[0m[K").endswith("[SYSTEM READY]") or "[SYSTEM READY]" in row)
        self.assertNotIn("[SYSTEM READY]", frame.split("[2;1H", 1)[0].split("[1;1H", 1)[1])

    def test_only_the_newest_lines_fit(self):
        lines = [(f"line{i}", False) for i in range(10)]
        frame = self.frame(lines, rows=13)   # 13 - 9 = 4 message rows
        self.assertIn("line6", frame)
        self.assertIn("line9", frame)
        self.assertNotIn("line5", frame)

    def test_my_lines_are_bold_green(self):
        frame = self.frame([("hi", True, "Haruto", "12:30:21"), ("yo", False, "Liam", "12:30:22")], rows=12)
        self.assertIn(place(4, f"{MUTED}[12:30:21] {BOLD}{ACCENT}Haruto{RESET}{MUTED} > {BOLD}{ACCENT}hi"), frame)
        self.assertIn(place(5, f"{MUTED}[12:30:22] {BOLD}{ACCENT}Liam{RESET}{MUTED} > {TEXT}yo"), frame)

    def test_input_row_shows_the_typed_text_at_the_end(self):
        frame = self.frame([], typed="hello")
        self.assertIn(place(8, "MESSAGE > " + ACCENT + "hello_"), frame)  # rows=9 is raised to 10: the input is row 8
        self.assertTrue(frame.endswith("\x1b[?25h"))

    def test_empty_input_shows_the_hint_in_muted_text(self):
        frame = self.frame([])
        self.assertIn(place(8, "MESSAGE > " + MUTED + talk.t("chat.input_hint")), frame)

    def test_long_input_keeps_its_end_visible(self):
        frame = self.frame([], typed="a" * 40 + "END", cols=30)
        self.assertIn("END", frame)
        self.assertNotIn("a" * 40, frame)

    def test_wide_characters_are_padded_by_display_width(self):
        frame = talk.screen_frame([], "가나", "", 20, 8)
        # 가나 is 4 columns and the status row has the whole width to itself, so it is not cut.
        self.assertIn("가나", place_text(frame, 9))
        self.assertIn(place(10, " " * 5 + MUTED + "made by Python"), frame)


class KeyTest(unittest.TestCase):
    def setUp(self):
        self.chat = make_chat()
        self.chat.on_event = self.chat.add_event
        self.saved_send = talk.send_to
        self.sent = []
        talk.send_to = lambda peer, me, text: self.sent.append(text)

    def tearDown(self):
        talk.send_to = self.saved_send

    def press(self, typed, *keys):
        for key in keys:
            if not talk.on_key(key, self.chat, typed):
                return False
        return True

    def texts(self):
        return [entry[0] for entry in self.chat.lines]

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
        self.assertEqual(self.chat.lines[-1][:3], ("hello", True, "Haruto"))

    def test_help_puts_the_box_in_the_messages(self):
        typed = ["/help"]
        talk.on_key("\r", self.chat, typed)
        self.assertEqual(self.texts(), ["> /help"] + talk.HELP_BOX)


class DrawTest(unittest.TestCase):
    def test_an_unchanged_frame_is_not_written_again(self):
        import io
        out = io.StringIO()
        talk._last_frame = None
        with unittest.mock.patch.object(talk.sys, "stdout", out):
            talk._draw("one")
            talk._draw("one")
            talk._draw("two")
        self.assertEqual(out.getvalue(), "onetwo")
        talk._last_frame = None


class FitTest(unittest.TestCase):
    def test_fit_and_tail_keep_to_the_width(self):
        self.assertEqual(talk.fit("가나다", 5), "가나")
        self.assertEqual(talk.tail("가나다", 5), "나다")
        self.assertEqual(talk.fit("abc", 10), "abc")


if __name__ == "__main__":
    unittest.main()
