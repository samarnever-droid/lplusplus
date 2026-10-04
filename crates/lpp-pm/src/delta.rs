//! The Delta layer: the diff between the last build's fingerprints and the
//! current ones, and the transitive *rebuild set* it implies
//! (docs/rewrite/DELTA.md).

use std::collections::{BTreeMap, BTreeSet};

/// The diff between two fingerprint maps (key = `"<member>|<target>"`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delta {
    /// Keys present now but not before.
    pub added: Vec<String>,
    /// Keys present in both, with a different fingerprint.
    pub changed: Vec<String>,
    /// Keys present before but not now (deleted packages).
    pub removed: Vec<String>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }

    /// Every key that is new, changed, or gone.
    pub fn all(&self) -> BTreeSet<String> {
        self.added
            .iter()
            .chain(self.changed.iter())
            .chain(self.removed.iter())
            .cloned()
            .collect()
    }

    /// The affected member names (keys strip their `|target`).
    pub fn members(&self) -> BTreeSet<String> {
        self.all()
            .iter()
            .map(|k| k.split('|').next().unwrap_or(k).to_string())
            .collect()
    }
}

/// Diff `stored` (last build) against `current` (now).
pub fn diff(stored: &BTreeMap<String, String>, current: &BTreeMap<String, String>) -> Delta {
    let mut delta = Delta::default();
    for (k, fp) in current {
        match stored.get(k) {
            Some(old) if old == fp => {}
            Some(_) => delta.changed.push(k.clone()),
            None => delta.added.push(k.clone()),
        }
    }
    for k in stored.keys() {
        if !current.contains_key(k) {
            delta.removed.push(k.clone());
        }
    }
    delta.added.sort();
    delta.changed.sort();
    delta.removed.sort();
    delta
}

/// The rebuild set implied by `delta`: every affected key PLUS all of its
/// transitive dependents.
///
/// `depends_on` maps key → the keys it depends on (direct edges).
pub fn invalidate(delta: &Delta, depends_on: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    // Reverse edges: key → its dependents.
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, deps) in depends_on {
        for d in deps {
            dependents.entry(d.clone()).or_default().push(k.clone());
        }
    }
    let mut set: BTreeSet<String> = delta.all();
    let mut worklist: Vec<String> = set.iter().cloned().collect();
    while let Some(k) = worklist.pop() {
        for dep_of in dependents.get(&k).into_iter().flatten() {
            if set.insert(dep_of.clone()) {
                worklist.push(dep_of.clone());
            }
        }
    }
    set
}

/// Mirror fingerprint entries into a hot KV cache.
/// Keys: `fp/<member>|<target>` → fingerprint. The durable TOML store stays
/// the source of truth; this is the opt-in hot layer.
pub fn mirror_to_kv(kv: &mut dyn crate::kv::KvCache, fps: &BTreeMap<String, String>) {
    for (k, fp) in fps {
        kv.set(&format!("fp/{k}"), fp.as_bytes());
    }
}
