"""The pure-Python crypto against public test vectors: RFC 7748 (X25519), RFC 8439 (ChaCha20-Poly1305), and the
Cacophony vector for Noise_IK_25519_ChaChaPoly_BLAKE2s (the pattern the Rust edition runs through snow)."""

import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "src"))

import noise  # noqa: E402

h = bytes.fromhex


class X25519Test(unittest.TestCase):
    def test_rfc7748_section_5_2(self):
        self.assertEqual(
            noise.x25519(h("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"),
                         h("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")).hex(),
            "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
        self.assertEqual(
            noise.x25519(h("4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d"),
                         h("e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493")).hex(),
            "95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957")

    def test_rfc7748_one_iteration(self):
        nine = b"\x09" + b"\x00" * 31
        self.assertEqual(noise.x25519(nine, nine).hex(), "422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079")

    def test_rfc7748_section_6_1_diffie_hellman(self):
        alice = h("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
        bob = h("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb")
        alice_public, bob_public = noise.public_key(alice), noise.public_key(bob)
        self.assertEqual(alice_public.hex(), "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        self.assertEqual(bob_public.hex(), "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
        shared = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"
        self.assertEqual(noise.x25519(alice, bob_public).hex(), shared)
        self.assertEqual(noise.x25519(bob, alice_public).hex(), shared)

    def test_a_low_order_point_is_refused(self):
        with self.assertRaises(noise.NoiseError):
            noise._dh(os.urandom(32), b"\x00" * 32)


class ChaChaPolyTest(unittest.TestCase):
    def test_rfc8439_chacha20_block(self):
        block = noise._chacha_block(bytes(range(32)), 1, h("000000090000004a00000000"))
        self.assertEqual(block[:16].hex(), "10f1e7e4d13b5915500fdd1fa32071c4")
        self.assertEqual(block[-16:].hex(), "b5129cd1de164eb9cbd083e8a2503c4e")

    def test_rfc8439_poly1305(self):
        key = h("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b")
        self.assertEqual(noise.poly1305(key, b"Cryptographic Forum Research Group").hex(), "a8061dc1305136c6c22b8baf0c0127a9")

    def test_rfc8439_aead(self):
        plaintext = (b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, "
                     b"sunscreen would be it.")
        key, nonce = bytes(range(0x80, 0xA0)), h("070000004041424344454647")
        aad = h("50515253c0c1c2c3c4c5c6c7")
        sealed = noise.aead_encrypt(key, nonce, aad, plaintext)
        self.assertEqual(sealed[:32].hex(), "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d6")
        self.assertEqual(sealed[-16:].hex(), "1ae10b594f09e26a7e902ecbd0600691")
        self.assertEqual(noise.aead_decrypt(key, nonce, aad, sealed), plaintext)

    def test_any_change_breaks_the_tag(self):
        key, nonce = os.urandom(32), os.urandom(12)
        sealed = noise.aead_encrypt(key, nonce, b"ad", b"hello")
        for index in range(len(sealed)):
            bad = bytearray(sealed)
            bad[index] ^= 1
            with self.assertRaises(noise.NoiseError):
                noise.aead_decrypt(key, nonce, b"ad", bytes(bad))
        with self.assertRaises(noise.NoiseError):
            noise.aead_decrypt(key, nonce, b"other", sealed)


class NoiseVectorTest(unittest.TestCase):
    def test_cacophony_ik_vector(self):
        with open(os.path.join(HERE, "noise_ik_vector.json"), encoding="utf-8") as f:
            v = json.load(f)
        self.assertEqual(v["protocol_name"], noise.PROTOCOL.decode())
        m = v["messages"]
        initiator = noise.Initiator(h(v["init_static"]), h(v["init_remote_static"]), h(v["init_prologue"]), h(v["init_ephemeral"]))
        responder = noise.Responder(h(v["resp_static"]), h(v["resp_prologue"]), h(v["resp_ephemeral"]))
        first = initiator.write_message1(h(m[0]["payload"]))
        self.assertEqual(first.hex(), m[0]["ciphertext"])
        self.assertEqual(responder.read_message1(first), h(m[0]["payload"]))
        self.assertEqual(responder.remote_static, noise.public_key(h(v["init_static"])))
        second = responder.write_message2(h(m[1]["payload"]))
        self.assertEqual(second.hex(), m[1]["ciphertext"])
        self.assertEqual(initiator.read_message2(second), h(m[1]["payload"]))
        self.assertEqual(initiator.handshake_hash.hex(), v["handshake_hash"])
        self.assertEqual(responder.handshake_hash.hex(), v["handshake_hash"])
        # Transport messages alternate: initiator, responder, initiator ...
        for index, message in enumerate(m[2:]):
            sender, receiver = (initiator, responder) if index % 2 == 0 else (responder, initiator)
            sealed = sender.send.encrypt(h(message["payload"]))
            self.assertEqual(sealed.hex(), message["ciphertext"])
            self.assertEqual(receiver.recv.decrypt(sealed), h(message["payload"]))

    def test_random_handshake_and_wrong_key(self):
        a_public, a_secret = noise.generate_keypair()
        b_public, b_secret = noise.generate_keypair()
        initiator, responder = noise.Initiator(a_secret, b_public), noise.Responder(b_secret)
        responder.read_message1(initiator.write_message1())
        self.assertEqual(responder.remote_static, a_public)
        initiator.read_message2(responder.write_message2())
        self.assertEqual(responder.recv.decrypt(initiator.send.encrypt("안녕".encode())).decode(), "안녕")
        # A message meant for another key does not open.
        _, c_secret = noise.generate_keypair()
        with self.assertRaises(noise.NoiseError):
            noise.Responder(c_secret).read_message1(noise.Initiator(a_secret, b_public).write_message1())


if __name__ == "__main__":
    unittest.main()
