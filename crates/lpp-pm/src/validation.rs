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

/// Validate a git registry remote URL or local repository path.
///
/// Rejects empty strings, leading `-` (Git option injection CVE-2017-1000117),
/// ASCII control characters, and insecure or code-executing transports (`ext::`,
/// `fd::`, plaintext `http://`, `git://`, `ftp://`).
pub fn registry_remote(remote: &str) -> Result<&str> {
    let trimmed = remote.trim();
    if trimmed.is_empty() {
        return Err(PmError::Git("registry remote cannot be empty".to_string()));
    }
    if trimmed.starts_with('-') {
        return Err(PmError::Git(format!(
            "invalid registry remote '{trimmed}': remote must not start with '-'"
        )));
    }
    if trimmed.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(PmError::Git(
            "invalid registry remote: control characters are forbidden".to_string(),
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    for forbidden in ["ext::", "fd::", "http://", "git://", "ftp://", "ftps://"] {
        if lower.starts_with(forbidden) {
            return Err(PmError::Git(format!(
                "insecure or forbidden git transport in registry remote '{trimmed}'"
            )));
        }
    }
    Ok(trimmed)
}
