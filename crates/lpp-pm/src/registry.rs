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
use crate::blob::{BlobStore, DiskBlobStore};
use crate::error::{PmError, Result};
use crate::index::{IndexEntry, VersionEntry, try_index_path};

/// A git-backed registry: one repository holds the whole index + all blobs.
#[derive(Debug, Clone)]
pub struct Registry {
    remote: String,
    dir: PathBuf,
    blob_store: Option<PathBuf>,
}

impl Registry {
    /// `remote` is the git URL (`file://…` or `https://…`); `dir` is the local
    /// clone. Call [`Self::sync`] before registry-refreshing operations.
    pub fn new(remote: impl Into<String>, dir: impl Into<PathBuf>) -> Self {
        Self {
            remote: remote.into(),
            dir: dir.into(),
            blob_store: None,
        }
    }

    /// Attach a durable content-addressed store shared across registry clones.
    pub fn with_blob_store(mut self, root: impl Into<PathBuf>) -> Self {
        self.blob_store = Some(root.into());
        self
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
        let remote = crate::validation::registry_remote(&self.remote)?;
        if self.dir.join(".git").exists() {
            let origin = self.git_stdout(&["remote", "get-url", "origin"])?;
            if origin.trim() != remote {
                return Err(PmError::Git(format!(
                    "registry cache origin mismatch: cache points to '{}', requested '{}'",
                    origin.trim(),
                    remote
                )));
            }
            self.git(&["fetch", "--depth", "1", "origin"])?;
            self.git(&["reset", "--hard", "FETCH_HEAD"])?;
            self.git(&["clean", "-fd"])?;
            return Ok(());
        }
        let parent = self
            .dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        std::fs::create_dir_all(&parent).map_err(io_err)?;
        let out = self.run_git(
            &parent,
            &[
                "clone",
                "--depth",
                "1",
                "--",
                remote,
                &self.dir.to_string_lossy(),
            ],
        )?;
        fail_on(out)
    }

    /// Read + parse the index entry for `name` (offline, from the local clone).
    pub fn lookup(&self, name: &str) -> Result<IndexEntry> {
        let path = try_index_path(name)?;
        let p = self.dir.join("index").join(path);
        let text =
            std::fs::read_to_string(&p).map_err(|_| PmError::PackageNotFound(name.to_string()))?;
        let entry: IndexEntry =
            serde_json::from_str(&text).map_err(|e| PmError::IndexParse(e.to_string()))?;
        entry.validate()?;
        if entry.name != name {
            return Err(PmError::IndexParse(format!(
                "index path for '{name}' contains entry for '{}'",
                entry.name
            )));
        }
        Ok(entry)
    }

    /// Fetch a version for new resolution. Yanked versions are refused.
    pub fn fetch(&self, name: &str, version: &str) -> Result<(Vec<u8>, VersionEntry)> {
        self.fetch_impl(name, version, false)
    }

    /// Fetch an exactly locked version. Yanking prevents new selection but
    /// does not invalidate an existing lockfile.
    pub fn fetch_locked(&self, name: &str, version: &str) -> Result<(Vec<u8>, VersionEntry)> {
        self.fetch_impl(name, version, true)
    }

    fn fetch_impl(
        &self,
        name: &str,
        version: &str,
        allow_yanked: bool,
    ) -> Result<(Vec<u8>, VersionEntry)> {
        let entry = self.lookup(name)?;
        let version_entry = entry
            .versions
            .iter()
            .find(|candidate| candidate.version == version && (allow_yanked || !candidate.yanked))
            .cloned()
            .ok_or_else(|| PmError::VersionNotFound {
                name: name.to_string(),
                version: version.to_string(),
            })?;
        let address = ContentAddress::try_new(&version_entry.checksum)?;

        let bytes = if let Some(root) = &self.blob_store {
            let store = DiskBlobStore::open(root)?;
            match store.fetch(&address) {
                Ok(bytes) => bytes,
                Err(PmError::BlobNotFound(_)) => {
                    let bytes = std::fs::read(self.dir.join("blob").join(address.as_str()))
                        .map_err(|_| PmError::BlobNotFound(address.to_string()))?;
                    store.insert(&bytes)?;
                    bytes
                }
                Err(error) => return Err(error),
            }
        } else {
            std::fs::read(self.dir.join("blob").join(address.as_str()))
                .map_err(|_| PmError::BlobNotFound(address.to_string()))?
        };
        let actual = ContentAddress::of_bytes(&bytes).to_string();
        if actual != version_entry.checksum {
            return Err(PmError::ChecksumMismatch {
                expected: version_entry.checksum,
                actual,
            });
        }
        Ok((bytes, version_entry))
    }

    /// Write `entry` (index) + `artifact` (blob) into the clone and `git commit`
    /// them together (atomic). Returns the blob's content address.
    pub fn publish(&self, entry: &IndexEntry, artifact: &[u8], message: &str) -> Result<String> {
        crate::validation::registry_remote(&self.remote)?;
        entry.validate()?;
        let checksum = ContentAddress::of_bytes(artifact).to_string();
        if !entry
            .versions
            .iter()
            .any(|version| version.checksum == checksum)
        {
            return Err(PmError::IndexParse(format!(
                "published artifact checksum {checksum} is not referenced by the index entry for {}",
                entry.name
            )));
        }

        let blob_dir = self.dir.join("blob");
        std::fs::create_dir_all(&blob_dir).map_err(io_err)?;
        let blob_path = blob_dir.join(&checksum);
        crate::fsutil::atomic_write(&blob_path, artifact)?;
        let disk_bytes = std::fs::read(&blob_path).map_err(io_err)?;
        let disk_checksum = ContentAddress::of_bytes(&disk_bytes).to_string();
        if disk_checksum != checksum {
            return Err(PmError::ChecksumMismatch {
                expected: checksum,
                actual: disk_checksum,
            });
        }

        let idx = self.dir.join("index").join(try_index_path(&entry.name)?);
        if let Some(parent) = idx.parent() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        let doc =
            serde_json::to_string_pretty(entry).map_err(|e| PmError::IndexParse(e.to_string()))?;
        crate::fsutil::atomic_write(&idx, doc.as_bytes())?;

        // Re-verify the written index entry via lookup before staging.
        let verified = self.lookup(&entry.name)?;
        if verified != *entry {
            return Err(PmError::IndexParse(format!(
                "round-trip verification failed for index entry '{}'",
                entry.name
            )));
        }

        let aggregate_path = self.dir.join("registry").join("index.json");
        if aggregate_path.is_file() {
            self.sync_public_aggregate_index(&aggregate_path)?;
            self.git(&["add", "--", "index", "blob", "registry/index.json"])?;
        } else {
            self.git(&["add", "--", "index", "blob"])?;
        }
        let staged = self.git_stdout(&["status", "--porcelain"])?;
        if !staged.trim().is_empty() {
            self.git(&["commit", "-m", message])?;
        }
        Ok(checksum)
    }

    /// Deterministically update `registry/index.json` in memory (never executing
    /// repository scripts) when publishing to a registry that maintains the
    /// public HTTP mirror catalog (`samarnever-droid/llppregistry`).
    fn sync_public_aggregate_index(&self, aggregate_path: &Path) -> Result<()> {
        const REGISTRY_URL: &str = "https://registry.lplusplus.bond";

        let raw_text = std::fs::read_to_string(aggregate_path).map_err(io_err)?;
        let existing_root: serde_json::Value =
            serde_json::from_str(&raw_text).map_err(|e| PmError::IndexParse(e.to_string()))?;

        let mut packages = serde_json::Map::new();
        if let Some(existing_pkgs) = existing_root.get("packages") {
            match existing_pkgs {
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        packages.insert(k.clone(), v.clone());
                    }
                }
                serde_json::Value::Array(arr) => {
                    for item in arr {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            packages.insert(name.to_string(), item.clone());
                        }
                    }
                }
                _ => {}
            }
        }

        for entry in self.list()? {
            let latest_version = entry
                .versions
                .last()
                .map(|v| v.version.clone())
                .unwrap_or_else(|| "0.0.0".to_string());
            let mut versions_map = serde_json::Map::new();
            for ver in &entry.versions {
                let mut ver_obj = match serde_json::to_value(ver)
                    .map_err(|e| PmError::IndexParse(e.to_string()))?
                {
                    serde_json::Value::Object(m) => m,
                    _ => serde_json::Map::new(),
                };
                ver_obj.insert(
                    "download_url".to_string(),
                    serde_json::Value::String(format!("{REGISTRY_URL}/blob/{}", ver.checksum)),
                );
                versions_map.insert(ver.version.clone(), serde_json::Value::Object(ver_obj));
            }

            let mut pkg_obj = match packages.remove(&entry.name) {
                Some(serde_json::Value::Object(existing)) => existing,
                _ => serde_json::Map::new(),
            };
            pkg_obj.insert(
                "name".to_string(),
                serde_json::Value::String(entry.name.clone()),
            );
            pkg_obj.insert(
                "version".to_string(),
                serde_json::Value::String(latest_version),
            );
            pkg_obj.insert(
                "versions".to_string(),
                serde_json::Value::Object(versions_map),
            );
            packages.insert(entry.name, serde_json::Value::Object(pkg_obj));
        }

        let mut registry_meta = serde_json::Map::new();
        registry_meta.insert(
            "description".to_string(),
            serde_json::Value::String(
                "Official L++ package registry \u{2014} git-backed, static-mirrored, SHA-256 verified"
                    .to_string(),
            ),
        );
        registry_meta.insert(
            "name".to_string(),
            serde_json::Value::String("L++ Official Package Registry".to_string()),
        );
        registry_meta.insert(
            "package_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(packages.len())),
        );
        registry_meta.insert(
            "repository".to_string(),
            serde_json::Value::String("samarnever-droid/llppregistry".to_string()),
        );
        registry_meta.insert(
            "source_of_truth".to_string(),
            serde_json::Value::String("git".to_string()),
        );
        registry_meta.insert(
            "url".to_string(),
            serde_json::Value::String(REGISTRY_URL.to_string()),
        );
        registry_meta.insert(
            "version".to_string(),
            serde_json::Value::String("3.0.0".to_string()),
        );

        let mut root_obj = serde_json::Map::new();
        root_obj.insert("packages".to_string(), serde_json::Value::Object(packages));
        root_obj.insert(
            "registry".to_string(),
            serde_json::Value::Object(registry_meta),
        );

        let mut formatted = serde_json::to_string_pretty(&serde_json::Value::Object(root_obj))
            .map_err(|e| PmError::IndexParse(e.to_string()))?;
        formatted.push('\n');
        crate::fsutil::atomic_write(aggregate_path, formatted.as_bytes())
    }

    /// Push the clone to its remote (needs write access). Call after [`Self::publish`].
    pub fn push(&self) -> Result<()> {
        crate::validation::registry_remote(&self.remote)?;
        self.git(&["push", "origin", "HEAD"])
    }

    /// Every package in the registry (parses all index docs), sorted by name.
    pub fn list(&self) -> Result<Vec<IndexEntry>> {
        let index_root = self.dir.join("index");
        let mut out = Vec::new();
        let mut stack = vec![index_root.clone()];
        while let Some(dir) = stack.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
                {
                    continue;
                }
                let meta = std::fs::symlink_metadata(&p).map_err(io_err)?;
                if meta.file_type().is_symlink() {
                    return Err(PmError::IndexParse(format!(
                        "refusing symlink in registry index: {}",
                        p.display()
                    )));
                }
                if meta.is_dir() {
                    stack.push(p);
                } else if meta.is_file() {
                    let text = std::fs::read_to_string(&p).map_err(io_err)?;
                    let entry: IndexEntry = serde_json::from_str(&text).map_err(|error| {
                        PmError::IndexParse(format!("{}: {error}", p.display()))
                    })?;
                    entry.validate()?;
                    let expected_rel = try_index_path(&entry.name)?;
                    let actual_rel = p
                        .strip_prefix(&index_root)
                        .map_err(|e| PmError::IndexParse(e.to_string()))?;
                    if actual_rel != Path::new(&expected_rel) {
                        return Err(PmError::IndexParse(format!(
                            "index entry '{}' at unexpected path {}",
                            entry.name,
                            actual_rel.display()
                        )));
                    }
                    out.push(entry);
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
            .args([
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.https.allow=always",
                "-c",
                "protocol.ssh.allow=always",
                "-c",
                "protocol.file.allow=always",
                "-c",
                "core.symlinks=false",
                "-c",
                "core.fsmonitor=false",
            ])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ALLOW_PROTOCOL", "https:ssh:file")
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
