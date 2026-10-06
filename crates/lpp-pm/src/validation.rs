//! Validation at package-manager trust boundaries.

use crate::address::ContentAddress;
use crate::error::{PmError, Result};
use crate::semver::{Req, Version};

/// Validate a package name used in manifests, sparse-index paths, lockfiles,
/// and staging directories.
///
/// Names are 1..=64 ASCII characters, begin with a lowercase letter or digit,
/// and contain only lowercase letters, digits, and `-`.
pub fn package_name(name: &str) -> Result<()> {
    let bytes = name.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= 64
        && matches!(bytes[0], b'a'..=b'z' | b'0'..=b'9')
        && bytes
            .iter()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(PmError::InvalidPackageName(name.to_string()))
    }
}

/// Validate a concrete package version.
pub fn version(value: &str) -> Result<Version> {
    Version::parse(value).ok_or_else(|| PmError::InvalidVersion(value.to_string()))
}

/// Validate a dependency requirement.
pub fn requirement(value: &str) -> Result<Req> {
    Req::parse(value).ok_or_else(|| PmError::InvalidRequirement(value.to_string()))
}

/// Validate a SHA-256 content address.
pub fn checksum(value: &str) -> Result<ContentAddress> {
    ContentAddress::try_new(value)
}
