import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))
os.environ.setdefault("HERDR_TALK_LANG", "en")

import talk  # noqa: E402


class UdpPortTest(unittest.TestCase):
    def test_unset_or_empty_gives_the_default(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(talk.udp_port(), 47850)
        self.assertEqual(talk.parse_udp_port(""), 47850)
        self.assertEqual(talk.parse_udp_port("  "), 47850)

    def test_set_value_is_used(self):
        with mock.patch.dict(os.environ, {talk.UDP_PORT_VARIABLE: "47851"}):
            self.assertEqual(talk.udp_port(), 47851)

    def test_bad_values_raise_instead_of_falling_back(self):
        for bad in ["0", "65536", "-1", "abc", "47850x", "4.5", "+47850", "٤٧٨٥٠"]:
            with self.assertRaises(ValueError) as caught:
                talk.parse_udp_port(bad)
            self.assertIn(talk.UDP_PORT_VARIABLE, str(caught.exception))

    def test_same_default_as_rust(self):
        self.assertEqual(talk.DEFAULT_UDP_PORT, 47850)


if __name__ == "__main__":
    unittest.main()
