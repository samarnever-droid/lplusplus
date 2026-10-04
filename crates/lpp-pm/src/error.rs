//! Error model for the package-manager core.
//!
//! Codes live in the **`E6xxx`** package-manager block, distinct from the
//! codegen `E5xxx` and driver `E9xxx` blocks.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmError {
    /// A [`ContentAddress`](crate::ContentAddress) was malformed (not 64
    /// lowercase hex characters).
    InvalidAddress(String),
    /// An I/O failure in the durable blob store.
    Io(String),
    /// A stored blob is missing from the durable store.
    BlobNotFound(String),
    /// A cached value exceeded the configured size limit.
    ValueTooLarge { limit: usize, actual: usize },
    /// A manifest could not be parsed (bad TOML or missing required fields).
    ManifestParse(String),
    /// The requested package is not present in the registry index.
    PackageNotFound(String),
    /// The requested version of a package is absent or yanked.
    VersionNotFound { name: String, version: String },
    /// A fetched blob's SHA-256 did not match its recorded checksum.
    ChecksumMismatch { expected: String, actual: String },
    /// A registry index document was malformed JSON.
    IndexParse(String),
    /// The `git` binary was missing or could not be spawned.
    Git(String),
    /// A `git` command exited non-zero.
    GitCommand { code: Option<i32>, stderr: String },
    /// A dependency conflict: two incompatible requirements for one package.
    ResolveConflict {
        name: String,
        chosen: String,
        required: String,
    },
    /// No available version of a package satisfies a requirement.
    NoMatchingVersion { name: String, req: String },
    /// A `Keel.lock` could not be parsed or serialized.
    LockParse(String),
    /// A dependency cycle among workspace members.
    WorkspaceCycle { chain: String },
    /// A `[workspace].members` entry has no package manifest.
    MemberNotFound { member: String },
    /// Two workspace members share a package name.
    DuplicateMember { name: String },
    /// A path dependency points outside the workspace.
    PathDepOutsideWorkspace { dep: String, from: String },
    /// An unsupported `[workspace].members` glob pattern.
    BadMemberPattern(String),
    /// A fingerprint store file could not be read or parsed (treated as cold).
    FingerprintStore(String),
    /// A pinned (locked) version no longer exists in the registry — it was
    /// removed or yanked after the lockfile was written.
    LockedVersionGone { name: String, version: String },
    /// No `Keel.lock` in the workspace (run `keel fetch` first).
    NoLockFile,
    /// A package name was not found in `Keel.lock`.
    NotInLockFile(String),
    /// A publish of a version that already exists in the registry. `same`
    /// = identical artifact (a no-op republish); `false` = conflicting
    /// artifact (published versions are immutable).
    PublishConflict {
        name: String,
        version: String,
        same: bool,
    },
}

impl std::fmt::Display for PmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAddress(a) => write!(f, "E6001: invalid content address: {a}"),
            Self::Io(e) => write!(f, "E6003: cache I/O error: {e}"),
            Self::BlobNotFound(k) => write!(f, "E6004: blob not found: {k}"),
            Self::ValueTooLarge { limit, actual } => {
                write!(f, "E6005: value {actual} bytes exceeds limit {limit}")
            }
            Self::ManifestParse(m) => write!(f, "E6006: failed to parse manifest: {m}"),
            Self::PackageNotFound(n) => write!(f, "E6007: package not found in registry: {n}"),
            Self::VersionNotFound { name, version } => {
                write!(
                    f,
                    "E6008: version {version} of package {name} not found or yanked"
                )
            }
            Self::ChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "E6009: checksum mismatch: expected {expected}, got {actual}"
                )
            }
            Self::IndexParse(e) => write!(f, "E6010: malformed registry index: {e}"),
            Self::Git(e) => write!(f, "E6011: git error: {e}"),
            Self::GitCommand { code, stderr } => {
                write!(f, "E6012: git command failed (exit {code:?}): {stderr}")
            }
            Self::ResolveConflict {
                name,
                chosen,
                required,
            } => {
                write!(
                    f,
                    "E6013: version conflict for {name}: have {chosen}, but {required} is required"
                )
            }
            Self::NoMatchingVersion { name, req } => {
                write!(f, "E6014: no available version of {name} matches {req}")
            }
            Self::LockParse(e) => write!(f, "E6015: Keel.lock error: {e}"),
            Self::WorkspaceCycle { chain } => {
                write!(
                    f,
                    "E6016: dependency cycle among workspace members: {chain}"
                )
            }
            Self::MemberNotFound { member } => {
                write!(
                    f,
                    "E6017: workspace member '{member}' has no Keel.toml [package]"
                )
            }
            Self::DuplicateMember { name } => {
                write!(f, "E6018: duplicate workspace member name: {name}")
            }
            Self::PathDepOutsideWorkspace { dep, from } => {
                write!(
                    f,
                    "E6019: path dependency '{dep}' (from '{from}') is not a workspace member"
                )
            }
            Self::BadMemberPattern(p) => {
                write!(
                    f,
                    "E6020: unsupported members pattern '{p}' (one '*' per pattern)"
                )
            }
            Self::FingerprintStore(e) => write!(f, "E6021: fingerprint store error: {e}"),
            Self::LockedVersionGone { name, version } => write!(
                f,
                "E6022: locked version {version} of {name} is no longer available in the registry; run `keel update` to re-resolve"
            ),
            Self::NoLockFile => write!(
                f,
                "E6023: no Keel.lock in this workspace — run `keel fetch` first"
            ),
            Self::NotInLockFile(n) => write!(f, "E6024: '{n}' is not in Keel.lock"),
            Self::PublishConflict {
                name,
                version,
                same,
            } => {
                if *same {
                    write!(
                        f,
                        "E6025: {name} {version} is already published (identical artifact — nothing to do)"
                    )
                } else {
                    write!(
                        f,
                        "E6025: {name} {version} is already published with a different checksum — published versions are immutable"
                    )
                }
            }
        }
    }
}

impl std::error::Error for PmError {}

pub type Result<T> = std::result::Result<T, PmError>;
