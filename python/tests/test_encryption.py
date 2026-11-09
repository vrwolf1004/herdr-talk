import json
import os
import shutil
import socket
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src"))
os.environ.setdefault("HERDR_TALK_LANG", "en")

import noise  # noqa: E402
import store  # noqa: E402
import talk  # noqa: E402


def make_chat(name="Me", plaintext=False):
    public, secret = noise.generate_keypair()
    return talk.Chat({"id": store.peer_id(public), "name": name, "port": 1, "key": public, "secret": secret}, plaintext=plaintext)


def announce(chat, peer, ip, **changes):
    """Feed one announcement of `peer` (a Chat, or a dict of fields) into chat the way the UDP listener does."""
    me = peer.me if isinstance(peer, talk.Chat) else peer
    info = {"app": talk.APP, "id": me["id"], "name": me["name"], "port": me["port"]}
    if me.get("key") is not None:
        info["key"] = store.to_hex(me["key"])
    info.update(changes)
    info = {k: v for k, v in info.items() if v is not None}
    packet = json.dumps(info).encode("utf-8")

    class Sock:
        def recvfrom(self, _size):
            if not Sock.sent:
                Sock.sent = True
                return packet, (ip, 1)
            talk.STOP.set()
            raise OSError("closed")

        sent = False

    talk.STOP.clear()
    talk._listen_loop(Sock(), chat)
    talk.STOP.clear()


class Base(unittest.TestCase):
    def setUp(self):
        talk.STOP.clear()
        self.lines = []
        self.saved = talk._sink
        talk._sink = lambda line, mine=False: self.lines.append(line)
        self.dir = tempfile.mkdtemp(prefix="herdr-talk-enc-")

    def tearDown(self):
        talk._sink = self.saved
        talk.STOP.clear()
        shutil.rmtree(self.dir, ignore_errors=True)


class StoreTest(Base):
    def test_identity_is_made_once_and_read_back(self):
        first = store.load_or_create_identity(self.dir)
        self.assertEqual(store.load_or_create_identity(self.dir), first)
        self.assertEqual(noise.public_key(first[1]), first[0])
        with open(os.path.join(self.dir, "identity.json"), encoding="utf-8") as f:
            self.assertEqual(set(json.load(f)), {"public", "secret"})

    def test_damaged_identity_is_an_error_not_a_new_key(self):
        with open(os.path.join(self.dir, "identity.json"), "w", encoding="utf-8") as f:
            f.write('{"public": "zz", "secret": "00"}')
        with self.assertRaises(ValueError):
            store.load_or_create_identity(self.dir)

    def test_trusted_round_trip_in_the_rust_format(self):
        peer = {"name": "Bob 밥", "key": bytes([9]) * 32, "ip": "10.0.0.2"}
        store.write_trusted(self.dir, [peer])
        with open(os.path.join(self.dir, "trusted.json"), encoding="utf-8") as f:
            self.assertEqual(json.load(f), [{"name": "Bob 밥", "key": "09" * 32, "ip": "10.0.0.2"}])
        self.assertEqual(store.read_trusted(self.dir), [peer])

    def test_ids_and_codes(self):
        key = bytes(range(32))
        self.assertEqual(store.peer_id(key), "630dcd2966c4336691125448bbb25b4ff412a49c732db2c8abc1b8581bd710dd")
        a, b = bytes([3]) * 32, bytes([4]) * 32
        code = store.fingerprint(a, b)
        self.assertEqual(code, store.fingerprint(b, a))
        self.assertRegex(code, r"^([0-9a-f]{4}-){7}[0-9a-f]{4}$")
        self.assertEqual(store.normalize_code(f" {code.upper()} "), store.normalize_code(code))

    def test_plaintext_follows_config_json(self):
        saved = os.environ.get("HERDR_PLUGIN_CONFIG_DIR")
        try:
            os.environ["HERDR_PLUGIN_CONFIG_DIR"] = self.dir
            self.assertFalse(store.plaintext_allowed())
            with open(os.path.join(self.dir, "config.json"), "w", encoding="utf-8") as f:
                f.write('{"plaintext": true}')
            self.assertTrue(store.plaintext_allowed())
        finally:
            if saved is None:
                os.environ.pop("HERDR_PLUGIN_CONFIG_DIR", None)
            else:
                os.environ["HERDR_PLUGIN_CONFIG_DIR"] = saved


class AnnouncementTest(Base):
    def test_forged_key_is_refused(self):
        me, other = make_chat(), make_chat("Eve")
        victim_id = store.peer_id(noise.generate_keypair()[0])
        announce(me, other, "10.0.0.5", id=victim_id)  # Eve's key does not hash to this id
        self.assertEqual(me.peers, {})

    def test_stripping_the_key_to_force_plain_text_is_refused(self):
        me, other = make_chat(), make_chat("Bob")
        announce(me, other, "10.0.0.5", key=None)
        self.assertEqual(me.peers, {})

    def test_a_plain_peer_is_kept_without_a_key(self):
        me = make_chat()
        announce(me, {"id": "old-python", "name": "Old", "port": 5}, "10.0.0.5")
        (peer,) = me.peers.values()
        self.assertIsNone(peer["key"])
        self.assertEqual(me.link_of(peer), "plain")

    def test_new_peer_shows_a_code_both_sides_agree_on(self):
        mine, theirs = make_chat("A"), make_chat("B")
        announce(mine, theirs, "10.0.0.5")
        announce(theirs, mine, "10.0.0.6")
        code = store.fingerprint(mine.me["key"], theirs.me["key"])
        self.assertIn(talk.t("peer.fingerprint", name="B", ip="10.0.0.5", label=store.key_label(theirs.me["key"]), code=code), self.lines)
        self.assertIn(talk.t("peer.fingerprint", name="A", ip="10.0.0.6", label=store.key_label(mine.me["key"]), code=code), self.lines)

    def test_own_key_is_taken_only_from_this_pc(self):
        me = make_chat()
        twin = {"id": me.me["id"], "name": "Pane2", "port": 2, "key": me.me["key"]}
        announce(me, twin, "203.0.113.7")
        self.assertEqual(me.peers, {})
        announce(me, twin, "127.0.0.1")
        self.assertEqual([peer["name"] for peer in me.peers.values()], ["Pane2"])
        self.assertEqual(me.link_of(next(iter(me.peers.values()))), "encrypted")

    def test_a_new_key_from_a_trusted_address_is_a_changed_key(self):
        me, old, new = make_chat(), make_chat("Bob"), make_chat("Bob")
        me.trusted.append({"name": "Bob", "key": old.me["key"], "ip": "10.0.0.5"})
        announce(me, new, "10.0.0.5")
        self.assertIn(talk.t("peer.key_changed", name="Bob", ip="10.0.0.5"), self.lines)
        self.assertFalse(me.trusts(new.me["key"]))

    def test_another_identity_beside_a_live_trusted_one_is_not_a_changed_key(self):
        me, bob, other = make_chat(), make_chat("Bob"), make_chat("Zed")
        me.trusted.append({"name": "Bob", "key": bob.me["key"], "ip": "10.0.0.5"})
        announce(me, bob, "10.0.0.5")
        announce(me, other, "10.0.0.5")
        self.assertNotIn(talk.t("peer.key_changed", name="Zed", ip="10.0.0.5"), self.lines)
        self.assertFalse(me.trusts(other.me["key"]))

    def test_a_trusted_key_keeps_the_name_the_user_verified(self):
        me, bob = make_chat(), make_chat("Bob")
        me.trusted.append({"name": "Robert", "key": bob.me["key"], "ip": "10.0.0.5"})
        announce(me, bob, "10.0.0.5")
        self.assertEqual([peer["name"] for peer in me.peers.values()], ["Robert"])

    def test_a_peer_without_a_verified_key_cannot_take_my_name(self):
        me = make_chat("Same")
        me.me["id"] = "f" * 64
        announce(me, {"id": "a-keyless-id", "name": "Same", "port": 5}, "10.0.0.5")
        self.assertEqual(me.me["name"], "Same")


class ExchangeTest(Base):
    """Two chats on loopback, real sockets: A sends, B's receive loop answers."""

    def pair(self, a_trusts_b=True, b_trusts_a=True, plaintext=False):
        self.a, self.b = make_chat("A", plaintext), make_chat("B", plaintext)
        self.servers = []
        for chat in (self.a, self.b):
            server = socket.socket()
            server.bind(("127.0.0.1", 0))
            server.listen()
            chat.me["port"] = server.getsockname()[1]
            threading.Thread(target=talk.receive_messages, args=(server, chat), daemon=True).start()
            self.servers.append(server)
        announce(self.a, self.b, "127.0.0.1")
        announce(self.b, self.a, "127.0.0.1")
        if a_trusts_b:
            self.a.trusted.append({"name": "B", "key": self.b.me["key"], "ip": "127.0.0.1"})
        if b_trusts_a:
            self.b.trusted.append({"name": "A", "key": self.a.me["key"], "ip": "127.0.0.1"})
        self.lines.clear()

    def tearDown(self):
        talk.STOP.set()
        for server in getattr(self, "servers", []):
            server.close()
        super().tearDown()

    def peer(self, chat):
        return next(iter(chat.peers.values()))

    def wait_for(self, count):
        deadline = time.time() + 5
        while len(self.lines) < count and time.time() < deadline:
            time.sleep(0.02)

    def test_verified_peers_exchange_an_encrypted_message(self):
        self.pair()
        talk.send_to(self.peer(self.a), self.a.me, "안녕 암호화")
        self.wait_for(1)
        self.assertEqual(self.lines, ["A: 안녕 암호화"])

    def test_what_goes_on_the_wire_is_not_plain(self):
        self.pair()
        seen = []
        original = socket.socket.sendall
        try:
            socket.socket.sendall = lambda sock, data, *rest: (seen.append(bytes(data)), original(sock, data, *rest))[1]
            talk.send_to(self.peer(self.a), self.a.me, "secret-text")
        finally:
            socket.socket.sendall = original
        self.wait_for(1)
        self.assertTrue(seen)
        self.assertFalse(any(b"secret-text" in chunk for chunk in seen))

    def test_an_unverified_sender_gets_no_answer_and_nothing_is_shown(self):
        self.pair(b_trusts_a=False)
        with self.assertRaises(OSError):
            talk.send_to(self.peer(self.a), self.a.me, "let me in")
        time.sleep(0.2)
        self.assertEqual(self.lines, [])

    def test_plain_text_is_dropped_unless_turned_on(self):
        self.pair()
        legacy = {"ip": "127.0.0.1", "port": self.b.me["port"], "key": None}
        talk.send_to(legacy, {"id": "old", "name": "Old"}, "plain hello")
        time.sleep(0.3)
        self.assertEqual(self.lines, [])
        self.b.plaintext = True
        talk.send_to(legacy, {"id": "old", "name": "Old"}, "plain hello")
        self.wait_for(1)
        self.assertEqual(self.lines, ["Old (plain text): plain hello"])

    def test_a_key_derived_id_cannot_pass_as_plain(self):
        self.pair(plaintext=True)
        legacy = {"ip": "127.0.0.1", "port": self.b.me["port"], "key": None}
        talk.send_to(legacy, {"id": self.a.me["id"], "name": "A"}, "I am A")  # A's id, no key
        time.sleep(0.3)
        self.assertEqual(self.lines, [])

    def test_message_id_must_match_the_handshake_key(self):
        self.pair()
        data = json.dumps({"app": talk.APP, "id": store.peer_id(b"x" * 32), "from": "A", "text": "spoof"}).encode()
        talk.deliver(data, self.a.me["key"], self.b)
        self.assertEqual(self.lines, [])

    def test_send_goes_by_link_and_says_why_it_skips(self):
        self.pair(b_trusts_a=True, a_trusts_b=False)
        self.a.handle("hello")
        self.assertEqual(self.lines, [talk.t("peer.unverified", name="B")])
        self.lines.clear()
        self.a.peers.clear()
        announce(self.a, {"id": "old", "name": "Old", "port": 9}, "127.0.0.1")
        self.lines.clear()
        self.a.handle("hello")
        self.assertEqual(self.lines, [talk.t("peer.plain_off", name="Old")])

    def test_trust_command_saves_and_then_the_message_flows(self):
        self.pair(a_trusts_b=False, b_trusts_a=False)
        self.b.state_dir = self.dir
        code = store.fingerprint(self.b.me["key"], self.a.me["key"])
        self.b.handle("/trust 0000-0000")
        self.assertIn(talk.t("trust.none", code="00000000"), self.lines)
        self.b.handle(f"/trust {code.upper()}")
        self.assertIn(talk.t("trust.done", name="A", ip="127.0.0.1"), self.lines)
        self.assertEqual(store.read_trusted(self.dir)[0]["key"], self.a.me["key"])
        self.assertEqual(self.b.link_of(self.peer(self.b)), "encrypted")
        self.lines.clear()
        self.a.trusted.append({"name": "B", "key": self.b.me["key"], "ip": "127.0.0.1"})
        talk.send_to(self.peer(self.a), self.a.me, "now it works")
        self.wait_for(1)
        self.assertEqual(self.lines, ["A: now it works"])

    def test_trust_refuses_two_different_keys_with_one_code(self):
        me = make_chat()
        code = "abcd"
        k1, k2 = bytes([1]) * 32, bytes([2]) * 32
        original = store.fingerprint
        try:
            store.fingerprint = lambda own, key: code
            for key in (k1, k2):
                me.peers[key.hex()] = {"id": store.peer_id(key), "name": "X", "ip": "10.0.0.9", "port": 1, "seen": time.time(), "key": key}
            me.handle("/trust abcd")
        finally:
            store.fingerprint = original
        self.assertIn(talk.t("trust.ambiguous", code=code), self.lines)
        self.assertEqual(me.trusted, [])


if __name__ == "__main__":
    unittest.main()
