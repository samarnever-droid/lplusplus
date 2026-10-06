//! The validated `Keel.lock` model.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{PmError, Result};
use crate::resolve::Resolved;

pub const LOCK_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    #[serde(rename = "version")]
    pub lock_version: u32,
    /// Canonical registry identity used to create this lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
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
    /// Exact requirements associated with `deps`. Kept separately so older
    /// lockfiles with name-only edges remain parseable and can be upgraded.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dep_reqs: BTreeMap<String, String>,
}

impl Lock {
    pub fn from_resolved(resolved: &Resolved) -> Self {
        let mut packages: Vec<LockedPkg> = resolved
            .packages
            .values()
            .map(|package| LockedPkg {
                name: package.name.clone(),
                version: package.version.to_string(),
                source: package.source.clone(),
                checksum: package.checksum.clone(),
                deps: package.deps.iter().map(|(name, _)| name.clone()).collect(),
                dep_reqs: package
                    .deps
                    .iter()
                    .map(|(name, requirement)| (name.clone(), requirement.to_string()))
                    .collect(),
            })
            .collect();
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        Lock {
            lock_version: LOCK_VERSION,
            registry: None,
            packages,
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(|e| PmError::LockParse(e.to_string()))
    }

    pub fn parse(doc: &str) -> Result<Self> {
        let lock: Self = toml::from_str(doc).map_err(|e| PmError::LockParse(e.to_string()))?;
        lock.validate()?;
        Ok(lock)
    }

    pub fn validate(&self) -> Result<()> {
        if self.lock_version != 1 && self.lock_version != LOCK_VERSION {
            return Err(PmError::LockParse(format!(
                "unsupported lock format version {} (supported: 1 and {LOCK_VERSION})",
                self.lock_version
            )));
        }
        let mut names = BTreeSet::new();
        for package in &self.packages {
            crate::validation::package_name(&package.name)?;
            crate::validation::version(&package.version)?;
            if !names.insert(package.name.clone()) {
                return Err(PmError::LockParse(format!(
                    "duplicate package '{}'",
                    package.name
                )));
            }
            match package.source.as_str() {
                "root" | "path" => {
                    if package.checksum.is_some() {
                        return Err(PmError::LockParse(format!(
                            "local package '{}' must not have a registry checksum",
                            package.name
                        )));
                    }
                }
                "registry" => {
                    let checksum = package.checksum.as_deref().ok_or_else(|| {
                        PmError::LockParse(format!(
                            "registry package '{}' has no checksum",
                            package.name
                        ))
                    })?;
                    crate::validation::checksum(checksum)?;
                }
                other => {
                    return Err(PmError::LockParse(format!(
                        "package '{}' has unknown source '{other}'",
                        package.name
                    )));
                }
            }
            for dependency in &package.deps {
                crate::validation::package_name(dependency)?;
            }
            for (dependency, requirement) in &package.dep_reqs {
                crate::validation::package_name(dependency)?;
                crate::validation::requirement(requirement)?;
                if !package.deps.contains(dependency) {
                    return Err(PmError::LockParse(format!(
                        "package '{}' has a requirement for undeclared dependency '{}'",
                        package.name, dependency
                    )));
                }
            }
        }
        for package in &self.packages {
            for dependency in &package.deps {
                if !names.contains(dependency) {
                    return Err(PmError::LockParse(format!(
                        "package '{}' references missing dependency '{}'",
                        package.name, dependency
                    )));
                }
                if let Some(requirement) = package.dep_reqs.get(dependency) {
                    let requirement = crate::validation::requirement(requirement)?;
                    let selected = self
                        .packages
                        .iter()
                        .find(|candidate| candidate.name == *dependency)
                        .expect("dependency existence checked above");
                    let selected_version = crate::validation::version(&selected.version)?;
                    if !requirement.matches(&selected_version) {
                        return Err(PmError::LockParse(format!(
                            "{} requires {} {}, but lock selects {}",
                            package.name, dependency, requirement, selected.version
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        let document = self.to_toml()?;
        crate::fsutil::atomic_write(path, document.as_bytes())
    }

    pub fn checksum(&self, name: &str) -> Option<&str> {
        self.packages
            .iter()
            .find(|package| package.name == name)
            .and_then(|package| package.checksum.as_deref())
    }

    pub fn package(&self, name: &str) -> Option<&LockedPkg> {
        self.packages.iter().find(|package| package.name == name)
    }
}
