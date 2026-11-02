import json
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))
os.environ.setdefault("HERDR_TALK_LANG", "en")

import talk  # noqa: E402

ESC = "\x1b"
BELL = "\x07"


class FakeSocket:
    """받을 패킷을 차례로 주고, 다 떨어지면 종료 신호를 보낸 뒤 OSError 를 낸다 (루프 종료)."""

    def __init__(self, packets):
        self.packets = list(packets)

    def recvfrom(self, size):
        if not self.packets:
            talk.STOP.set()
            raise OSError("closed")
        return self.packets.pop(0), ("10.0.0.9", 52311)


class FakeConn:
    """받은 바이트를 한 번에 주고 닫는 연결."""

    def __init__(self, data):
        self.data = data

    def settimeout(self, _seconds):
        pass

    def recv(self, _size):
        data, self.data = self.data, b""
        return data

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class CollectSayTest(unittest.TestCase):
    def setUp(self):
        self.lines = []
        self.saved = talk._sink
        talk._sink = lambda line, style="": self.lines.append(line)
        talk.STOP.clear()

    def tearDown(self):
        talk._sink = self.saved
        talk.STOP.clear()


class PeerNameTest(CollectSayTest):
    def test_control_characters_become_spaces_and_the_name_is_cut_to_32(self):
        name = talk.peer_name("Li" + ESC + "am" + "x" * 40)
        self.assertNotIn(ESC, name)
        self.assertLessEqual(len(name), 32)
        self.assertTrue(name.startswith("Li"))

    def test_empty_or_blank_name_becomes_unknown(self):
        for value in ("", "   ", BELL + "\x00", None, 42, ["Liam"]):
            with self.subTest(value=value):
                self.assertEqual(talk.peer_name(value), "unknown")

    def test_same_rule_as_the_message_sender(self):
        raw = "Liam" + BELL + ESC + "[31m" + "y" * 50
        self.assertEqual(talk.peer_name(raw), talk.clean_text(raw, 32))


class JoinNoticeTest(CollectSayTest):
    def packet(self, peer_id, name):
        return json.dumps({"app": talk.APP, "id": peer_id, "name": name, "port": 52311}).encode("utf-8")

    def test_join_notice_and_stored_name_use_the_cleaned_name(self):
        chat = talk.Chat({"id": "me", "name": "Me", "port": 1})
        raw = "Li" + ESC + "am" + "z" * 60
        talk._listen_loop(FakeSocket([self.packet("p1", raw)]), chat)
        stored = chat.peers["p1"]["name"]
        self.assertNotIn(ESC, stored)
        self.assertLessEqual(len(stored), 32)
        self.assertTrue(any(stored in line for line in self.lines), self.lines)
        self.assertNotIn(ESC, "".join(self.lines))

    def test_blank_name_is_stored_and_announced_as_unknown(self):
        chat = talk.Chat({"id": "me", "name": "Me", "port": 1})
        talk._listen_loop(FakeSocket([self.packet("p2", BELL + "\x02")]), chat)
        self.assertEqual(chat.peers["p2"]["name"], "unknown")
        self.assertTrue(any("unknown" in line for line in self.lines), self.lines)


class SenderNameTest(CollectSayTest):
    """메시지 보낸 사람 이름은 기존 동작 그대로 (제어 문자 공백, 32자, 빈 값이면 unknown)."""

    def send(self, sender, text="hi"):
        payload = json.dumps({"app": talk.APP, "from": sender, "text": text}).encode("utf-8")
        talk.receive_one(FakeConn(payload))

    def test_sender_name_is_cleaned_and_cut_to_32_as_before(self):
        raw = "Ka" + ESC + "ren" + "q" * 40
        self.send(raw)
        self.assertEqual(self.lines, [f"{talk.clean_text(raw, 32)}: hi"])

    def test_sender_of_only_control_characters_keeps_the_old_result(self):
        self.send(BELL + "\x02")
        self.assertEqual(self.lines, ["  : hi"])


class JoinNameMatchesSenderTest(CollectSayTest):
    def test_the_same_raw_name_gives_the_same_cleaned_name_for_sender_and_peer(self):
        raw = "Li" + ESC + "am" + "z" * 40
        chat = talk.Chat({"id": "me", "name": "Me", "port": 1})
        packet = json.dumps({"app": talk.APP, "id": "p9", "name": raw, "port": 1}).encode("utf-8")
        talk._listen_loop(FakeSocket([packet]), chat)
        self.assertEqual(chat.peers["p9"]["name"], talk.clean_text(raw, 32))


if __name__ == "__main__":
    unittest.main()
