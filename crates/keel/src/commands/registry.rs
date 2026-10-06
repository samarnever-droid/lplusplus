//! `keel fetch` / `keel search` / `keel publish` — the git-registry commands.

use std::path::Path;

use tabled::builder::Builder;

use lpp_pm::registry::Registry;

/// Split `name@version` into `(name, Option<version>)`.
fn split_name_version(s: &str) -> (&str, Option<&str>) {
    match s.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (s, None),
    }
}

fn cmp_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let left = lpp_pm::Version::parse(left).expect("validated registry version");
    let right = lpp_pm::Version::parse(right).expect("validated registry version");
    left.cmp(&right)
}

/// Fetch one package: `keel fetch <name>` (latest) or `keel fetch <name>@<version>`.
pub fn fetch(reg: &Registry, target: &str) -> Result<(), String> {
    reg.sync().map_err(|e| e.to_string())?;
    let (name, maybe_version) = split_name_version(target);
    let entry = reg.lookup(name).map_err(|e| e.to_string())?;
    let available: Vec<String> = entry
        .versions
        .iter()
        .filter(|v| !v.yanked)
        .map(|v| v.version.clone())
        .collect();
    if available.is_empty() {
        return Err(format!("package '{name}' has no available versions"));
    }
    let version = match maybe_version {
        Some(v) => v.to_string(),
        None => available
            .iter()
            .max_by(|a, b| cmp_versions(a, b))
            .cloned()
            .unwrap(),
    };
    let (bytes, v) = reg.fetch(name, &version).map_err(|e| e.to_string())?;
    let mut b = Builder::default();
    b.push_record(["field".to_string(), "value".to_string()]);
    b.push_record(["package".to_string(), name.to_string()]);
    b.push_record(["version".to_string(), v.version]);
    b.push_record(["checksum".to_string(), v.checksum]);
    b.push_record(["size".to_string(), format!("{} bytes", bytes.len())]);
    b.push_record(["registry".to_string(), reg.remote().to_string()]);
    println!("{}", b.build());
    Ok(())
}

/// Search the registry index by (substring of) name: `keel search <query>`.
pub fn search(reg: &Registry, query: &str) -> Result<(), String> {
    reg.sync().map_err(|e| e.to_string())?;
    let hits = reg.search(query).map_err(|e| e.to_string())?;
    if hits.is_empty() {
        println!("no packages matching '{query}'");
        return Ok(());
    }
    let mut b = Builder::default();
    b.push_record(["package".to_string(), "versions".to_string()]);
    for e in hits {
        let versions = e
            .versions
            .iter()
            .map(|v| v.version.clone())
            .collect::<Vec<_>>()
            .join(", ");
        b.push_record([e.name, versions]);
    }
    println!("{}", b.build());
    Ok(())
}

/// Registry candidates for a package name (for the resolver). Yanked
/// versions are never offered.
fn candidates(reg: &Registry, name: &str) -> Option<Vec<lpp_pm::Candidate>> {
    reg.lookup(name).ok().map(|entry| {
        entry
            .versions
            .into_iter()
            .filter(|ve| !ve.yanked)
            .map(|version| lpp_pm::Candidate {
                version: lpp_pm::Version::parse(&version.version)
                    .expect("Registry::lookup validates concrete versions"),
                checksum: Some(version.checksum),
                source: "registry".to_string(),
                deps: version
                    .deps
                    .into_iter()
                    .filter(|dependency| !dependency.optional)
                    .map(|dependency| {
                        (
                            dependency.name,
                            lpp_pm::Req::parse(&dependency.req)
                                .expect("Registry::lookup validates requirements"),
                        )
                    })
                    .collect(),
            })
            .collect()
    })
}

/// Resolve the WORKSPACE's full dependency graph against the registry and
/// write ONE `Keel.lock` at the workspace root. (`dir` is any dir inside the
/// workspace; the blobs are already in the local clone — per-package
/// verification is `keel fetch <name>`.)
/// The workspace's resolver roots: one [`lpp_pm::Pkg`] per member, with
/// path deps and member-named deps excluded (they never hit the registry).
/// Shared by `keel fetch` (all) and `keel update`.
pub fn workspace_roots(ws: &lpp_pm::Workspace) -> Result<Vec<lpp_pm::Pkg>, String> {
    let member_names: std::collections::BTreeSet<String> =
        ws.members.iter().map(|m| m.name().to_string()).collect();
    ws.members
        .iter()
        .map(|m| {
            let version = lpp_pm::validation::version(m.manifest.version())
                .map_err(|error| error.to_string())?;
            let default_features = m
                .manifest
                .features
                .get("default")
                .cloned()
                .unwrap_or_default();
            let deps = m
                .manifest
                .dependencies
                .iter()
                .filter(|(name, dependency)| {
                    dependency.path().is_none()
                        && !member_names.contains(name.as_str())
                        && (!dependency.optional()
                            || default_features.iter().any(|feature| feature == *name))
                })
                .map(|(name, dependency)| {
                    Ok((
                        name.clone(),
                        lpp_pm::validation::requirement(dependency.version())
                            .map_err(|error| error.to_string())?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(lpp_pm::Pkg {
                name: m.name().to_string(),
                version,
                checksum: None,
                source: if m.dir == ws.root {
                    "root".to_string()
                } else {
                    "path".to_string()
                },
                deps,
            })
        })
        .collect()
}

/// Resolve and cache every dependency required by the workspace.
///
/// A local/path-only workspace installs without a configured registry and gets
/// a deterministic lockfile containing its workspace packages. Registry-backed
/// dependencies require the normal `--registry`/`KEEL_REGISTRY` configuration.
pub fn install(reg: Option<&Registry>, dir: &Path) -> Result<(), String> {
    let workspace = lpp_pm::Workspace::discover(dir).map_err(|error| error.to_string())?;
    let roots = workspace_roots(&workspace)?;
    let has_registry_dependencies = roots.iter().any(|package| !package.deps.is_empty());
    if has_registry_dependencies {
        let registry = reg.ok_or_else(|| {
            "registry dependencies require --registry <git-url> or KEEL_REGISTRY".to_string()
        })?;
        return fetch_all(registry, dir);
    }

    let resolved = lpp_pm::resolve_workspace(&roots, &|_| Some(Vec::new()))
        .map_err(|error| error.to_string())?;
    let lock = lpp_pm::Lock::from_resolved(&resolved);
    lock.save_atomic(&workspace.root.join("Keel.lock"))
        .map_err(|error| error.to_string())?;
    println!(
        "installed 0 registry package(s); locked {} workspace package(s)",
        roots.len()
    );
    Ok(())
}

pub fn fetch_all(reg: &Registry, dir: &Path) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|error| error.to_string())?;
    let roots = workspace_roots(&ws)?;
    let lock_path = ws.root.join("Keel.lock");

    // `fetch` preserves a valid lock. `update` is the command that changes
    // selected versions.
    if let Ok(document) = std::fs::read_to_string(&lock_path)
        && let Ok(lock) = lpp_pm::Lock::parse(&document)
        && lock_satisfies_roots(&lock, &roots)
    {
        if let Some(identity) = &lock.registry
            && identity != reg.remote()
        {
            return Err(format!(
                "Keel.lock belongs to registry '{identity}', but '{}' was requested",
                reg.remote()
            ));
        }
        reg.sync().map_err(|error| error.to_string())?;
        verify_locked_artifacts(reg, &lock)?;
        println!("Keel.lock is unchanged; locked artifacts verified and cached.");
        return Ok(());
    }

    reg.sync().map_err(|error| error.to_string())?;
    let resolved = lpp_pm::resolve_workspace(&roots, &|name| candidates(reg, name))
        .map_err(|error| error.to_string())?;
    let mut lock = lpp_pm::Lock::from_resolved(&resolved);
    lock.registry = Some(reg.remote().to_string());
    verify_locked_artifacts(reg, &lock)?;
    lock.save_atomic(&lock_path)
        .map_err(|error| error.to_string())?;

    let mut table = Builder::default();
    table.push_record(["package".to_string(), "version".to_string()]);
    for package in resolved.packages.values() {
        if package.source == "registry" {
            table.push_record([package.name.clone(), package.version.to_string()]);
        }
    }
    println!(
        "resolved {} package(s) → Keel.lock\n{}",
        resolved.len().saturating_sub(ws.members.len()),
        table.build()
    );
    Ok(())
}

fn lock_satisfies_roots(lock: &lpp_pm::Lock, roots: &[lpp_pm::Pkg]) -> bool {
    roots.iter().all(|root| {
        let Some(locked_root) = lock.package(&root.name) else {
            return false;
        };
        if locked_root.version != root.version.to_string() {
            return false;
        }
        root.deps.iter().all(|(name, requirement)| {
            lock.package(name)
                .and_then(|package| lpp_pm::Version::parse(&package.version))
                .is_some_and(|version| requirement.matches(&version))
        })
    })
}

fn verify_locked_artifacts(reg: &Registry, lock: &lpp_pm::Lock) -> Result<(), String> {
    for package in lock
        .packages
        .iter()
        .filter(|package| package.source == "registry")
    {
        let expected = package
            .checksum
            .as_deref()
            .ok_or_else(|| format!("locked registry package '{}' has no checksum", package.name))?;
        let (bytes, _) = reg
            .fetch_locked(&package.name, &package.version)
            .map_err(|error| error.to_string())?;
        let actual = lpp_pm::ContentAddress::of_bytes(&bytes).to_string();
        if actual != expected {
            return Err(lpp_pm::PmError::ChecksumMismatch {
                expected: expected.to_string(),
                actual,
            }
            .to_string());
        }
        let store = lpp_pm::DiskBlobStore::open(crate::cache_dir().join("content"))
            .map_err(|error| error.to_string())?;
        lpp_pm::BlobStore::insert(&store, &bytes).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Merge a new version into a package's index entry (append-only version
/// history). Returns [`lpp_pm::PmError::PublishConflict`] when the version
/// already exists: `same = true` for an identical artifact (no-op
/// republish), `false` for a conflicting artifact (immutability).
pub fn merge_publish(
    existing: Option<&lpp_pm::index::IndexEntry>,
    name: &str,
    new: lpp_pm::index::VersionEntry,
) -> Result<lpp_pm::index::IndexEntry, lpp_pm::PmError> {
    lpp_pm::validation::package_name(name)?;
    lpp_pm::validation::version(&new.version)?;
    lpp_pm::validation::checksum(&new.checksum)?;
    if let Some(entry) = existing {
        entry.validate()?;
        if entry.name != name {
            return Err(lpp_pm::PmError::IndexParse(format!(
                "cannot merge package '{name}' into index entry '{}'",
                entry.name
            )));
        }
    }
    match existing {
        None => Ok(lpp_pm::index::IndexEntry {
            name: name.to_string(),
            versions: vec![new],
        }),
        Some(entry) => {
            if let Some(old) = entry.versions.iter().find(|v| v.version == new.version) {
                return Err(lpp_pm::PmError::PublishConflict {
                    name: name.to_string(),
                    version: new.version.clone(),
                    same: old.checksum == new.checksum,
                });
            }
            let mut versions = entry.versions.clone();
            versions.push(new);
            versions.sort_by(|left, right| cmp_versions(&left.version, &right.version));
            Ok(lpp_pm::index::IndexEntry {
                name: name.to_string(),
                versions,
            })
        }
    }
}

/// Publish the project in `directory` to the registry.
pub fn publish(reg: &Registry, directory: &Path) -> Result<(), String> {
    // Ensure the registry clone exists first (publish commits into it).
    reg.sync().map_err(|e| e.to_string())?;
    let manifest_path = directory.join("Keel.toml");
    if !manifest_path.exists() {
        return Err("no Keel.toml in the project directory — run `keel init` first".to_string());
    }
    let manifest = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let name = manifest.name().to_string();
    let version = manifest.version().to_string();

    if let Some((dependency, _)) = manifest
        .dependencies
        .iter()
        .find(|(_, dependency)| dependency.path().is_some())
    {
        return Err(format!(
            "cannot publish with path dependency '{dependency}'; publish it and use a registry version requirement first"
        ));
    }
    let artifact = super::archive::pack(directory)?;
    let checksum = lpp_pm::ContentAddress::of_bytes(&artifact).to_string();
    let deps: Vec<lpp_pm::index::DepSpec> = manifest
        .dependencies
        .iter()
        .map(|(dependency_name, dependency)| lpp_pm::index::DepSpec {
            name: dependency_name.clone(),
            req: dependency.version().to_string(),
            optional: dependency.optional(),
            features: dependency.features().to_vec(),
        })
        .collect();
    let new_version = lpp_pm::index::VersionEntry {
        version: version.clone(),
        deps,
        features: manifest.features.clone(),
        checksum: checksum.clone(),
        targets: manifest
            .targets
            .as_ref()
            .map(|targets| targets.supported.clone())
            .unwrap_or_default(),
        yanked: false,
    };
    // Merge into the existing index entry: published versions are
    // immutable, and a new publish appends rather than clobbering the
    // package's history (clobbering would silently "remove" old versions
    // from every consumer's lockfile).
    let existing = match reg.lookup(&name) {
        Ok(entry) => Some(entry),
        Err(lpp_pm::PmError::PackageNotFound(_)) => None,
        Err(error) => return Err(error.to_string()),
    };
    let entry =
        merge_publish(existing.as_ref(), &name, new_version).map_err(|error| error.to_string())?;

    reg.publish(&entry, &artifact, &format!("publish {name} {version}"))
        .map_err(|e| e.to_string())?;
    reg.push().map_err(|e| e.to_string())?;
    println!(
        "published {name} {version} (sha256 {checksum}) → {}",
        reg.remote()
    );
    Ok(())
}
