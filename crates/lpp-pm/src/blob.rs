//! The durable, content-addressed blob store — the source of truth for
//! downloaded packages and built artifacts.

use std::path::{Path, PathBuf};

use crate::address::ContentAddress;
use crate::error::{PmError, Result};

/// A content-addressed store: bytes in, their address out. Idempotent —
/// storing the same bytes twice yields the same address and one blob on disk.
pub trait BlobStore {
    fn insert(&self, bytes: &[u8]) -> Result<ContentAddress>;
    fn fetch(&self, address: &ContentAddress) -> Result<Vec<u8>>;
    fn contains(&self, address: &ContentAddress) -> bool;
    /// The on-disk root of this store.
    fn root(&self) -> &Path;
}

/// A content-addressed store laid out as `<root>/blobs/<xx>/<rest>` (two-char
/// fanout, git-objects-style), so a large cache stays a forest of 256 shallow
/// directories rather than one giant flat one.
pub struct DiskBlobStore {
    root: PathBuf,
}

impl DiskBlobStore {
    /// Open (creating if needed) a store rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("blobs"))
            .map_err(|e| PmError::Io(e.to_string()))?;
        Ok(Self { root })
    }

    fn blob_path(&self, address: &ContentAddress) -> PathBuf {
        let s = address.as_str();
        self.root.join("blobs").join(&s[0..2]).join(&s[2..])
    }
}

impl BlobStore for DiskBlobStore {
    fn insert(&self, bytes: &[u8]) -> Result<ContentAddress> {
        let address = ContentAddress::of_bytes(bytes);
        let path = self.blob_path(&address);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| PmError::Io(e.to_string()))?;
            }
            std::fs::write(&path, bytes).map_err(|e| PmError::Io(e.to_string()))?;
        }
        Ok(address)
    }

    fn fetch(&self, address: &ContentAddress) -> Result<Vec<u8>> {
        let path = self.blob_path(address);
        std::fs::read(&path).map_err(|_| PmError::BlobNotFound(address.as_str().to_string()))
    }

    fn contains(&self, address: &ContentAddress) -> bool {
        self.blob_path(address).exists()
    }

    fn root(&self) -> &Path {
        &self.root
    }
}
