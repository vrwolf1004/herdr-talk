import os
import sys
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

import relay_chat as rc  # noqa: E402
import store  # noqa: E402
import talk  # noqa: E402
from i18n import t  # noqa: E402

OWN = bytes([1]) * 32
PEER = bytes([7]) * 32
OTHER = bytes([9]) * 32


class RelayTrustTests(unittest.TestCase):
    """/trust also verifies the configured relay peer by its Noise key (Python edition, same rule as Rust chat.rs)."""

    def setUp(self):
        self.saved_sink = talk._sink
        self.lines = []
        talk._sink = lambda line, mine=False: self.lines.append(line)
        self.chat = talk.Chat({"id": "me", "name": "Me", "port": 1, "key": OWN, "secret": bytes([2]) * 32})
        self.chat.supabase = True

    def tearDown(self):
        talk._sink = self.saved_sink

    def code_for(self, key):
        return store.normalize_code(store.fingerprint(OWN, key))

    def test_relay_peer_with_the_matching_code_is_trusted(self):
        self.chat.relay_candidate = ("Bob", PEER)
        self.chat.trust(self.code_for(PEER))
        self.assertTrue(self.chat.trusts(PEER))
        self.assertIn(t("trust.done", name="Bob", ip="relay"), self.lines)

    def test_relay_peer_with_another_code_is_refused(self):
        self.chat.relay_candidate = ("Bob", PEER)
        self.chat.trust(self.code_for(OTHER))
        self.assertFalse(self.chat.trusts(PEER))
        self.assertEqual(self.lines[-1], t("trust.none", code=self.code_for(OTHER)))

    def test_lan_and_relay_with_the_same_code_are_ambiguous(self):
        self.chat.peers["id1"] = {"name": "Bob", "ip": "10.0.0.2", "port": 52311, "seen": time.time(), "key": PEER}
        self.chat.relay_candidate = ("Bob", PEER)
        self.chat.trust(self.code_for(PEER))
        self.assertFalse(self.chat.trusts(PEER))
        self.assertEqual(self.lines[-1], t("trust.ambiguous", code=self.code_for(PEER)))

    def test_relay_candidate_is_ignored_in_local_mode(self):
        self.chat.supabase = False
        self.chat.relay_candidate = ("Bob", PEER)
        self.chat.trust(self.code_for(PEER))
        self.assertFalse(self.chat.trusts(PEER))
        self.assertEqual(self.lines[-1], t("trust.none", code=self.code_for(PEER)))

    def test_trust_wakes_a_waiting_relay(self):
        relay = rc.Relay(talk.RelayHooks(self.chat), None, None, {}, None)
        self.chat.relay = relay
        self.chat.relay_candidate = ("Bob", PEER)
        threading.Timer(0.05, self.chat.trust, args=(self.code_for(PEER),)).start()
        started = time.monotonic()
        relay._wait(5)
        self.assertLess(time.monotonic() - started, 2)
        self.assertTrue(self.chat.trusts(PEER))

    def test_stop_relay_forgets_the_candidate(self):
        self.chat.relay_candidate = ("Bob", PEER)
        self.chat.stop_relay()
        self.assertIsNone(self.chat.relay_candidate)

    def test_candidate_comes_from_a_row_with_a_32_byte_key_only(self):
        self.assertEqual(rc.relay_candidate_of({"name": "Bob", "noise_pk": PEER.hex()}), ("Bob", PEER))
        self.assertIsNone(rc.relay_candidate_of({"name": "Bob", "noise_pk": "ab"}))
        self.assertIsNone(rc.relay_candidate_of({"name": "Bob"}))


if __name__ == "__main__":
    unittest.main()
