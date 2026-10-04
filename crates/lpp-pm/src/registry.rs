//! The git-backed registry client.
//!
//! The registry is a **git repository** with a fixed layout:
//!
//! ```text
//! index/<sparse-path for name>   → the per-package [`IndexEntry`] (JSON)
//! blob/<sha256>                   → the package artifact (bytes), named by content
//! ```
//!
//! **Reads are offline**: [`Registry::sync`] clones (or fast-forwards) the repo
//! once, after which every [`Registry::lookup`] / [`Registry::fetch`] reads
//! straight from the local clone. **Writes** ([`Registry::publish`] +
//! [`Registry::push`]) are a `git commit` (plus an optional `git push`).
//!
//! This module drives the system `git` binary via `std::process::Command`, so
//! it adds no dependencies.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::address::ContentAddress;
use crate::error::{PmError, Result};
use crate::index::{IndexEntry, VersionEntry, index_path};

/// A git-backed registry: one repository holds the whole index + all blobs.
#[derive(Debug, Clone)]
pub struct Registry {
    remote: String,
    dir: PathBuf,
}

impl Registry {
    /// `remote` is the git URL (`file://…` or `https://…`); `dir` is the local
    /// clone. Call [`Self::sync`] before reading.
    pub fn new(remote: impl Into<String>, dir: impl Into<PathBuf>) -> Self {
        Self { remote: remote.into(), dir: dir.into() }
    }

    /// The git remote URL.
    pub fn remote(&self) -> &str {
        &self.remote
    }

    /// The local clone directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Clone the registry if `dir` is not a clone yet, else fast-forward it to
    /// the remote tip (discarding any local changes).
    pub fn sync(&self) -> Result<()> {
        if self.dir.join(".git").exists() {
            self.git(&["fetch", "--depth", "1", "origin"])?;
            self.git(&["reset", "--hard", "FETCH_HEAD"])?;
            self.git(&["clean", "-fd"])?;
            return Ok(());
        }
        let parent = self.dir.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        std::fs::create_dir_all(&parent).map_err(io_err)?;
        let out = self.run_git(
            &parent,
            &["clone", "--depth", "1", &self.remote, &self.dir.to_string_lossy()],
        )?;
        fail_on(out)
    }

    /// Read + parse the index entry for `name` (offline, from the local clone).
    pub fn lookup(&self, name: &str) -> Result<IndexEntry> {
        let p = self.dir.join("index").join(index_path(name));
        let text = std::fs::read_to_string(&p)
            .map_err(|_| PmError::PackageNotFound(name.to_string()))?;
        serde_json::from_str(&text).map_err(|e| PmError::IndexParse(e.to_string()))
    }

    /// Fetch one version: [`Self::lookup`] → pick the (non-yanked) version →
    /// read its blob → verify its SHA-256. Returns the artifact + its entry.
    pub fn fetch(&self, name: &str, version: &str) -> Result<(Vec<u8>, VersionEntry)> {
        let entry = self.lookup(name)?;
        let v = entry
            .versions
            .iter()
            .find(|v| v.version == version && !v.yanked)
            .cloned()
            .ok_or_else(|| PmError::VersionNotFound { name: name.to_string(), version: version.to_string() })?;
        let bytes = std::fs::read(self.dir.join("blob").join(&v.checksum))
            .map_err(|_| PmError::BlobNotFound(v.checksum.clone()))?;
        let actual = ContentAddress::of_bytes(&bytes).to_string();
        if actual != v.checksum {
            return Err(PmError::ChecksumMismatch { expected: v.checksum, actual });
        }
        Ok((bytes, v))
    }

    /// Write `entry` (index) + `artifact` (blob) into the clone and `git commit`
    /// them together (atomic). Returns the blob's content address.
    pub fn publish(&self, entry: &IndexEntry, artifact: &[u8], message: &str) -> Result<String> {
        let checksum = ContentAddress::of_bytes(artifact).to_string();

        let blob_dir = self.dir.join("blob");
        std::fs::create_dir_all(&blob_dir).map_err(io_err)?;
        std::fs::write(blob_dir.join(&checksum), artifact).map_err(io_err)?;

        let idx = self.dir.join("index").join(index_path(&entry.name));
        if let Some(parent) = idx.parent() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        let doc = serde_json::to_string_pretty(entry).map_err(|e| PmError::IndexParse(e.to_string()))?;
        std::fs::write(&idx, doc).map_err(io_err)?;

        self.git(&["add", "index", "blob"])?;
        let staged = self.git_stdout(&["status", "--porcelain"])?;
        if !staged.trim().is_empty() {
            self.git(&["commit", "-m", message])?;
        }
        Ok(checksum)
    }

    /// Push the clone to its remote (needs write access). Call after [`Self::publish`].
    pub fn push(&self) -> Result<()> {
        self.git(&["push", "origin", "HEAD"])
    }

    /// Every package in the registry (parses all index docs), sorted by name.
    pub fn list(&self) -> Result<Vec<IndexEntry>> {
        let mut out = Vec::new();
        let mut stack = vec![self.dir.join("index")];
        while let Some(dir) = stack.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(text) = std::fs::read_to_string(&p) {
                    if let Ok(entry) = serde_json::from_str::<IndexEntry>(&text) {
                        out.push(entry);
                    }
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Case-insensitive name search over [`Self::list`].
    pub fn search(&self, query: &str) -> Result<Vec<IndexEntry>> {
        let q = query.to_lowercase();
        Ok(self
            .list()?
            .into_iter()
            .filter(|e| e.name.to_lowercase().contains(&q))
            .collect())
    }

    // -- git plumbing -------------------------------------------------------

    fn run_git(&self, cwd: &Path, args: &[&str]) -> Result<Output> {
        Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| PmError::Git(format!("failed to run `git {args:?}`: {e}")))
    }

    /// Run `git <args>` in the clone; error on a non-zero exit.
    fn git(&self, args: &[&str]) -> Result<()> {
        fail_on(self.run_git(&self.dir, args)?)
    }

    /// Run `git <args>` in the clone; return stdout on success.
    fn git_stdout(&self, args: &[&str]) -> Result<String> {
        let out = self.run_git(&self.dir, args)?;
        if !out.status.success() {
            return Err(PmError::GitCommand {
                code: out.status.code(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

fn fail_on(out: Output) -> Result<()> {
    if out.status.success() {
        Ok(())
    } else {
        Err(PmError::GitCommand {
            code: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

fn io_err(e: std::io::Error) -> PmError {
    PmError::Io(e.to_string())
}
