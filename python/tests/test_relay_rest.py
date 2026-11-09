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
# v2 vector: seed 00..1f, noise_pk ab×32, name Alice, ts 1700000000, nonce 00112233445566778899aabbccddeeff.
# The signature was made with OpenSSL 3.5.4 (pkeyutl -sign -rawin) over the message bytes below.
VECTOR_NOISE = "ab" * 32
VECTOR_NONCE = "00112233445566778899aabbccddeeff"
VECTOR_SIG = "afc3815f65780c85d388896dea433086219bd41938750cf27af23b77e79a01020b458f9fc33207b7cfc472f3fdb4e8eb235cd70269255854569559c3a70d4c08"
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
        request = relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, "Alice", ts=1700000000, nonce=VECTOR_NONCE)
        self.assertEqual(request["method"], "POST")
        self.assertEqual(request["url"], "http://127.0.0.1:9/functions/v1/put-peer")
        headers = request["headers"]
        self.assertEqual(headers["apikey"], DUMMY_KEY)
        self.assertEqual(headers["Authorization"], f"Bearer {DUMMY_KEY}")
        self.assertNotIn("x-herdr-pk", headers)
        self.assertEqual(request["body"], '{"pk":"' + PK_OF_SEED + '","noise_pk":"' + VECTOR_NOISE + '","name":"Alice","ts":1700000000,"nonce":"' + VECTOR_NONCE + '","sig":"' + VECTOR_SIG + '"}')

    def test_v2_message_and_signature_verify(self):
        message = relay_rest.put_message(PK_OF_SEED, VECTOR_NOISE, "Alice", 1700000000, VECTOR_NONCE)
        expected = chr(10).join(["herdr-talk-put-v2", PK_OF_SEED, VECTOR_NOISE, "Alice", "1700000000", VECTOR_NONCE]).encode("utf-8")
        self.assertEqual(message, expected)
        self.assertEqual(ed25519.sign(SEED, message).hex(), VECTOR_SIG)
        self.assertTrue(ed25519.verify(bytes.fromhex(PK_OF_SEED), message, bytes.fromhex(VECTOR_SIG)))

    def test_put_request_fills_ts_and_nonce(self):
        body = json.loads(relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, "Alice")["body"])
        self.assertEqual(list(body), ["pk", "noise_pk", "name", "ts", "nonce", "sig"])
        self.assertTrue(abs(body["ts"] - int(time.time())) < 60)
        self.assertRegex(body["nonce"], r"^[0-9a-f]{32}$")

    def test_get_request_selects_pk_and_name(self):
        request = relay_rest.get_request(dummy(), PK)
        self.assertEqual(request["method"], "GET")
        self.assertEqual(request["url"], f"http://127.0.0.1:9/rest/v1/peers?pk=eq.{PK}&select=pk,name")
        self.assertIsNone(request["body"])

    def test_bad_input_is_refused(self):
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, "")
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, "x" * 33)
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, "a"+chr(7)+"b")
        with self.assertRaises(ValueError):
            relay_rest.put_request(dummy(), SEED, VECTOR_NOISE, "Alice", ts=1, nonce="ABC")
        with self.assertRaises(ValueError):
            relay_rest.get_request(dummy(), "abc")

    def test_name_length_counts_characters(self):
        self.assertTrue(relay_rest.valid_name("가" * 32))
        self.assertFalse(relay_rest.valid_name("가" * 33))

    def test_answers_are_checked(self):
        good = f'[{{"pk":"{PK}","name":"Alice"}}]'
        self.assertEqual(relay_rest.read_get_response(200, good, PK), {"pk": PK, "name": "Alice"})
        self.assertIsNone(relay_rest.read_get_response(200, "[]", PK))
        for status, body, pk in [
            (401, good, PK),
            (200, "{}", PK),
            (200, "not json", PK),
            (200, good, "2" * 64),
            (200, f'[{{"pk":"{PK}","name":""}}]', PK),
            (200, f'[{{"pk":"{PK}","name":"A"}},{{"pk":"{PK}","name":"A"}}]', PK),
        ]:
            with self.assertRaises(ValueError, msg=body):
                relay_rest.read_get_response(status, body, pk)

    def test_write_status_must_be_2xx(self):
        self.assertTrue(relay_rest.put_succeeded(201))
        self.assertTrue(relay_rest.put_succeeded(204))
        self.assertFalse(relay_rest.put_succeeded(401))

    def test_put_peer_sends_the_built_request(self):
        class Response:
            status = 201

            def read(self):
                return b""

            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

        with mock.patch.object(relay_rest.urllib.request, "urlopen", return_value=Response()) as opened:
            self.assertTrue(relay_rest.put_peer(dummy(), SEED, VECTOR_NOISE, "Alice"))
        sent = opened.call_args[0][0]
        self.assertEqual(sent.get_method(), "POST")
        self.assertIsNone(sent.get_header("X-herdr-pk"))

    def test_network_errors_give_no_record(self):
        with mock.patch.object(relay_rest.urllib.request, "urlopen", side_effect=OSError("down")):
            self.assertIsNone(relay_rest.get_peer(dummy(), PK))
            self.assertFalse(relay_rest.put_peer(dummy(), SEED, VECTOR_NOISE, "Alice"))


if __name__ == "__main__":
    unittest.main()
