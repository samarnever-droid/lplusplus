//! The hot, in-memory key/value cache.
//!
//! [`KvCache`] is a string-keyed, byte-valued cache with hit/miss
//! accounting. The backend ([`InMemoryKv`]) is a deterministic `BTreeMap`.
//!
//! The cache is an *index over* the durable blob store, so it is cheap to
//! rebuild and is never the source of truth.

use std::collections::BTreeMap;

/// A key/value cache with hit/miss accounting. Mutating operations take
/// `&mut self` (single-threaded CLI ownership); reads of the key set and
/// stats take `&self`.
pub trait KvCache: Send + Sync {
    fn get(&mut self, key: &str) -> Option<Vec<u8>>;
    fn set(&mut self, key: &str, value: &[u8]);
    fn del(&mut self, key: &str) -> bool;
    /// Deterministic (sorted) enumeration of keys, for `scan`/`list`.
    fn keys(&self) -> Vec<String>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// A short, stable identifier for the backend, for `stats`/diagnostics.
    fn name(&self) -> &'static str;
    fn stats(&self) -> CacheStats;
}

/// Aggregate hit/miss/size accounting for a cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: u64,
}

impl CacheStats {
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// The boring, deterministic default backend. `BTreeMap` gives stable key
/// ordering and `O(log n)` ops — plenty for a metadata/build-fingerprint
/// index, and it is trivially safe to rely on.
#[derive(Default)]
pub struct InMemoryKv {
    map: BTreeMap<String, Vec<u8>>,
    hits: u64,
    misses: u64,
}

impl InMemoryKv {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KvCache for InMemoryKv {
    fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        match self.map.get(key) {
            Some(v) => {
                self.hits += 1;
                Some(v.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    fn set(&mut self, key: &str, value: &[u8]) {
        self.map.insert(key.to_string(), value.to_vec());
    }

    fn del(&mut self, key: &str) -> bool {
        self.map.remove(key).is_some()
    }

    fn keys(&self) -> Vec<String> {
        self.map.keys().cloned().collect()
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    fn name(&self) -> &'static str {
        "memory"
    }

    fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.map.len() as u64,
        }
    }
}
