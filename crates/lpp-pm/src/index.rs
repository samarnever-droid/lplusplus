//! The registry index schema + the sparse index URL layout.
//!
//! The registry (`registry.lplusplus.bond`) serves one JSON document per
//! package (the "sparse index"); this module is its schema + the name→path rule.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A per-package index document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: String,
    pub versions: Vec<VersionEntry>,
}

/// One published version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionEntry {
    pub version: String,
    #[serde(default)]
    pub deps: Vec<DepSpec>,
    #[serde(default)]
    pub features: BTreeMap<String, Vec<String>>,
    /// SHA-256 of the package artifact (its content address).
    pub checksum: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub yanked: bool,
}

/// A dependency edge in the index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DepSpec {
    pub name: String,
    pub req: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub features: Vec<String>,
}

/// The sparse index path for a package name (cargo-compatible, cache-friendly):
/// 1 char → `1/<n>` · 2 → `2/<n>` · 3 → `3/<a>/<bc>` · 4+ → `<aa>/<bb>/<name>`.
///
/// Package names are ASCII (lowercase alnum + hyphen), so byte slicing is safe.
pub fn index_path(name: &str) -> String {
    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{}", &name[0..1], &name[1..]),
        _ => format!("{}/{}/{}", &name[0..2], &name[2..4], name),
    }
}

impl IndexEntry {
    /// The artifact download URL for `version`, given the registry base URL.
    pub fn download_url(&self, base: &str, version: &str) -> Option<String> {
        self.versions
            .iter()
            .find(|v| v.version == version)
            .map(|v| format!("{base}/blob/{}", v.checksum))
    }
}
