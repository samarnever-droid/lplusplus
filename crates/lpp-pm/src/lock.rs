//! The `Keel.lock` model — the exact resolved dependency set (do not edit).

use serde::{Deserialize, Serialize};

use crate::error::{PmError, Result};
use crate::resolve::Resolved;

/// Lockfile format version.
pub const LOCK_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    #[serde(rename = "version")]
    pub lock_version: u32,
    pub packages: Vec<LockedPkg>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPkg {
    pub name: String,
    pub version: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default)]
    pub deps: Vec<String>,
}

impl Lock {
    /// Build a lockfile from a resolved set (deterministic: packages sorted by name).
    pub fn from_resolved(resolved: &Resolved) -> Self {
        let mut packages: Vec<LockedPkg> = resolved
            .packages
            .values()
            .map(|p| LockedPkg {
                name: p.name.clone(),
                version: p.version.to_string(),
                source: p.source.clone(),
                checksum: p.checksum.clone(),
                deps: p.deps.iter().map(|(n, _)| n.clone()).collect(),
            })
            .collect();
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Lock {
            lock_version: LOCK_VERSION,
            packages,
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| PmError::LockParse(e.to_string()))
    }

    pub fn parse(doc: &str) -> Result<Self> {
        toml::from_str(doc).map_err(|e| PmError::LockParse(e.to_string()))
    }

    /// The checksum for a package name, if it's a (registry) package with one.
    pub fn checksum(&self, name: &str) -> Option<&str> {
        self.packages
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.checksum.as_deref())
    }
}
