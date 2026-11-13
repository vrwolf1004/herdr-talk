import os
import shutil
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import relay_key  # noqa: E402
import relay_chat  # noqa: E402
import relay_rest  # noqa: E402


class ShortIdTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-short-")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_new_short_id_is_eight_letters_or_digits(self):
        for _ in range(50):
            self.assertRegex(relay_key.new_short_id(), r"^[A-Z0-9]{8}$")

    def test_made_once_then_kept(self):
        first = relay_key.load_or_create_short_id(self.dir)
        self.assertRegex(first, r"^[A-Z0-9]{8}$")
        self.assertEqual(relay_key.load_or_create_short_id(self.dir), first)

    def test_damaged_file_is_an_error_not_replaced(self):
        with open(os.path.join(self.dir, relay_key.SHORT_FILE), "w") as handle:
            handle.write("NOT-VALID\n")
        with self.assertRaises(relay_key.SigningKeyError):
            relay_key.load_or_create_short_id(self.dir)

    def test_replace_keeps_the_new_id(self):
        relay_key.load_or_create_short_id(self.dir)
        relay_key.replace_short_id(self.dir, "ZZ99ZZ99")
        self.assertEqual(relay_key.load_or_create_short_id(self.dir), "ZZ99ZZ99")

    @mock.patch.object(relay_key, "noise_public_hex", return_value="ab" * 32)
    def test_publish_retries_with_a_new_id_on_409(self, _noise):
        seed = bytes(range(32))
        statuses = iter([409, 204])
        seen = []

        def fake_put(config, seed_arg, noise_pk, short, name):
            seen.append(short)
            return next(statuses)

        relay = relay_chat.Relay(hooks=None, state=self.dir, config=None, settings={"name": "Alice"}, session=relay_chat.Session(seed))
        with mock.patch.object(relay_rest, "put_status", side_effect=fake_put):
            self.assertTrue(relay._publish())
        self.assertEqual(len(seen), 2)
        self.assertNotEqual(seen[0], seen[1])

    @mock.patch.object(relay_key, "noise_public_hex", return_value="ab" * 32)
    def test_publish_gives_up_after_five_taken_ids(self, _noise):
        seed = bytes(range(32))
        relay = relay_chat.Relay(hooks=None, state=self.dir, config=None, settings={"name": "Alice"}, session=relay_chat.Session(seed))
        with mock.patch.object(relay_rest, "put_status", return_value=409) as put:
            self.assertFalse(relay._publish())
        self.assertEqual(put.call_count, 5)


if __name__ == "__main__":
    unittest.main()
