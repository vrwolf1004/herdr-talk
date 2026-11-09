import os
import socket
import struct
import sys
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

import websocket as ws  # noqa: E402


def _reader(data):
    stream = [data]
    state = {"buf": data}

    def read(count):
        if len(state["buf"]) < count:
            raise ws.WebSocketError("short")
        out, state["buf"] = state["buf"][:count], state["buf"][count:]
        return out

    return read


class FramingTests(unittest.TestCase):
    def test_accept_key_matches_rfc_6455_example(self):
        self.assertEqual(ws.accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")

    def test_masked_client_frame_round_trips(self):
        frame = ws.encode_frame(0x1, b"hello")
        fin, opcode, payload = ws.read_frame(_reader(frame))
        self.assertEqual((fin, opcode, payload), (True, 0x1, b"hello"))
        self.assertTrue(frame[1] & 0x80, "client frames must be masked")

    def test_unmasked_server_frame_and_extended_lengths(self):
        for size in (125, 126, 300, 65536):
            body = b"x" * size
            frame = ws.encode_frame(0x2, body, mask=False)
            fin, opcode, payload = ws.read_frame(_reader(frame))
            self.assertEqual((fin, opcode, len(payload)), (True, 0x2, size))

    def test_frame_over_the_limit_is_refused(self):
        header = struct.pack("!BBQ", 0x81, 127, ws.MAX_MESSAGE + 1)
        with self.assertRaises(ws.WebSocketError):
            ws.read_frame(_reader(header))


def _server_frame(opcode, payload, fin=True):
    return ws.encode_frame(opcode, payload, mask=False, fin=fin)


class ClientTests(unittest.TestCase):
    def _serve(self, script):
        """A local server that completes the handshake, then runs `script(conn)`. Returns (port, thread)."""
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        port = listener.getsockname()[1]

        def run():
            conn, _ = listener.accept()
            head = b""
            while b"\r\n\r\n" not in head:
                head += conn.recv(1024)
            key = None
            for line in head.decode("ascii").split("\r\n"):
                if line.lower().startswith("sec-websocket-key:"):
                    key = line.split(":", 1)[1].strip()
            reply = (
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                f"Sec-WebSocket-Accept: {ws.accept_key(key)}\r\n\r\n"
            )
            conn.sendall(reply.encode("ascii"))
            try:
                script(conn)
            finally:
                conn.close()
                listener.close()

        thread = threading.Thread(target=run, daemon=True)
        thread.start()
        return port, thread

    def test_echo_text_over_a_local_connection(self):
        def script(conn):
            fin, opcode, payload = ws.read_frame(lambda n: _recv_exact(conn, n))
            conn.sendall(_server_frame(0x1, payload))

        port, thread = self._serve(script)
        client = ws.connect(f"ws://127.0.0.1:{port}/realtime/v1/websocket?apikey=k&vsn=1.0.0", timeout=5)
        client.send_text("ping text 한국어")
        self.assertEqual(client.recv_text(), "ping text 한국어")
        client.close()
        thread.join(5)

    def test_fragments_are_joined_and_ping_is_answered(self):
        seen = {}

        def script(conn):
            conn.sendall(_server_frame(0x1, b"hel", fin=False))
            conn.sendall(_server_frame(0x9, b"p"))
            conn.sendall(_server_frame(0x0, b"lo", fin=True))
            seen["pong"] = ws.read_frame(lambda n: _recv_exact(conn, n))

        port, thread = self._serve(script)
        client = ws.connect(f"ws://127.0.0.1:{port}/", timeout=5)
        self.assertEqual(client.recv_text(), "hello")
        client.close()
        thread.join(5)
        self.assertEqual(seen["pong"][1], 0xA)
        self.assertEqual(seen["pong"][2], b"p")

    def test_close_frame_raises(self):
        port, thread = self._serve(lambda conn: conn.sendall(_server_frame(0x8, b"\x03\xe8")))
        client = ws.connect(f"ws://127.0.0.1:{port}/", timeout=5)
        with self.assertRaises(ws.WebSocketError):
            client.recv_text()
        self.assertTrue(client.closed)
        thread.join(5)

    def test_bad_accept_is_refused(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        port = listener.getsockname()[1]

        def run():
            conn, _ = listener.accept()
            while b"\r\n\r\n" not in conn.recv(1024):
                pass
            conn.sendall(b"HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: wrong\r\n\r\n")
            conn.close()
            listener.close()

        threading.Thread(target=run, daemon=True).start()
        with self.assertRaises(ws.WebSocketError) as ctx:
            ws.connect(f"ws://127.0.0.1:{port}/", timeout=5)
        self.assertNotIn("127.0.0.1", str(ctx.exception))

    def test_non_websocket_scheme_is_refused_without_connecting(self):
        with self.assertRaises(ws.WebSocketError):
            ws.connect("http://example.com/", timeout=1)


def _recv_exact(conn, count):
    data = b""
    while len(data) < count:
        chunk = conn.recv(count - len(data))
        if not chunk:
            raise ws.WebSocketError("short")
        data += chunk
    return data


if __name__ == "__main__":
    unittest.main()
