//! Fast In-Memory Monotonic HLC Deduplication Buffer
//!
//! Provides a bounded, thread-safe cache to detect and discard duplicate or
//! out-of-order events before dispatching expensive database transactions.

use crate::hlc::is_stale;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Mutex;

/// Bounded deduplication buffer mapping keys to their latest seen HLC timestamp and optional value.
pub struct DeduplicationBuffer<K, V = ()> {
    capacity: usize,
    entries: Mutex<(HashMap<K, (String, V)>, VecDeque<K>)>,
}

impl<K: Hash + Eq + Clone, V: Clone> DeduplicationBuffer<K, V> {
    /// Creates a new deduplication buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    /// Evaluates if an incoming HLC timestamp is strictly newer than the cached HLC for `key`.
    ///
    /// If fresh: updates the cache with `(hlc, val)` and returns `true`.
    /// If stale or duplicate: leaves the cache untouched and returns `false`.
    pub fn check_and_update(&self, key: &K, hlc: &str, val: V) -> bool {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let (ref mut map, ref mut queue) = *guard;
        if let Some((existing_hlc, _)) = map.get(key) {
            if is_stale(hlc, existing_hlc) {
                return false;
            }
        } else {
            if map.len() >= self.capacity {
                if let Some(oldest) = queue.pop_front() {
                    map.remove(&oldest);
                }
            }
            queue.push_back(key.clone());
        }

        map.insert(key.clone(), (hlc.to_string(), val));
        true
    }

    /// Returns the cached value for `key` if present.
    pub fn get(&self, key: &K) -> Option<(String, V)> {
        let guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.0.get(key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deduplication_buffer_monotonic_filtering() {
        let dedup = DeduplicationBuffer::new(10);
        let key = "entity-1".to_string();

        // Fresh event 100.001
        assert!(dedup.check_and_update(&key, "100.001", 42));
        assert_eq!(dedup.get(&key), Some(("100.001".to_string(), 42)));

        // Duplicate event 100.001 rejected
        assert!(!dedup.check_and_update(&key, "100.001", 99));

        // Stale older event 99.999 rejected
        assert!(!dedup.check_and_update(&key, "99.999", 99));

        // Newer event 100.002 accepted
        assert!(dedup.check_and_update(&key, "100.002", 100));
        assert_eq!(dedup.get(&key), Some(("100.002".to_string(), 100)));
    }
}
