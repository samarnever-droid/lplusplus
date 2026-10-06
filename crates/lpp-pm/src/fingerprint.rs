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
pub const FP_STORE_VERSION: u32 = 2;

/// The durable fingerprint entry for one (member, target) build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintEntry {
    pub fingerprint: String,
    /// Hash of the produced artifact. Older stores omit it and rebuild once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hash: Option<String>,
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
                Ok(f) if f.version == FP_STORE_VERSION => Self {
                    path: Some(path.to_path_buf()),
                    file: f,
                },
                Ok(_) | Err(_) => Self {
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
        let document = self
            .file
            .to_toml()
            .map_err(|e| PmError::FingerprintStore(e.to_string()))?;
        crate::fsutil::atomic_write(path, document.as_bytes())
    }

    pub fn get(&self, key: &str) -> Option<&FingerprintEntry> {
        self.file.entries.get(key)
    }

    /// Record a successful build for `key` without an artifact hash (primarily
    /// useful to callers/tests that do not produce an artifact).
    pub fn upsert(&mut self, key: &str, fingerprint: &str) {
        self.upsert_artifact(key, fingerprint, None);
    }

    pub fn upsert_artifact(&mut self, key: &str, fingerprint: &str, artifact_hash: Option<String>) {
        self.file.version = FP_STORE_VERSION;
        self.file.entries.insert(
            key.to_string(),
            FingerprintEntry {
                fingerprint: fingerprint.to_string(),
                artifact_hash,
                built_at: now_stamp(),
            },
        );
    }

    /// Drop entries whose keys are no longer in `current` (deleted packages).
    pub fn prune(&mut self, current: &BTreeMap<String, String>) {
        self.file.entries.retain(|k, _| current.contains_key(k));
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
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
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

/// Resolve the compiler through PATH (when necessary) and hash its bytes.
/// This makes a compiler replacement invalidate every affected artifact even
/// when Keel was configured with the ordinary `lpp` command name.
pub fn lpp_identity(lpp_bin: &str) -> String {
    let requested = std::path::PathBuf::from(lpp_bin);
    let resolved = if requested.components().count() > 1 || requested.is_absolute() {
        requested
    } else {
        std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join(lpp_bin))
                    .find(|candidate| candidate.is_file())
            })
            .unwrap_or_else(|| std::path::PathBuf::from(lpp_bin))
    };
    let canonical = resolved.canonicalize().unwrap_or(resolved);
    match std::fs::read(&canonical) {
        Ok(bytes) => format!("{}#{}", canonical.display(), hash_bytes(&bytes)),
        Err(_) => format!("{}#missing", canonical.display()),
    }
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
            let Ok(metadata) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string());
                if matches!(
                    name.as_deref(),
                    Some("target" | ".git" | ".lpp_packages" | "tests" | "examples")
                ) {
                    continue;
                }
                walk(&p, base, out);
            } else if metadata.is_file() && p.extension().is_some_and(|x| x == "lpp") {
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
    compute_member_fps_with_lock(ws, lpp_bin, None)
}

/// Compute fingerprints with the registry portion of `Keel.lock` folded into
/// every member that can reach it.
pub fn compute_member_fps_with_lock(
    ws: &Workspace,
    lpp_bin: &str,
    lock: Option<&crate::Lock>,
) -> Result<BTreeMap<String, String>> {
    // Order members so deps come first (the plan gives exactly that).
    let plan = ws.build_plan()?;
    let order: Vec<usize> = plan.iter().flatten().copied().collect();

    let mut fps: BTreeMap<String, String> = BTreeMap::new();
    let manifest_tomls: BTreeMap<usize, String> = ws
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| (i, m.manifest.to_toml().unwrap_or_else(|_| String::new())))
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
        let registry_fps = registry_fingerprints(m, lock)?;
        for target in &targets {
            // Prefer the dependency artifact for the same target and fall back
            // to host only when the dependency does not declare that target.
            let mut dep_fps: BTreeMap<String, String> = registry_fps.clone();
            for dependency_member in ws
                .members
                .iter()
                .filter(|dependency| m.path_deps.iter().any(|name| name == dependency.name()))
            {
                let found = fps
                    .get(&format!("{}|{}", dependency_member.name(), target))
                    .or_else(|| fps.get(&format!("{}|host", dependency_member.name())))
                    .cloned();
                let Some(found) = found else {
                    return Err(PmError::FingerprintStore(format!(
                        "no compatible fingerprint for path dependency '{}' target '{}'",
                        dependency_member.name(),
                        target
                    )));
                };
                dep_fps.insert(dependency_member.name().to_string(), found);
            }
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

fn registry_fingerprints(
    member: &crate::Member,
    lock: Option<&crate::Lock>,
) -> Result<BTreeMap<String, String>> {
    let mut output = BTreeMap::new();
    let Some(lock) = lock else {
        return Ok(output);
    };
    let default_features = member
        .manifest
        .features
        .get("default")
        .cloned()
        .unwrap_or_default();
    let mut queue: Vec<String> = member
        .manifest
        .dependencies
        .iter()
        .filter(|(name, dependency)| {
            dependency.path().is_none()
                && (!dependency.optional()
                    || default_features.iter().any(|feature| feature == *name))
        })
        .map(|(name, _)| name.clone())
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let package = lock.package(&name).ok_or_else(|| {
            PmError::LockParse(format!("dependency '{name}' is not present in Keel.lock"))
        })?;
        if package.source != "registry" {
            continue;
        }
        let checksum = package.checksum.as_deref().ok_or_else(|| {
            PmError::LockParse(format!("registry package '{name}' has no checksum"))
        })?;
        output.insert(
            format!("registry:{name}"),
            format!("{}:{checksum}", package.version),
        );
        queue.extend(package.deps.iter().cloned());
    }
    Ok(output)
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
