//! Cache backend selection.
//!
//! The CLI exposes `--cache-backend <kind>`; the core maps the choice to a
//! [`KvCache`]. The deterministic in-memory map is the single backend: a
//! package index is small (tens of thousands of entries) and the registry is
//! git-decentralized (a git index + GitHub releases, no central store), so a
//! heavyweight distributed cache engine is unwarranted here.

use crate::error::Result;
use crate::kv::{InMemoryKv, KvCache};

/// Which hot KV cache to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KvBackendKind {
    /// Pick the best available backend. Currently the in-memory map.
    #[default]
    Auto,
    /// The deterministic in-memory map (the safe default).
    Memory,
}

impl KvBackendKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "memory" => Some(Self::Memory),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Memory => "memory",
        }
    }
}

/// Build a hot KV cache for the requested backend. Both kinds return the
/// deterministic in-memory map.
pub fn create_kv(kind: KvBackendKind) -> Result<Box<dyn KvCache>> {
    match kind {
        KvBackendKind::Auto | KvBackendKind::Memory => Ok(Box::new(InMemoryKv::new())),
    }
}
