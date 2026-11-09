"""Replay protection for relay messages (same rules as rust/crates/talk-core/src/relay_replay.rs).

A message is accepted once, and only while its ts is within WINDOW_SECONDS of now. The cache lives in memory, so a
restart empties it. When the cache is full of entries still inside the window, a new message is refused: a live nonce
is never dropped, so an old one cannot come back.
"""

from collections import deque

WINDOW_SECONDS = 300
CAPACITY = 10_000


class NonceCache:
    def __init__(self, capacity=CAPACITY):
        self.capacity = capacity
        self._entries = deque()  # (ts, nonce) in arrival order
        self._seen = set()

    def accept(self, nonce, ts, now):
        """True when the message is accepted. Nothing is recorded for a refused one."""
        if abs(now - ts) > WINDOW_SECONDS:
            return False
        self._prune(now)
        if nonce in self._seen or len(self._entries) >= self.capacity:
            return False
        self._seen.add(nonce)
        self._entries.append((ts, nonce))
        return True

    def _prune(self, now):
        while self._entries and abs(now - self._entries[0][0]) > WINDOW_SECONDS:
            _, nonce = self._entries.popleft()
            self._seen.discard(nonce)

    def __len__(self):
        return len(self._entries)
