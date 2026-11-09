import json
import os
import sys
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import ed25519  # noqa: E402
import relay_rest  # noqa: E402

# Dummy values only. Nothing here sends a request: urlopen is mocked where a send is tested.
SEED = bytes(range(32))  # Ed25519 private key for the tests; its public key is PK_OF_SEED
# v3 vector (same as mobile/test-vectors/relay.json): seed 00..1f, noise_pk ab×32, short id ab12cd34, name Alice,
# ts 1700000000, nonce 00112233445566778899aabbccddeeff. Signed with OpenSSL 3.5.4 (pkeyutl -sign -rawin).
VECTOR_NOISE = "ab" * 32
VECTOR_SHORT = "ab12cd34"
VECTOR_NONCE = "00112233445566778899aabbccddeeff"
VECTOR_SIG = "c87383be2bc3213255c8122a7e7900dab7910ea4b97aeca17e1f8adbb43b371d533340a0f1debc1d26a358ddc8f563f1bcedd7fe8714d8acd4f482777f85c202"
PK_OF_SEED = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8"
PK = "0101010101010101010101010101010101010101010101010101010101010101"
DUMMY_URL = "http://127.0.0.1:9/"
DUMMY_KEY = "dummy-test-key"


def dummy():
    return relay_rest.config(DUMMY_URL, DUMMY_KEY)


class RelayRestTest(unittest.TestCase):
    def test_settings_need_both_values_and_https_unless_loopback(self):
        self.assertIsNone(relay_rest.config(None, DUMMY_KEY))
        self.assertIsNone(relay_rest.config("https://x.example", "  "))
        self.assertEqual(relay_rest.config("https://x.example.co/", "k").url, "https://x.example.co")
        with self.assertRaises(ValueError):
            relay_rest.config("http://example.com", "k")
        self.assertIsNotNone(relay_rest.config("http://localhost:9", "k"))

    def test_repr_hides_the_key(self):
        self.assertNotIn(DUMMY_KEY, repr(dummy()))

    def test_put_request_is_a_signed_put_peer_call(self):
        request = relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice", ts=1700000000, nonce=VECTOR_NONCE)
        self.assertEqual(request["method"], "POST")
        self.assertEqual(request["url"], "http://127.0.0.1:9/functions/v1/put-peer")
        headers = request["headers"]
        self.assertEqual(headers["apikey"], DUMMY_KEY)
        self.assertEqual(headers["Authorization"], f"Bearer {DUMMY_KEY}")
        self.assertNotIn("x-herdr-pk", headers)
        expected = (
            '{"pk":"' + PK_OF_SEED + '","noise_pk":"' + VECTOR_NOISE + '","short_id":"' + VECTOR_SHORT
            + '","name":"Alice","ts":1700000000,"nonce":"' + VECTOR_NONCE + '","sig":"' + VECTOR_SIG + '"}'
        )
        self.assertEqual(request["body"], expected)

    def test_v3_message_and_signature_verify(self):
        message = relay_rest.put_message(PK_OF_SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice", 1700000000, VECTOR_NONCE)
        expected = chr(10).join(["herdr-talk-put-v3", PK_OF_SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice", "1700000000", VECTOR_NONCE]).encode("utf-8")
        self.assertEqual(message, expected)
        self.assertEqual(len(message), 206)
        self.assertEqual(ed25519.sign(SEED, message).hex(), VECTOR_SIG)
        self.assertTrue(ed25519.verify(bytes.fromhex(PK_OF_SEED), message, bytes.fromhex(VECTOR_SIG)))

    def test_put_request_fills_ts_and_nonce(self):
        body = json.loads(relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice")["body"])
        self.assertEqual(list(body), ["pk", "noise_pk", "short_id", "name", "ts", "nonce", "sig"])
        self.assertTrue(abs(body["ts"] - int(time.time())) < 60)
        self.assertRegex(body["nonce"], r"^[0-9a-f]{32}$")

    def test_get_request_selects_by_short_id(self):
        request = relay_rest.get_request(dummy(), VECTOR_SHORT)
        self.assertEqual(request["method"], "GET")
        self.assertEqual(
            request["url"],
            "http://127.0.0.1:9/rest/v1/peers?short_id=eq.ab12cd34&select=pk,name,noise_pk,short_id",
        )
        self.assertIsNone(request["body"])

    def test_bad_input_is_refused(self):
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "")
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "x" * 33)
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "a" + chr(7) + "b")
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, "AB12CD34", "Alice")
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice", ts=1, nonce="ABC")
        with self.assertRaises(ValueError):
            relay_rest.get_request(dummy(), "abc")
        with self.assertRaises(ValueError):
            relay_rest.get_request(dummy(), PK)

    def test_name_length_counts_characters(self):
        self.assertTrue(relay_rest.valid_name("가" * 32))
        self.assertFalse(relay_rest.valid_name("가" * 33))

    def test_short_id_format(self):
        self.assertTrue(relay_rest.valid_short("ab12cd34"))
        for bad in ("ab12cd3", "ab12cd345", "AB12CD34", "ab12-d34", "", None):
            self.assertFalse(relay_rest.valid_short(bad), bad)

    def test_answers_are_checked(self):
        noise = "ab" * 32
        good = f'[{{"pk":"{PK}","name":"Alice","noise_pk":"{noise}","short_id":"{VECTOR_SHORT}"}}]'
        self.assertEqual(
            relay_rest.read_get_response(200, good, VECTOR_SHORT),
            {"pk": PK, "name": "Alice", "noise_pk": noise, "short_id": VECTOR_SHORT},
        )
        self.assertIsNone(relay_rest.read_get_response(200, "[]", VECTOR_SHORT))
        row = f'{{"pk":"{PK}","name":"A","noise_pk":"{noise}","short_id":"{VECTOR_SHORT}"}}'
        for status, body in [
            (401, good),
            (200, "{}"),
            (200, "not json"),
            (200, f'[{{"pk":"{PK}","name":"Alice","noise_pk":"{noise}","short_id":"zzzzzzzz"}}]'),
            (200, f'[{{"pk":"{PK}","name":"","noise_pk":"{noise}","short_id":"{VECTOR_SHORT}"}}]'),
            (200, f'[{{"pk":"{PK}","name":"A","short_id":"{VECTOR_SHORT}"}}]'),
            (200, f'[{{"pk":"{PK}","name":"A","noise_pk":"XYZ","short_id":"{VECTOR_SHORT}"}}]'),
            (200, f"[{row},{row}]"),
        ]:
            with self.assertRaises(ValueError, msg=body):
                relay_rest.read_get_response(status, body, VECTOR_SHORT)

    def test_write_status_must_be_2xx(self):
        self.assertTrue(relay_rest.put_succeeded(201))
        self.assertTrue(relay_rest.put_succeeded(204))
        self.assertFalse(relay_rest.put_succeeded(401))
        self.assertFalse(relay_rest.put_succeeded(409))

    def test_a_taken_short_id_is_reported_as_409(self):
        with mock.patch.object(relay_rest, "_send", return_value=(409, "")):
            self.assertEqual(relay_rest.put_status(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice"), 409)
            self.assertFalse(relay_rest.put_peer(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice"))

    def test_put_peer_sends_the_built_request(self):
        with mock.patch.object(relay_rest, "_send", return_value=(204, "")) as send:
            self.assertTrue(relay_rest.put_peer(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice"))
        sent = send.call_args[0][0]
        self.assertEqual(sent["method"], "POST")

    def test_get_peer_returns_the_checked_row(self):
        body = f'[{{"pk":"{PK}","name":"Alice","noise_pk":"{"ab" * 32}","short_id":"{VECTOR_SHORT}"}}]'
        with mock.patch.object(relay_rest, "_send", return_value=(200, body)):
            self.assertEqual(relay_rest.get_peer(dummy(), VECTOR_SHORT)["pk"], PK)
        with mock.patch.object(relay_rest, "_send", side_effect=OSError("down")):
            self.assertIsNone(relay_rest.get_peer(dummy(), VECTOR_SHORT))


if __name__ == "__main__":
    unittest.main()
