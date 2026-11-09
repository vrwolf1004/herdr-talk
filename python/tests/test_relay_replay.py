import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

from relay_replay import NonceCache, WINDOW_SECONDS  # noqa: E402


class ReplayTests(unittest.TestCase):
    def test_same_nonce_is_refused_the_second_time(self):
        cache = NonceCache()
        self.assertTrue(cache.accept("a" * 32, 1000, 1000))
        self.assertFalse(cache.accept("a" * 32, 1000, 1001))

    def test_window_edges(self):
        cache = NonceCache()
        self.assertTrue(cache.accept("1" * 32, 1000 - 299, 1000))
        self.assertFalse(cache.accept("2" * 32, 1000 - 301, 1000))
        self.assertTrue(cache.accept("3" * 32, 1000 + 299, 1000))
        self.assertFalse(cache.accept("4" * 32, 1000 + 301, 1000))

    def test_entries_leave_after_the_window(self):
        cache = NonceCache()
        self.assertTrue(cache.accept("a" * 32, 1000, 1000))
        later = 1000 + WINDOW_SECONDS + 10
        self.assertTrue(cache.accept("b" * 32, later, later))
        self.assertEqual(len(cache), 1)

    def test_full_cache_refuses_new_and_keeps_live_entries(self):
        cache = NonceCache(capacity=2)
        self.assertTrue(cache.accept("a" * 32, 1000, 1000))
        self.assertTrue(cache.accept("b" * 32, 1000, 1000))
        self.assertFalse(cache.accept("c" * 32, 1000, 1000))
        self.assertFalse(cache.accept("a" * 32, 1000, 1000))
        self.assertEqual(len(cache), 2)

    def test_room_returns_after_expiry(self):
        cache = NonceCache(capacity=1)
        self.assertTrue(cache.accept("a" * 32, 1000, 1000))
        later = 1000 + WINDOW_SECONDS + 1
        self.assertTrue(cache.accept("b" * 32, later, later))

    def test_refused_message_is_not_recorded(self):
        cache = NonceCache()
        self.assertFalse(cache.accept("a" * 32, 1000 - 400, 1000))
        self.assertTrue(cache.accept("a" * 32, 1000, 1000))


if __name__ == "__main__":
    unittest.main()
