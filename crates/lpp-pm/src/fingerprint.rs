//! Fingerprint incremental builds (docs/rewrite/DELTA.md).
//!
//! A member's fingerprint folds in the compiler identity, the target, its
//! manifest, its own sources, and — crucially — the fingerprints of its
//! path deps. So a change to a dep automatically dirties every transitive
//! dependent; no separate invalidation bookkeeping is needed to rebuild.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{PmError, Result};
use crate::workspace::Workspace;

/// Fingerprint store format version.
pub const FP_STORE_VERSION: u32 = 1;

/// The durable fingerprint entry for one (member, target) build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintEntry {
    pub fingerprint: String,
    /// RFC-3339-ish UTC timestamp (second precision), for humans.
    pub built_at: String,
}

/// The durable fingerprint store: `<workspace>/target/.keel/fingerprints.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintStoreFile {
    pub version: u32,
    /// Key: `"<member>|<target>"`.
    pub entries: BTreeMap<String, FingerprintEntry>,
}

/// The in-memory view of the fingerprint store.
#[derive(Debug, Clone, Default)]
pub struct FingerprintStore {
    path: Option<std::path::PathBuf>,
    pub file: FingerprintStoreFile,
}

impl FingerprintStore {
    /// Load the store from its durable location. A missing or corrupt file
    /// means "cold" (everything rebuilds) — never an error.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(doc) => match FingerprintStoreFile::parse(&doc) {
                Ok(f) => Self {
                    path: Some(path.to_path_buf()),
                    file: f,
                },
                Err(_) => Self {
                    path: Some(path.to_path_buf()),
                    file: FingerprintStoreFile::default(),
                },
            },
            Err(_) => Self {
                path: Some(path.to_path_buf()),
                file: FingerprintStoreFile::default(),
            },
        }
    }

    /// Persist the store (creating parent dirs).
    pub fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PmError::Io(e.to_string()))?;
        }
        let doc = self
            .file
            .to_toml()
            .map_err(|e| PmError::FingerprintStore(e.to_string()))?;
        std::fs::write(path, doc).map_err(|e| PmError::Io(e.to_string()))
    }

    pub fn get(&self, key: &str) -> Option<&FingerprintEntry> {
        self.file.entries.get(key)
    }

    /// Record a successful build for `key`.
    pub fn upsert(&mut self, key: &str, fingerprint: &str) {
        self.file.entries.insert(
            key.to_string(),
            FingerprintEntry {
                fingerprint: fingerprint.to_string(),
                built_at: now_stamp(),
            },
        );
    }

    /// Drop entries whose keys are no longer in `current` (deleted packages).
    pub fn prune(&mut self, current: &BTreeMap<String, String>) {
        self.file
            .entries
            .retain(|k, _| current.contains_key(k));
    }

    /// The fingerprint map (key → fingerprint) as a plain BTreeMap.
    pub fn fps(&self) -> BTreeMap<String, String> {
        self.file
            .entries
            .iter()
            .map(|(k, e)| (k.clone(), e.fingerprint.clone()))
            .collect()
    }
}

impl FingerprintStoreFile {
    pub fn parse(doc: &str) -> Result<Self> {
        toml::from_str(doc).map_err(|e| PmError::FingerprintStore(e.to_string()))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| PmError::FingerprintStore(e.to_string()))
    }
}

fn now_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Seconds → "YYYY-MM-DDTHH:MM:SSZ" (no external calendar crate needed for
    // a coarse human stamp; the fingerprint itself carries no wall-clock).
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's civil algorithm.
fn civil_from_days(z: u64) -> (u64, u64, u64) {
    let z = z as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u64; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u64; // [1, 12]
    ((if m <= 2 { y + 1 } else { y }) as u64, m, d)
}

/// SHA-256 of a file's bytes (hex).
pub fn hash_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(hash_bytes(&bytes))
}

/// SHA-256 of a byte slice (hex).
pub fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The compiler identity: path + mtime (a compiler rebuild changes the mtime).
pub fn lpp_identity(lpp_bin: &str) -> String {
    let mtime = std::fs::metadata(lpp_bin)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!("{lpp_bin}#{mtime}")
}

/// Every `.lpp` file under `dir` (recursive, `target/` excluded) as
/// `(relative path, sha256)`, sorted.
pub fn hash_sources(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string());
                if name.as_deref() == Some("target") {
                    continue;
                }
                walk(&p, base, out);
            } else if p.extension().is_some_and(|x| x == "lpp") {
                if let Some(h) = hash_file(&p) {
                    let rel = p
                        .strip_prefix(base)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .to_string();
                    out.insert(rel.replace('\\', "/"), h);
                }
            }
        }
    }
    walk(dir, dir, &mut out);
    out
}

/// The fingerprint for one (member, target) build.
pub fn member_fingerprint(
    lpp: &str,
    target: &str,
    manifest_toml: &str,
    sources: &BTreeMap<String, String>,
    dep_fps: &BTreeMap<String, String>,
) -> String {
    let mut acc = String::new();
    acc.push_str(&lpp_identity(lpp));
    acc.push('\n');
    acc.push_str(target);
    acc.push('\n');
    acc.push_str(manifest_toml);
    acc.push('\n');
    for (rel, h) in sources {
        acc.push_str(rel);
        acc.push(':');
        acc.push_str(h);
        acc.push('\n');
    }
    for (dep, fp) in dep_fps {
        acc.push_str(dep);
        acc.push('=');
        acc.push_str(fp);
        acc.push('\n');
    }
    hash_bytes(acc.as_bytes())
}

/// Compute the current fingerprint for every (member, target) in the
/// workspace — path deps first, so their fingerprints fold into dependents.
///
/// Key format: `"<member name>|<target>"`.
pub fn compute_member_fps(ws: &Workspace, lpp_bin: &str) -> Result<BTreeMap<String, String>> {
    // Order members so deps come first (the plan gives exactly that).
    let plan = ws.build_plan()?;
    let order: Vec<usize> = plan.iter().flatten().copied().collect();

    let mut fps: BTreeMap<String, String> = BTreeMap::new();
    let manifest_tomls: BTreeMap<usize, String> = ws
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            (
                i,
                m.manifest
                    .to_toml()
                    .unwrap_or_else(|_| String::new()),
            )
        })
        .collect();
    let sources: BTreeMap<usize, BTreeMap<String, String>> = ws
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| (i, hash_sources(&m.dir)))
        .collect();

    for &mi in &order {
        let m = &ws.members[mi];
        let targets: Vec<String> = match &m.manifest.targets {
            Some(t) if !t.supported.is_empty() => t.supported.clone(),
            _ => vec!["host".to_string()],
        };
        // Dep fingerprints: the same target first (deps are computed earlier
        // in `order`), falling back to the dep's host fingerprint.
        let mut dep_fps: BTreeMap<String, String> = BTreeMap::new();
        for dm in ws.members.iter().filter(|dm| m.path_deps.iter().any(|d| d == dm.name())) {
            let mut found: Option<String> = None;
            for target in &targets {
                if let Some(fp) = fps.get(&format!("{}|{}", dm.name(), target)) {
                    found = Some(fp.clone());
                    break;
                }
            }
            if found.is_none() {
                found = fps.get(&format!("{}|host", dm.name())).cloned();
            }
            if let Some(fp) = found {
                dep_fps.insert(dm.name().to_string(), fp);
            }
        }
        for target in &targets {
            let key = format!("{}|{}", m.name(), target);
            fps.insert(
                key,
                member_fingerprint(
                    lpp_bin,
                    target,
                    &manifest_tomls[&mi],
                    &sources[&mi],
                    &dep_fps,
                ),
            );
        }
    }
    Ok(fps)
}

/// The dependents graph over fingerprint keys: key → keys it depends on.
/// (Used by [`crate::delta::invalidate`]; inverted there.)
pub fn dep_key_graph(ws: &Workspace) -> BTreeMap<String, Vec<String>> {
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in &ws.members {
        let targets: Vec<String> = match &m.manifest.targets {
            Some(t) if !t.supported.is_empty() => t.supported.clone(),
            _ => vec!["host".to_string()],
        };
        for target in targets {
            let key = format!("{}|{}", m.name(), target);
            let mut deps = Vec::new();
            for dep_name in &m.path_deps {
                deps.push(format!("{dep_name}|{target}"));
            }
            graph.insert(key, deps);
        }
    }
    graph
}
