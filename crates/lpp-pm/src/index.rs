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
pub fn try_index_path(name: &str) -> crate::Result<String> {
    crate::validation::package_name(name)?;
    Ok(match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{}", &name[0..1], &name[1..]),
        _ => format!("{}/{}/{}", &name[0..2], &name[2..4], name),
    })
}

/// Sparse-index path for an already validated package name.
///
/// Kept for source compatibility; registry code uses [`try_index_path`] and
/// returns a typed error rather than panicking on hostile input.
pub fn index_path(name: &str) -> String {
    try_index_path(name).unwrap_or_else(|_| "invalid/invalid/invalid".to_string())
}

impl IndexEntry {
    pub fn validate(&self) -> crate::Result<()> {
        crate::validation::package_name(&self.name)?;
        let mut seen = std::collections::BTreeSet::new();
        for version in &self.versions {
            crate::validation::version(&version.version)?;
            if !seen.insert(version.version.clone()) {
                return Err(crate::PmError::IndexParse(format!(
                    "duplicate version {} for {}",
                    version.version, self.name
                )));
            }
            crate::validation::checksum(&version.checksum)?;
            for dependency in &version.deps {
                crate::validation::package_name(&dependency.name)?;
                crate::validation::requirement(&dependency.req)?;
            }
        }
        Ok(())
    }

    /// The artifact download URL for `version`, given the registry base URL.
    pub fn download_url(&self, base: &str, version: &str) -> Option<String> {
        self.versions
            .iter()
            .find(|v| v.version == version)
            .map(|v| format!("{base}/blob/{}", v.checksum))
    }
}
