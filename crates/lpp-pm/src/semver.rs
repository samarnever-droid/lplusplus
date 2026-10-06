//! Standards-backed semantic versions and version requirements.
//!
//! The public wrapper preserves Keel's small API while delegating parsing and
//! matching to the battle-tested `semver` crate. Package versions accept the
//! historical `1` / `1.2` shorthand and normalize it to `1.0.0` / `1.2.0`;
//! full SemVer pre-release and build metadata are preserved.

/// A semantic version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(semver::Version);

impl Version {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self(semver::Version::new(major, minor, patch))
    }

    /// Parse a package version. `1` and `1.2` are accepted for compatibility
    /// and normalized to three components.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let value = value.strip_prefix('v').unwrap_or(value);
        if value.is_empty() {
            return None;
        }
        if let Ok(version) = semver::Version::parse(value) {
            return Some(Self(version));
        }
        if value.bytes().all(|b| b.is_ascii_digit()) {
            return semver::Version::parse(&format!("{value}.0.0"))
                .ok()
                .map(Self);
        }
        if value.matches('.').count() == 1
            && value
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        {
            return semver::Version::parse(&format!("{value}.0")).ok().map(Self);
        }
        None
    }

    pub fn as_semver(&self) -> &semver::Version {
        &self.0
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A standards-compliant semantic-version requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Req {
    Any,
    Parsed {
        raw: String,
        inner: semver::VersionReq,
    },
}

impl Req {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() || value == "*" {
            return Some(Self::Any);
        }
        semver::VersionReq::parse(value)
            .ok()
            .map(|inner| Self::Parsed {
                raw: value.to_string(),
                inner,
            })
    }

    pub fn matches(&self, version: &Version) -> bool {
        match self {
            Self::Any => true,
            Self::Parsed { inner, .. } => inner.matches(version.as_semver()),
        }
    }
}

impl std::fmt::Display for Req {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any => f.write_str("*"),
            Self::Parsed { raw, .. } => f.write_str(raw),
        }
    }
}
