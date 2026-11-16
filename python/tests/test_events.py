import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

import talk  # noqa: E402
from core.events import Message, Notice  # noqa: E402


class EmitTest(unittest.TestCase):
    def test_emit_hands_the_event_to_the_screen_and_drops_it_without_one(self):
        chat = talk.Chat({"id": "me", "name": "Me", "port": 1})
        chat.on_event = None
        chat.emit(Notice("peer.joined", {"name": "B"}))  # no listener: nothing happens
        got = []
        chat.on_event = got.append
        chat.emit(Message("B", "hi"))
        chat.emit(Notice("peer.joined", {"name": "B", "ip": "10.0.0.2"}))
        self.assertEqual(got, [Message("B", "hi"), Notice("peer.joined", {"name": "B", "ip": "10.0.0.2"})])
        self.assertFalse(Message("B", "hi").mine)


if __name__ == "__main__":
    unittest.main()
