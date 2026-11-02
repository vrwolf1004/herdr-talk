import os
import sys
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))
os.environ.setdefault("HERDR_TALK_LANG", "en")

import talk  # noqa: E402


class CollectSayTest(unittest.TestCase):
    def setUp(self):
        self.lines = []
        self.saved = talk._sink
        talk._sink = lambda line, style="": self.lines.append(line)

    def tearDown(self):
        talk._sink = self.saved


class SendParallelTest(CollectSayTest):
    """메시지는 피어에게 동시에 보내고, 한 피어의 실패가 다른 피어를 막지 않는다. 실패 메시지와 기록은 기존과 같다."""

    DELAY = 1.0  # 가짜 피어 한 개가 응답하는 데 걸리는 시간. 실제 연결 제한 (3초) 대신 짧게 잡았다.

    def make_chat(self, names):
        chat = talk.Chat({"id": "me", "name": "Me", "port": 1})
        now = time.time()
        for index, name in enumerate(names):
            chat.peers[f"id{index}"] = {"name": name, "ip": "10.0.0.%d" % (index + 1), "port": 52311, "seen": now}
        return chat

    def test_three_silent_peers_finish_in_about_one_delay_not_three(self):
        chat = self.make_chat(["a", "b", "c"])

        def slow_send(peer, me, text):
            time.sleep(self.DELAY)

        with mock.patch.object(talk, "send_to", side_effect=slow_send):
            started = time.monotonic()
            chat.handle("hello")
            elapsed = time.monotonic() - started
        self.assertLess(elapsed, 2 * self.DELAY, "순차 전송이면 3 × DELAY 가 걸린다")

    def test_one_failing_peer_does_not_stop_the_others(self):
        chat = self.make_chat(["a", "b", "c"])
        delivered = []
        lock = threading.Lock()

        def send(peer, me, text):
            if peer["name"] == "b":
                raise OSError("unreachable")
            with lock:
                delivered.append(peer["name"])

        with mock.patch.object(talk, "send_to", side_effect=send):
            chat.handle("hello")
        self.assertEqual(sorted(delivered), ["a", "c"])
        self.assertEqual(self.lines.count(talk.t("peer.unreachable", name="b")), 1, self.lines)

    def test_failure_messages_follow_peer_order_and_the_record_line_is_last(self):
        chat = self.make_chat(["a", "b", "c"])

        def send(peer, me, text):
            if peer["name"] in ("a", "c"):
                raise OSError("unreachable")

        with mock.patch.object(talk, "send_to", side_effect=send):
            chat.handle("hello")
        self.assertIn("a", self.lines[0])
        self.assertIn("c", self.lines[1])
        self.assertTrue(self.lines[-1].endswith("hello"), self.lines[-1])

    def test_no_more_than_send_workers_are_sent_at_once(self):
        chat = self.make_chat([f"p{i}" for i in range(talk.SEND_WORKERS + 4)])
        active = [0]
        most = [0]
        lock = threading.Lock()

        def send(peer, me, text):
            with lock:
                active[0] += 1
                most[0] = max(most[0], active[0])
            time.sleep(0.2)
            with lock:
                active[0] -= 1

        with mock.patch.object(talk, "send_to", side_effect=send):
            chat.handle("hello")
        self.assertLessEqual(most[0], talk.SEND_WORKERS)
        self.assertGreater(most[0], 1)

    def test_no_peers_sends_nothing(self):
        chat = self.make_chat([])
        with mock.patch.object(talk, "send_to") as send:
            chat.handle("hello")
        send.assert_not_called()
        self.assertTrue(any("hint" in line.lower() or "peer" in line.lower() for line in self.lines), self.lines)


if __name__ == "__main__":
    unittest.main()
