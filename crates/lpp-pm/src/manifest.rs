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

    pub fn optional(&self) -> bool {
        match self {
            Self::Version(_) => false,
            Self::Detailed { optional, .. } => *optional,
        }
    }

    pub fn features(&self) -> &[String] {
        match self {
            Self::Version(_) => &[],
            Self::Detailed { features, .. } => features,
        }
    }

    pub fn git(&self) -> Option<&str> {
        match self {
            Self::Version(_) => None,
            Self::Detailed { git, .. } => git.as_deref(),
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
    /// Parse and validate a `Keel.toml` document.
    pub fn parse(doc: &str) -> Result<Self> {
        let manifest: Self =
            toml::from_str(doc).map_err(|e| PmError::ManifestParse(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        crate::validation::package_name(&self.package.name)?;
        crate::validation::version(&self.package.version)?;
        if self.package.edition.trim().is_empty() {
            return Err(PmError::ManifestParse(
                "package edition cannot be empty".to_string(),
            ));
        }
        for (name, dependency) in &self.dependencies {
            crate::validation::package_name(name)?;
            crate::validation::requirement(dependency.version())?;
            if dependency.path().is_some_and(|path| path.trim().is_empty()) {
                return Err(PmError::ManifestParse(format!(
                    "path dependency '{name}' has an empty path"
                )));
            }
            if dependency.path().is_some() && dependency.git().is_some() {
                return Err(PmError::ManifestParse(format!(
                    "dependency '{name}' cannot specify both path and git"
                )));
            }
            if dependency.git().is_some() {
                return Err(PmError::ManifestParse(format!(
                    "git dependency '{name}' is not supported yet; publish it to the configured registry or use a workspace path dependency"
                )));
            }
        }
        for (feature, members) in &self.features {
            if feature.trim().is_empty() {
                return Err(PmError::ManifestParse(
                    "feature names cannot be empty".to_string(),
                ));
            }
            for member in members {
                if member.trim().is_empty() {
                    return Err(PmError::ManifestParse(format!(
                        "feature '{feature}' contains an empty member"
                    )));
                }
            }
        }
        Ok(())
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
