import json
import os
import sys
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

from core import ed25519  # noqa: E402
from core import relay_rest  # noqa: E402

# Dummy values only. Nothing here sends a request: urlopen is mocked where a send is tested.
SEED = bytes(range(32))  # Ed25519 private key for the tests; its public key is PK_OF_SEED
# v3 vector (same as mobile/test-vectors/relay.json): seed 00..1f, noise_pk ab×32, short id AB12CD34, name Alice,
# ts 1700000000, nonce 00112233445566778899aabbccddeeff. Signed with OpenSSL 3.5.4 (pkeyutl -sign -rawin).
VECTOR_NOISE = "ab" * 32
VECTOR_SHORT = "AB12CD34"
VECTOR_NONCE = "00112233445566778899aabbccddeeff"
VECTOR_SIG = "5a5d9f0ee7490ec09cf1eb71643d08c89e840a7b0a4209d32d328a38098a43a0c2c31ddb456d5ff719d599c63162511acd51bb8c558cd065d212bf0502d77301"
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

    def test_name_length_counts_characters(self):
        self.assertTrue(relay_rest.valid_name("가" * 32))
        self.assertFalse(relay_rest.valid_name("가" * 33))

    def test_name_refuses_control_format_and_space_separator_characters(self):
        # The same rule as talk-core's is_printable: bidi and zero-width format characters, tabs and other separators are out.
        for bad in ["a‮b", "a​b", "a\tb", "a b", "ab"]:
            self.assertFalse(relay_rest.valid_name(bad), repr(bad))
        self.assertTrue(relay_rest.valid_name("a b"), "a plain space is kept")

    def test_short_id_format(self):
        self.assertTrue(relay_rest.valid_short("AB12CD34"))
        for bad in ("AB12CD3", "AB12CD345", "ab12cd34", "AB12-D34", "", None):
            self.assertFalse(relay_rest.valid_short(bad), bad)

    def test_write_status_must_be_2xx(self):
        self.assertTrue(relay_rest.put_succeeded(201))
        self.assertTrue(relay_rest.put_succeeded(204))
        self.assertFalse(relay_rest.put_succeeded(401))
        self.assertFalse(relay_rest.put_succeeded(409))

    def test_a_taken_short_id_is_reported_as_409(self):
        with mock.patch.object(relay_rest, "_send", return_value=(409, "")):
            self.assertEqual(relay_rest.put_status(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice"), 409)

    def test_put_status_sends_the_built_request(self):
        with mock.patch.object(relay_rest, "_send", return_value=(204, "")) as send:
            self.assertEqual(relay_rest.put_status(dummy(), SEED, VECTOR_NOISE, VECTOR_SHORT, "Alice"), 204)
        sent = send.call_args[0][0]
        self.assertEqual(sent["method"], "POST")



SEED = bytes(range(32))
PK_OF_SEED = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8"
VECTOR_NONCE = "00112233445566778899aabbccddeeff"
ROOM_CREATE_SIG = "f26ec1d02def0df18204cf6507e74e121432cf29ff999b89f30b3137f6084fd2e10b3b6a5d1982a13a531547bbb23370fe821a91a27af0b616b3a05c9b718b0c"
ROOM_JOIN_SIG = "4128444185a03e137deca1e066f73d7bf2121162a1f620c01b75f0e217511189388e4da58a7774bb5b18f0e2263a77ff65136eeab8b6a765ac53ca8a15f33c08"


class RoomTests(unittest.TestCase):
    """create-room and join-room: request shape and answer checks. No network: the sender is replaced."""

    PK = "01" * 32

    def test_room_codes_are_eight_uppercase_letters_or_digits(self):
        self.assertTrue(relay_rest.valid_room_code("ABCD1234"))
        for bad in ("abcd1234", "ABCD123", "ABCD12345", "ABCD-234", None, 12345678):
            self.assertFalse(relay_rest.valid_room_code(bad), bad)

    def test_create_room_request_sends_the_key_and_a_signed_body(self):
        request = relay_rest.create_room_request(dummy(), SEED, "Alice", "12345678", ts=1700000000, nonce=VECTOR_NONCE)
        self.assertEqual(request["url"], dummy().url + "/functions/v1/create-room")
        self.assertEqual(request["headers"]["apikey"], DUMMY_KEY)
        self.assertEqual(request["headers"]["Authorization"], "Bearer " + DUMMY_KEY)
        self.assertEqual(json.loads(request["body"]), {"peer_pk": PK_OF_SEED, "display_name": "Alice", "ts": 1700000000,
                                                        "nonce": VECTOR_NONCE, "sig": ROOM_CREATE_SIG, "trust": "12345678"})

    def test_the_trust_code_is_eight_digits_and_is_refused_before_a_request_exists(self):
        self.assertTrue(relay_rest.valid_trust("12345678"))
        for bad in ("1234567", "123456789", "1234567a", "１２３４５６７８", "", None):
            self.assertFalse(relay_rest.valid_trust(bad), bad)
            with self.assertRaises(ValueError):
                relay_rest.join_room_request(dummy(), SEED, "ABCD1234", "Bob", bad)
            with self.assertRaises(ValueError):
                relay_rest.create_room_request(dummy(), SEED, "Alice", bad)

    def test_room_signature_matches_the_fixed_vector_and_verifies(self):
        message = relay_rest.room_message("create", "", PK_OF_SEED, "Alice", 1700000000, VECTOR_NONCE)
        self.assertEqual(message.decode("utf-8").split("\n"), ["herdr-talk-room-v1", "create", "", PK_OF_SEED, "Alice", "1700000000", VECTOR_NONCE])
        self.assertEqual(ed25519.sign(SEED, message).hex(), ROOM_CREATE_SIG)
        self.assertTrue(ed25519.verify(bytes.fromhex(PK_OF_SEED), message, bytes.fromhex(ROOM_CREATE_SIG)))
        join = relay_rest.room_message("join", "ABCD1234", PK_OF_SEED, "Bob", 1700000000, VECTOR_NONCE)
        self.assertEqual(ed25519.sign(SEED, join).hex(), ROOM_JOIN_SIG)
        self.assertNotEqual(ROOM_CREATE_SIG, ROOM_JOIN_SIG, "the action and the room code are signed")

    def test_bad_input_is_refused_before_a_request_exists(self):
        with self.assertRaises(ValueError):
            relay_rest.create_room_request(dummy(), SEED, "", "12345678")
        with self.assertRaises(ValueError):
            relay_rest.create_room_request(dummy(), SEED, "Alice", "12345678", ts=True, nonce=VECTOR_NONCE)
        with self.assertRaises(ValueError):
            relay_rest.join_room_request(dummy(), SEED, "abcd1234", "Alice", "12345678")

    def test_create_room_answer_is_checked(self):
        self.assertEqual(relay_rest.read_room_code(200, '{"room_code":"ABCD1234","message":"ok"}'), "ABCD1234")
        for status, body in ((500, '{"error":"x"}'), (200, "not json"), (200, '{"room_code":"bad"}'), (200, "[]")):
            with self.assertRaises(ValueError, msg=f"{status} {body}"):
                relay_rest.read_room_code(status, body)

    def test_the_operator_issue_answers_are_read(self):
        self.assertEqual(relay_rest.read_issue_answer(200, '{"code": "T5N9Q1ZB", "max_uses": 20}'), "T5N9Q1ZB")
        for status, text in ((401, "wrong"), (429, "later"), (503, "secrets")):
            with self.subTest(status=status):
                with self.assertRaises(ValueError):
                    relay_rest.read_issue_answer(status, "{}")
        with self.assertRaises(ValueError):
            relay_rest.read_issue_answer(200, "not json")

    def test_a_wrong_trust_code_is_refused_and_says_so(self):
        with self.assertRaises(ValueError) as caught:
            relay_rest.read_join_room(403, "")
        self.assertIn("trust", str(caught.exception))

    def test_a_full_room_says_so(self):
        with self.assertRaises(ValueError) as caught:
            relay_rest.read_join_room(409, "{}")
        self.assertIn("10", str(caught.exception))

    def test_join_room_request_and_answer(self):
        request = relay_rest.join_room_request(dummy(), SEED, "ABCD1234", "Bob", "12345678", ts=1700000000, nonce=VECTOR_NONCE)
        self.assertEqual(request["url"], dummy().url + "/functions/v1/join-room")
        self.assertEqual(json.loads(request["body"]), {"room_code": "ABCD1234", "peer_pk": PK_OF_SEED, "display_name": "Bob",
                                                        "ts": 1700000000, "nonce": VECTOR_NONCE, "sig": ROOM_JOIN_SIG, "trust": "12345678"})
        info = relay_rest.read_join_room(200, json.dumps({"room_code": "ABCD1234", "owner_pk": self.PK, "members": [self.PK]}))
        self.assertEqual(info, {"room_code": "ABCD1234", "owner_pk": self.PK, "members": [self.PK]})
        with self.assertRaises(ValueError):
            relay_rest.read_join_room(200, json.dumps({"room_code": "ABCD1234", "owner_pk": self.PK, "members": ["zz"]}))
        with self.assertRaises(ValueError):
            relay_rest.read_join_room(404, "{}")
        with self.assertRaisesRegex(ValueError, "방 기한 만료"):
            relay_rest.read_join_room(410, '{"error":"방 기한이 지났습니다"}')

    def test_create_and_join_send_one_request_each(self):
        with mock.patch.object(relay_rest, "_send", return_value=(200, '{"room_code":"ABCD1234"}')) as send:
            self.assertEqual(relay_rest.create_room(dummy(), SEED, "Alice", "12345678"), "ABCD1234")
        self.assertIn("create-room", send.call_args.args[0]["url"])
        with mock.patch.object(relay_rest, "_send", side_effect=OSError("offline")):
            with self.assertRaises(OSError):
                relay_rest.join_room(dummy(), SEED, "ABCD1234", "Bob", "12345678")


class SetupCodeTests(unittest.TestCase):
    ANSWER = '{"url": "https://new.supabase.co", "key": "new-dummy-key"}'

    def test_normalize_drops_spaces_and_dashes_and_uppercases(self):
        self.assertEqual(relay_rest.normalize_setup_code(" abcd-2345 "), "ABCD2345")
        self.assertEqual(relay_rest.normalize_setup_code("ab cd 23 45"), "ABCD2345")
        self.assertEqual(relay_rest.normalize_setup_code("AB12CD34"), "AB12CD34")

    def test_normalize_refuses_wrong_length_symbols_and_non_ascii(self):
        for text in ("ABC", "ABCD23456", "ABCD234!", "ABCD_234", "ABCD234É", "ＡＢＣＤ２３４５", "", None):
            with self.subTest(text=text):
                self.assertIsNone(relay_rest.normalize_setup_code(text))

    def test_setup_url_env_wins_else_default(self):
        with mock.patch.dict(os.environ, {relay_rest.URL_VARIABLE: " http://127.0.0.1:9/ "}):
            self.assertEqual(relay_rest.setup_url(), "http://127.0.0.1:9/")
        with mock.patch.dict(os.environ, {relay_rest.URL_VARIABLE: ""}):
            self.assertEqual(relay_rest.setup_url(), relay_rest.DEFAULT_URL)

    def test_redeem_posts_the_normalized_code_without_an_apikey(self):
        with mock.patch.object(relay_rest, "_send", return_value=(200, self.ANSWER)) as send:
            url, key = relay_rest.redeem_setup_code("abcd-2345", "https://base.supabase.co/")
        self.assertEqual((url, key), ("https://new.supabase.co", "new-dummy-key"))
        request = send.call_args.args[0]
        self.assertEqual(request["method"], "POST")
        self.assertEqual(request["url"], "https://base.supabase.co/functions/v1/redeem-setup-code")
        self.assertEqual(request["headers"], {"Content-Type": "application/json"})
        self.assertEqual(json.loads(request["body"]), {"code": "ABCD2345"})

    def test_redeem_uses_the_env_url_or_the_default(self):
        with mock.patch.object(relay_rest, "_send", return_value=(200, self.ANSWER)) as send, \
                mock.patch.dict(os.environ, {relay_rest.URL_VARIABLE: ""}):
            relay_rest.redeem_setup_code("ABCD2345")
        self.assertEqual(send.call_args.args[0]["url"], relay_rest.DEFAULT_URL + "/functions/v1/redeem-setup-code")

    def test_redeem_answers(self):
        cases = [(404, "", "wrong, used or expired"), (429, "", "too many wrong codes"), (500, "", "status 500"),
                 (200, "not json", "not JSON"), (200, '{"url": "https://x.supabase.co"}', "no url and key"),
                 (200, '{"url": "http://example.com", "key": "k"}', "https")]
        for status, body, reason in cases:
            with self.subTest(status=status, body=body):
                with mock.patch.object(relay_rest, "_send", return_value=(status, body)):
                    with self.assertRaisesRegex(ValueError, reason):
                        relay_rest.redeem_setup_code("ABCD2345")

    def test_a_bad_code_is_refused_before_any_request(self):
        with mock.patch.object(relay_rest, "_send") as send:
            with self.assertRaisesRegex(ValueError, "8 letters or digits"):
                relay_rest.redeem_setup_code("ABC")
        send.assert_not_called()

    def test_a_network_error_is_raised_for_the_caller(self):
        with mock.patch.object(relay_rest, "_send", side_effect=OSError("offline")):
            with self.assertRaises(OSError):
                relay_rest.redeem_setup_code("ABCD2345")


if __name__ == "__main__":
    unittest.main()
