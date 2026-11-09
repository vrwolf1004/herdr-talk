import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))

import ed25519  # noqa: E402
import relay_key  # noqa: E402

# RFC 8032 section 7.1, TEST 1 to TEST 3 (secret, public, message, signature).
RFC8032 = [
    ("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
     "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
     "",
     "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"),
    ("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
     "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
     "72",
     "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"),
    ("c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
     "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
     "af82",
     "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a"),
]


class Ed25519Test(unittest.TestCase):
    def test_rfc_8032_vectors_for_keys_signatures_and_verify(self):
        for secret, public, message, signature in RFC8032:
            seed, pub, msg, sig = (bytes.fromhex(x) for x in (secret, public, message, signature))
            self.assertEqual(ed25519.public_key(seed), pub)
            self.assertEqual(ed25519.sign(seed, msg), sig)
            self.assertTrue(ed25519.verify(pub, msg, sig))

    def test_changed_message_or_signature_fails(self):
        secret, public, message, signature = RFC8032[1]
        pub, sig = bytes.fromhex(public), bytes.fromhex(signature)
        self.assertFalse(ed25519.verify(pub, b"\x73", sig))
        self.assertFalse(ed25519.verify(pub, bytes.fromhex(message), sig[:-1] + bytes([sig[-1] ^ 1])))
        self.assertFalse(ed25519.verify(pub, bytes.fromhex(message), sig[:32] + b"\xff" * 32))

    def test_bad_lengths_do_not_raise(self):
        self.assertFalse(ed25519.verify(b"\x00" * 31, b"", b"\x00" * 64))
        self.assertFalse(ed25519.verify(b"\x00" * 32, b"", b"\x00" * 63))
        with self.assertRaises(ValueError):
            ed25519.sign(b"\x00" * 31, b"")


class RelayKeyTest(unittest.TestCase):
    def test_created_once_then_read_back(self):
        with tempfile.TemporaryDirectory() as state:
            first = relay_key.load_or_create(state)
            self.assertEqual(len(first), 32)
            self.assertEqual(relay_key.load_or_create(state), first)
            self.assertNotEqual(first, relay_key.load_or_create(tempfile.mkdtemp()))

    def test_damaged_file_is_an_error_not_replaced(self):
        with tempfile.TemporaryDirectory() as state:
            path = os.path.join(state, relay_key.KEY_FILE)
            with open(path, "w", encoding="utf-8") as f:
                f.write("not a key\n")
            with self.assertRaises(relay_key.SigningKeyError):
                relay_key.load_or_create(state)
            with open(path, encoding="utf-8") as f:
                self.assertEqual(f.read(), "not a key\n")


if __name__ == "__main__":
    unittest.main()
