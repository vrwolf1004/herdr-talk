//! Replay protection for relay messages: a memory cache of the nonces seen in the last window.
//! A message is accepted once, and only while its ts is within WINDOW_SECONDS of now. The cache lives in memory,
//! so a restart empties it: a message captured before a restart can be replayed within the window after it.
//! Entries older than the window are dropped. When the cache is full of entries still inside the window, the new
//! message is refused (the cache never drops a live nonce, so an old one cannot come back).

use std::collections::{HashSet, VecDeque};

pub const WINDOW_SECONDS: u64 = 300;
pub const CAPACITY: usize = 10_000;

pub struct NonceCache {
    capacity: usize,
    /// (ts, nonce) in arrival order.
    entries: VecDeque<(u64, String)>,
    seen: HashSet<String>,
}

impl Default for NonceCache {
    fn default() -> Self {
        Self::with_capacity(CAPACITY)
    }
}

impl NonceCache {
    pub fn with_capacity(capacity: usize) -> Self {
        NonceCache { capacity, entries: VecDeque::new(), seen: HashSet::new() }
    }

    /// Accept a message with this nonce and ts at time `now`, once. False: refused (replayed, outside the window,
    /// or the cache is full of live entries). Nothing is recorded for a refused message.
    pub fn accept(&mut self, nonce: &str, ts: u64, now: u64) -> bool {
        if now.abs_diff(ts) > WINDOW_SECONDS {
            return false;
        }
        self.prune(now);
        if self.seen.contains(nonce) || self.entries.len() >= self.capacity {
            return false;
        }
        self.seen.insert(nonce.to_string());
        self.entries.push_back((ts, nonce.to_string()));
        true
    }

    /// Drop entries whose ts has left the window.
    fn prune(&mut self, now: u64) {
        while let Some((ts, _)) = self.entries.front() {
            if now.abs_diff(*ts) <= WINDOW_SECONDS {
                break;
            }
            let (_, nonce) = self.entries.pop_front().expect("front exists");
            self.seen.remove(&nonce);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000;

    #[test]
    fn the_same_nonce_is_refused_the_second_time() {
        let mut cache = NonceCache::default();
        assert!(cache.accept("aa", NOW, NOW));
        assert!(!cache.accept("aa", NOW, NOW));
        assert!(cache.accept("bb", NOW, NOW));
    }

    #[test]
    fn ts_299_seconds_off_is_accepted_and_301_is_refused() {
        let mut cache = NonceCache::default();
        assert!(cache.accept("a1", NOW - 299, NOW));
        assert!(cache.accept("a2", NOW + 299, NOW));
        assert!(!cache.accept("a3", NOW - 301, NOW));
        assert!(!cache.accept("a4", NOW + 301, NOW));
    }

    #[test]
    fn entries_leave_the_cache_when_the_window_passes() {
        let mut cache = NonceCache::default();
        assert!(cache.accept("old", NOW, NOW));
        assert_eq!(cache.len(), 1);
        // A later message prunes the old entry; a replay of "old" is now outside the window anyway.
        assert!(cache.accept("new", NOW + 400, NOW + 400));
        assert_eq!(cache.len(), 1);
        assert!(!cache.accept("old", NOW, NOW + 400));
    }

    #[test]
    fn a_full_cache_of_live_entries_refuses_new_messages() {
        let mut cache = NonceCache::with_capacity(3);
        assert!(cache.accept("n1", NOW, NOW));
        assert!(cache.accept("n2", NOW, NOW));
        assert!(cache.accept("n3", NOW, NOW));
        assert!(!cache.accept("n4", NOW, NOW), "full: refused, live nonces are never dropped");
        assert!(!cache.accept("n1", NOW, NOW), "the oldest live nonce is still remembered");
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn a_full_cache_frees_room_when_entries_expire() {
        let mut cache = NonceCache::with_capacity(2);
        assert!(cache.accept("n1", NOW, NOW));
        assert!(cache.accept("n2", NOW, NOW));
        assert!(cache.accept("n3", NOW + 301, NOW + 301), "n1 and n2 have left the window");
        assert_eq!(cache.len(), 1);
    }
}
