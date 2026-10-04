//! Content addressing for the global package cache.
//!
//! A [`ContentAddress`] is the hex-encoded SHA-256 of a byte payload. It is
//! the stable, cross-run key that both the durable blob store and the hot
//! index agree on, mirroring how cargo addresses registry blobs.

use sha2::{Digest, Sha256};

use crate::error::PmError;

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentAddress(String);

impl ContentAddress {
    /// Compute the address of a byte payload.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        Self(hex_of(digest.as_slice()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Validate and parse an address string (64 lowercase hex chars).
    pub fn try_new(s: &str) -> Result<Self, PmError> {
        let valid = s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if valid {
            Ok(Self(s.to_string()))
        } else {
            Err(PmError::InvalidAddress(s.to_string()))
        }
    }
}

impl std::fmt::Display for ContentAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for ContentAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ContentAddress(\"{}\")", self.0)
    }
}

fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
