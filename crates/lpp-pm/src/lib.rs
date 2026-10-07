//! L++ package-manager core.
//!
//! This is the storage/index foundation of the rewritten `lpp` command
//! surface (see `docs/rewrite/PM_CLI_REWRITE.md`). It owns three things and
//! depends on no compiler stage, so the CLI and the driver can share it:
//!
//! - **Content addressing** ([`ContentAddress`]) — stable SHA-256 keys, the
//!   same convention cargo uses for registry blobs.
//! - **The durable blob store** ([`BlobStore`], [`DiskBlobStore`]) —
//!   content-addressed artifacts on disk. This is the source of truth.
//! - **The hot key/value cache** ([`KvCache`], [`InMemoryKv`]) — a fast index
//!   layer that sits *over* the durable store. A package index is small and the
//!   registry is git-decentralized, so the deterministic in-memory map is the
//!   only backend (see [`backend`]).
//!
//! The crate is deliberately dependency-light (only `sha2`).
#![allow(clippy::all, warnings)]

pub mod address;
pub mod backend;
pub mod blob;
pub mod delta;
pub mod error;
pub mod fingerprint;
mod fsutil;
pub mod index;
pub mod kv;
pub mod lock;
pub mod manifest;
pub mod registry;
pub mod resolve;
pub mod semver;
pub mod validation;
pub mod workspace;

pub use address::ContentAddress;
pub use backend::{KvBackendKind, create_kv};
pub use blob::{BlobStore, DiskBlobStore};
pub use error::{PmError, Result};
pub use kv::{CacheStats, InMemoryKv, KvCache};
pub use lock::{Lock, LockedPkg};
pub use registry::Registry;
pub use resolve::{Candidate, Pkg, Resolved, resolve, resolve_workspace};
pub use semver::{Req, Version};
pub use workspace::{Member, Workspace};
