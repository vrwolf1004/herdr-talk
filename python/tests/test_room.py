import json
import os
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from core import ed25519, noise, relay_rest, room  # noqa: E402

ROOM = "ABCD2345"
NOW = 1_700_000_000


def keys():
    seed = os.urandom(32)
    noise_public, noise_secret = noise.generate_keypair()
    return {"seed": seed, "pk": ed25519.public_key(seed).hex(), "noise_public": noise_public, "noise_secret": noise_secret}


class NonceCacheTest(unittest.TestCase):
    def test_a_nonce_is_accepted_once_inside_the_window(self):
        cache = room.NonceCache()
        self.assertTrue(cache.accept("aa", NOW, NOW))
        self.assertFalse(cache.accept("aa", NOW, NOW), "a replay is refused")

    def test_a_ts_outside_the_window_is_refused_and_not_recorded(self):
        cache = room.NonceCache()
        self.assertFalse(cache.accept("bb", NOW - room.WINDOW_SECONDS - 1, NOW))
        self.assertTrue(cache.accept("bb", NOW, NOW), "a refused message does not use up its nonce")

    def test_old_nonces_are_forgotten_after_the_window(self):
        cache = room.NonceCache()
        cache.accept("cc", NOW, NOW)
        self.assertTrue(cache.accept("cc", NOW + room.WINDOW_SECONDS + 1, NOW + room.WINDOW_SECONDS + 1))

    def test_a_full_cache_refuses_rather_than_forget_a_live_nonce(self):
        cache = room.NonceCache(capacity=2)
        self.assertTrue(cache.accept("n1", NOW, NOW))
        self.assertTrue(cache.accept("n2", NOW, NOW))
        self.assertFalse(cache.accept("n3", NOW, NOW))
        self.assertFalse(cache.accept("n1", NOW, NOW), "n1 is still live, so it cannot come back")


class SealTest(unittest.TestCase):
    def test_the_recipient_reads_the_payload_and_the_proven_sender_key(self):
        sender, recipient = keys(), keys()
        ciphertext = room.seal(sender["noise_secret"], recipient["noise_public"], b"hello")
        payload, remote = room.open_message(recipient["noise_secret"], ciphertext)
        self.assertEqual(payload, b"hello")
        self.assertEqual(remote, sender["noise_public"])

    def test_a_message_for_someone_else_does_not_open(self):
        sender, recipient, other = keys(), keys(), keys()
        ciphertext = room.seal(sender["noise_secret"], recipient["noise_public"], b"hello")
        with self.assertRaises(noise.NoiseError):
            room.open_message(other["noise_secret"], ciphertext)


class CheckRowTest(unittest.TestCase):
    def setUp(self):
        self.me, self.peer = keys(), keys()
        self.members = {self.peer["pk"]: self.peer["noise_public"]}
        self.trusted = {self.peer["noise_public"]: "Liam > x"}  # a verified name that contains a separator mark
        self.nonce = "11" * 16

    def row(self, text="hi", ts=NOW, nonce=None, sender=None):
        sender = sender or self.peer
        payload = room.payload_bytes("Liam", text, ts, nonce or self.nonce)
        ciphertext = room.seal(sender["noise_secret"], self.me["noise_public"], payload).hex()
        return {"sender_pk": self.peer["pk"], "ts": ts, "nonce": nonce or self.nonce, "ciphertext": ciphertext}

    def check(self, row, cache=None, now=NOW):
        return room.check_row(row, self.members, self.me["noise_secret"], self.trusted.get, cache or room.NonceCache(), now)

    def test_a_valid_row_shows_the_verified_name_and_the_text(self):
        self.assertEqual(self.check(self.row("hello")), ("Liam > x", "hello"))

    def test_a_replayed_row_is_shown_once(self):
        cache = room.NonceCache()
        row = self.row()
        self.assertIsNotNone(self.check(row, cache))
        self.assertIsNone(self.check(row, cache))

    def test_a_row_from_a_non_member_is_dropped(self):
        row = self.row()
        row["sender_pk"] = keys()["pk"]
        self.assertIsNone(self.check(row))

    def test_a_row_signed_with_another_noise_key_is_dropped(self):
        imposter = keys()
        self.assertIsNone(self.check(self.row(sender=imposter)))

    def test_an_untrusted_sender_is_dropped(self):
        self.trusted.clear()
        self.assertIsNone(self.check(self.row()))

    def test_a_ts_that_differs_from_the_payload_is_dropped(self):
        row = self.row()
        row["ts"] = NOW + 1
        self.assertIsNone(self.check(row, now=NOW + 1))

    def test_a_row_outside_the_window_is_dropped(self):
        self.assertIsNone(self.check(self.row(ts=NOW - room.WINDOW_SECONDS - 5), now=NOW))

    def test_a_row_sent_before_this_node_started_is_dropped(self):
        started = NOW
        before = self.row(ts=started - 1)
        self.assertIsNone(room.check_row(before, self.members, self.me["noise_secret"], self.trusted.get,
                                         room.NonceCache(), started, not_before=started))
        after = self.row(ts=started + 1)
        self.assertIsNotNone(room.check_row(after, self.members, self.me["noise_secret"], self.trusted.get,
                                            room.NonceCache(), started + 1, not_before=started))

    def test_an_empty_text_is_dropped(self):
        self.assertIsNone(self.check(self.row("   ")))


class PostAndFetchTest(unittest.TestCase):
    """A text posted to two trusted members is read back by each of them, with the sender's verified name."""

    def test_a_round_trip_through_the_post_body(self):
        sender, first, second = keys(), keys(), keys()
        config = relay_rest.Config("https://x.supabase.co", "dummy-test-key")
        posted = {}

        def fake_send(request):
            posted["body"] = json.loads(request["body"])
            return 200, ""

        with mock.patch.object(relay_rest, "_send", side_effect=fake_send):
            ok = room.post_text(config, sender["seed"], ROOM, sender["noise_secret"],
                                [(first["pk"], first["noise_public"]), (second["pk"], second["noise_public"])],
                                "Alice", "hello room", ts=NOW)
        self.assertTrue(ok)
        body = posted["body"]
        self.assertEqual([row["recipient_pk"] for row in body["deliveries"]], sorted([first["pk"], second["pk"]]))
        lines = [relay_rest.POST_TAG, "post", ROOM, sender["pk"], body["message_id"], str(NOW), body["nonce"],
                 relay_rest.deliveries_digest(body["deliveries"])]
        self.assertTrue(ed25519.verify(bytes.fromhex(sender["pk"]), "\n".join(lines).encode("utf-8"), bytes.fromhex(body["sig"])))

        for reader in (first, second):
            delivery = next(row for row in body["deliveries"] if row["recipient_pk"] == reader["pk"])
            row = {"sender_pk": sender["pk"], "ts": NOW, "nonce": body["nonce"], "ciphertext": delivery["ciphertext"]}
            shown = room.check_row(row, {sender["pk"]: sender["noise_public"]}, reader["noise_secret"],
                                   {sender["noise_public"]: "Alice"}.get, room.NonceCache(), NOW)
            self.assertEqual(shown, ("Alice", "hello room"))

    def test_recipients_must_be_known_and_bounded(self):
        sender = keys()
        with self.assertRaises(ValueError):
            room.post_text(relay_rest.Config("https://x", "k"), sender["seed"], ROOM, sender["noise_secret"], [], "A", "x", ts=NOW)


class DisplayNameTest(unittest.TestCase):
    def test_a_clash_gets_the_first_four_characters_of_the_key(self):
        names = {"aaaa" + "0" * 60: "Liam", "bbbb" + "0" * 60: "Liam"}
        self.assertEqual(room.display_name(names, "aaaa" + "0" * 60, "Liam"), "Liam (aaaa)")
        self.assertEqual(room.display_name({"cccc" + "0" * 60: "Liam"}, "cccc" + "0" * 60, "Liam"), "Liam")


class SharedVectorTest(unittest.TestCase):
    """The room values in mobile/test-vectors/relay.json (the Rust test checks the same file)."""

    def test_python_computes_the_shared_room_values(self):
        vectors = json.loads((Path(__file__).resolve().parents[2] / "mobile" / "test-vectors" / "relay.json")
                             .read_text(encoding="utf-8"))
        i, o = vectors["inputs"], vectors["outputs"]
        self.assertEqual(ed25519.public_key(bytes.fromhex(i["roomSenderSeed"])).hex(), o["roomSenderPublic"])
        self.assertEqual(noise.public_key(bytes.fromhex(i["roomSenderNoisePrivate"])).hex(), o["roomSenderNoisePublic"])
        self.assertEqual(room.payload_bytes(i["roomName"], i["roomText"], i["roomTs"], i["roomNonce"]).decode("utf-8"),
                         o["roomPayloadJson"])
        digest = relay_rest.deliveries_digest([{"recipient_pk": o["roomRecipientPublic"], "ciphertext": o["roomCiphertext"]}])
        self.assertEqual(digest, o["roomDeliveriesDigest"])
        lines = [relay_rest.POST_TAG, "post", i["roomCode"], o["roomSenderPublic"], i["roomMessageId"], str(i["roomTs"]),
                 i["roomNonce"], digest]
        self.assertEqual(chr(10).join(lines), o["roomPostSignedText"])
        self.assertTrue(ed25519.verify(bytes.fromhex(o["roomSenderPublic"]), chr(10).join(lines).encode("utf-8"),
                                       bytes.fromhex(o["roomPostSig"])))
        self.assertEqual(ed25519.sign(bytes.fromhex(i["roomSenderSeed"]), chr(10).join(lines).encode("utf-8")).hex(), o["roomPostSig"])
        rows = relay_rest.read_fetch_response(200, o["roomFetchAnswer"])
        self.assertEqual(rows[0]["ciphertext"], o["roomCiphertext"])
        cache = room.NonceCache()
        self.assertEqual(cache.accept(i["roomNonce"], i["roomTs"], i["roomTs"]), o["roomNonceFirstAccept"] == "true")
        self.assertEqual(cache.accept(i["roomNonce"], i["roomTs"], i["roomTs"]), o["roomNonceSecondAccept"] == "true")
        opened, remote = room.open_message(bytes.fromhex(i["roomRecipientNoisePrivate"]), bytes.fromhex(o["roomCiphertext"]))
        self.assertEqual(opened.decode("utf-8"), o["roomPayloadJson"])
        self.assertEqual(remote.hex(), o["roomSenderNoisePublic"])


if __name__ == "__main__":
    unittest.main()
