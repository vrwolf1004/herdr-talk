import json
import os
import socket
import sys
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

import relay  # noqa: E402
import relay_realtime as rt  # noqa: E402
import relay_rest  # noqa: E402
import websocket as ws  # noqa: E402

KEY_A = bytes([1]) * 32
KEY_B = bytes([2]) * 32


def _config(url):
    return relay_rest.Config(url, "dummy-test-key")


def _envelope():
    return relay.seal(KEY_A, KEY_B, b"\x00\x01\x02\xff")


class FrameTests(unittest.TestCase):
    def test_socket_url_uses_wss_for_https_and_ws_for_loopback(self):
        self.assertEqual(
            rt.socket_url(_config("https://x.supabase.co")),
            "wss://x.supabase.co/realtime/v1/websocket?apikey=dummy-test-key&vsn=1.0.0",
        )
        self.assertTrue(rt.socket_url(_config("http://127.0.0.1:9")).startswith("ws://127.0.0.1:9/"))

    def test_socket_url_refuses_keys_that_need_escaping(self):
        with self.assertRaises(ValueError) as ctx:
            rt.socket_url(relay_rest.Config("https://x.supabase.co", "a b&c"))
        self.assertNotIn("a b", str(ctx.exception))

    def test_topic_is_realtime_and_the_channel_name(self):
        self.assertEqual(rt.topic_for("abc"), "realtime:abc")

    def test_join_frame_has_the_public_channel_config(self):
        frame = rt.join_frame("realtime:abc", "1")
        self.assertEqual(frame["event"], "phx_join")
        self.assertIs(frame["payload"]["config"]["private"], False)
        self.assertEqual(frame["payload"]["config"]["broadcast"], {"ack": False, "self": False})

    def test_heartbeat_frame_is_the_protocol_message(self):
        self.assertEqual(
            json.loads(rt.encode(rt.heartbeat_frame("7"))),
            {"topic": "phoenix", "event": "heartbeat", "payload": {}, "ref": "7"},
        )

    def test_encode_then_decode_gives_the_same_frame(self):
        frame = rt.join_frame("realtime:abc", "3")
        self.assertEqual(rt.decode(rt.encode(frame)), frame)
        self.assertIsNone(rt.decode("not json"))
        self.assertIsNone(rt.decode(json.dumps({"topic": 1, "event": "x"})))

    def test_a_broadcast_gives_back_the_envelope(self):
        line = _envelope()
        frame = rt.decode(rt.encode(rt.broadcast_frame("realtime:abc", line, "2")))
        self.assertEqual(json.loads(rt.envelope_of(frame, "realtime:abc")), json.loads(line))
        self.assertIsNone(rt.envelope_of(frame, "realtime:other"))
        self.assertIsNone(rt.envelope_of(rt.heartbeat_frame("1"), "realtime:abc"))

    def test_send_rule_needs_the_verified_noise_key(self):
        peer = {"noise_pk": KEY_B.hex()}
        self.assertTrue(rt.may_send(peer, KEY_B))
        self.assertFalse(rt.may_send(peer, KEY_A))
        self.assertFalse(rt.may_send(peer, None))
        self.assertFalse(rt.may_send(None, KEY_B))

    def test_receive_rule_needs_the_verified_sender(self):
        line = relay.seal(KEY_B, KEY_A, b"x")
        self.assertEqual(rt.accept(line, KEY_A, KEY_B)["from"], KEY_B)
        with self.assertRaises(relay.Reject):
            rt.accept(relay.seal(KEY_A, KEY_B, b"x"), KEY_A, KEY_B)


def _recv_exact(conn, count):
    data = b""
    while len(data) < count:
        chunk = conn.recv(count - len(data))
        if not chunk:
            raise ws.WebSocketError("short")
        data += chunk
    return data


class LinkTests(unittest.TestCase):
    def _serve(self, script):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        port = listener.getsockname()[1]

        def run():
            conn, _ = listener.accept()
            head = b""
            while b"\r\n\r\n" not in head:
                head += conn.recv(1024)
            lines = head.decode().split("\r\n")
            key = [line.split(":", 1)[1].strip() for line in lines if line.lower().startswith("sec-websocket-key")][0]
            conn.sendall(
                (
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                    f"Sec-WebSocket-Accept: {ws.accept_key(key)}\r\n\r\n"
                ).encode()
            )
            try:
                script(conn)
            finally:
                conn.close()
                listener.close()

        thread = threading.Thread(target=run, daemon=True)
        thread.start()
        return port, thread

    def test_join_send_and_receive_over_a_local_websocket(self):
        seen = {}
        line = _envelope()

        def script(conn):
            fin, op, payload = ws.read_frame(lambda n: _recv_exact(conn, n))
            joined = rt.decode(payload.decode())
            seen["join_topic"] = joined["topic"]
            reply = {"topic": joined["topic"], "event": "phx_reply", "payload": {"status": "ok"}, "ref": joined["ref"]}
            conn.sendall(ws.encode_frame(0x1, rt.encode(reply).encode(), mask=False))
            fin, op, payload = ws.read_frame(lambda n: _recv_exact(conn, n))
            seen["sent"] = rt.decode(payload.decode())
            incoming = rt.broadcast_frame(joined["topic"], line, None)
            conn.sendall(ws.encode_frame(0x1, rt.encode(incoming).encode(), mask=False))

        port, thread = self._serve(script)
        channel = relay.channel_name(KEY_A, KEY_B)
        link = rt.Link.connect(_config(f"http://127.0.0.1:{port}"), channel)
        link.send_envelope(line)
        got = None
        for _ in range(20):
            got = link.poll_envelope()
            if got:
                break
        link.close()
        thread.join(5)
        self.assertEqual(seen["join_topic"], rt.topic_for(channel))
        self.assertEqual(seen["sent"]["payload"]["payload"], json.loads(line))
        self.assertEqual(json.loads(got), json.loads(line))

    def test_join_refused_raises(self):
        def script(conn):
            fin, op, payload = ws.read_frame(lambda n: _recv_exact(conn, n))
            joined = rt.decode(payload.decode())
            reply = {"topic": joined["topic"], "event": "phx_reply", "payload": {"status": "error"}, "ref": joined["ref"]}
            conn.sendall(ws.encode_frame(0x1, rt.encode(reply).encode(), mask=False))

        port, thread = self._serve(script)
        with self.assertRaises(ws.WebSocketError):
            rt.Link.connect(_config(f"http://127.0.0.1:{port}"), "abc")
        thread.join(5)


if __name__ == "__main__":
    unittest.main()
