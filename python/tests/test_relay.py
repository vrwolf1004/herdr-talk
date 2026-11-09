import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import relay  # noqa: E402

# Same vector as rust/crates/talk-core/src/relay.rs. The channel was computed outside this code with
# `sha256sum` over the bytes "herdr-talk-relay-v1" || 0x01×32 || 0x02×32.
KEY_A = bytes([1]) * 32
KEY_B = bytes([2]) * 32
CHANNEL = "c3492c89862240d31414b19d37da93d42c204f15687632dd436238f5701eb83e"
CT = bytes([0, 1, 2, 255])
ENVELOPE = (
    '{"v":1,"channel":"c3492c89862240d31414b19d37da93d42c204f15687632dd436238f5701eb83e",'
    '"from":"0101010101010101010101010101010101010101010101010101010101010101",'
    '"to":"0202020202020202020202020202020202020202020202020202020202020202","ct":"AAEC/w=="}'
)


class RelayTest(unittest.TestCase):
    def test_channel_name_matches_the_external_vector_in_either_order(self):
        self.assertEqual(relay.channel_name(KEY_A, KEY_B), CHANNEL)
        self.assertEqual(relay.channel_name(KEY_B, KEY_A), CHANNEL)

    def test_seal_gives_the_vector_line(self):
        self.assertEqual(relay.seal(KEY_A, KEY_B, CT), ENVELOPE)

    def test_open_reads_the_vector_line_from_either_side(self):
        expected = {"from": KEY_A, "to": KEY_B, "ct": CT}
        self.assertEqual(relay.open_envelope(ENVELOPE, (KEY_A, KEY_B)), expected)
        self.assertEqual(relay.open_envelope(ENVELOPE, (KEY_B, KEY_A)), expected)

    def test_version_two_is_rejected(self):
        line = ENVELOPE.replace('"v":1', '"v":2', 1)
        with self.assertRaises(relay.Reject) as caught:
            relay.open_envelope(line, (KEY_A, KEY_B))
        self.assertEqual(caught.exception.reason, "version")

    def test_a_sender_outside_the_pair_is_rejected(self):
        line = ENVELOPE.replace(KEY_A.hex(), bytes([3]).hex() * 32, 1)
        with self.assertRaises(relay.Reject) as caught:
            relay.open_envelope(line, (KEY_A, KEY_B))
        self.assertEqual(caught.exception.reason, "party")

    def test_a_channel_that_does_not_match_is_rejected(self):
        line = ENVELOPE.replace(CHANNEL, "0" * 64, 1)
        with self.assertRaises(relay.Reject) as caught:
            relay.open_envelope(line, (KEY_A, KEY_B))
        self.assertEqual(caught.exception.reason, "channel")

    def test_bad_shapes_are_format_rejects(self):
        key = bytes([0xAB]) * 32
        line = relay.seal(key, KEY_B, CT)
        for bad in ["not json", "[]", line.replace(key.hex(), key.hex().upper(), 1)]:
            with self.assertRaises(relay.Reject) as caught:
                relay.open_envelope(bad, (key, KEY_B))
            self.assertEqual(caught.exception.reason, "format", bad)

    def test_key_of_wrong_length_is_not_sealed(self):
        self.assertIsNone(relay.seal(b"\x01" * 31, KEY_B, CT))


if __name__ == "__main__":
    unittest.main()
