//! The `Keel.toml` package manifest.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{PmError, Result};

/// A parsed `Keel.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub package: Package,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, Dependency>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targets: Option<Targets>,
    /// The optional `[workspace]` section (workspace roots only).
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceSection>,
}

/// The `[package]` section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    #[serde(default = "default_edition")]
    pub edition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_edition() -> String {
    "2024".to_string()
}

/// A dependency: a bare version (`otherlib = "1"`) or a detailed table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Dependency {
    /// `otherlib = "1"`
    Version(String),
    /// `simdlib = { version = "2", optional = true, features = ["fast"] }`
    /// or a monorepo path dep: `math = { path = "../crates/math" }`.
    Detailed {
        /// Optional: path deps carry no version; omitted = any version.
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        optional: bool,
        #[serde(default)]
        features: Vec<String>,
        /// Monorepo path dependency (a sibling package).
        #[serde(default)]
        path: Option<String>,
        /// Git dependency.
        #[serde(default)]
        git: Option<String>,
    },
}

impl Dependency {
    /// The version requirement, regardless of form (`"*"` when absent).
    pub fn version(&self) -> &str {
        match self {
            Self::Version(v) => v,
            Self::Detailed { version, .. } => version.as_deref().unwrap_or("*"),
        }
    }

    /// The local path for a path dependency, if any.
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Version(_) => None,
            Self::Detailed { path, .. } => path.as_deref(),
        }
    }
}

/// The optional `[workspace]` section (workspace roots only).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkspaceSection {
    /// Member directories, relative to the root. One `*` per pattern is
    /// allowed (e.g. `crates/*`); plain directories match exactly.
    #[serde(default)]
    pub members: Vec<String>,
}

/// The optional `[targets]` section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Targets {
    pub supported: Vec<String>,
}

impl Manifest {
    /// Parse a `Keel.toml` document.
    pub fn parse(doc: &str) -> Result<Self> {
        toml::from_str(doc).map_err(|e| PmError::ManifestParse(e.to_string()))
    }

    /// Serialize back to `Keel.toml` format.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| PmError::ManifestParse(e.to_string()))
    }

    pub fn name(&self) -> &str {
        &self.package.name
    }
    pub fn version(&self) -> &str {
        &self.package.version
    }
}
