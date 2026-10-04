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

/// The numeric components of a dotted version (`1.2.3` → `[1,2,3]`).
fn version_key(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|s| {
            let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<u64>().unwrap_or(0)
        })
        .collect()
}

/// A minimal dotted-version compare (enough to pick "latest").
fn cmp_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let (ka, kb) = (version_key(a), version_key(b));
    for i in 0..ka.len().max(kb.len()) {
        let x = ka.get(i).copied().unwrap_or(0);
        let y = kb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            o @ (std::cmp::Ordering::Less | std::cmp::Ordering::Greater) => return o,
            _ => {}
        }
    }
    std::cmp::Ordering::Equal
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
            .map(|ve| lpp_pm::Candidate {
                version: lpp_pm::Version::parse(&ve.version)
                    .unwrap_or(lpp_pm::Version::new(0, 0, 0)),
                checksum: Some(ve.checksum),
                source: "registry".to_string(),
                deps: ve
                    .deps
                    .into_iter()
                    .map(|d| {
                        (
                            d.name,
                            lpp_pm::Req::parse(&d.req).unwrap_or(lpp_pm::Req::Any),
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
            let version = lpp_pm::Version::parse(m.manifest.version())
                .ok_or_else(|| format!("invalid version in Keel.toml: {}", m.manifest.version()))?;
            Ok(lpp_pm::Pkg {
                name: m.name().to_string(),
                version,
                checksum: None,
                // Path deps are local (never in the registry); deps named
                // after a member are the member itself.
                source: if m.dir == ws.root {
                    "root".to_string()
                } else {
                    "path".to_string()
                },
                deps: m
                    .manifest
                    .dependencies
                    .iter()
                    .filter(|(n, d)| d.path().is_none() && !member_names.contains(n.as_str()))
                    .map(|(n, d)| {
                        (
                            n.clone(),
                            lpp_pm::Req::parse(d.version()).unwrap_or(lpp_pm::Req::Any),
                        )
                    })
                    .collect(),
            })
        })
        .collect()
}

pub fn fetch_all(reg: &Registry, dir: &Path) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    reg.sync().map_err(|e| e.to_string())?;
    let roots = workspace_roots(&ws)?;

    let resolved = lpp_pm::resolve_workspace(&roots, &|name| candidates(reg, name))
        .map_err(|e| e.to_string())?;

    let lock = lpp_pm::Lock::from_resolved(&resolved);
    std::fs::write(
        ws.root.join("Keel.lock"),
        lock.to_toml().map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    let mut b = Builder::default();
    b.push_record(["package".to_string(), "version".to_string()]);
    for p in resolved.packages.values() {
        if p.source == "registry" {
            b.push_record([p.name.clone(), p.version.to_string()]);
        }
    }
    println!(
        "resolved {} package(s) → Keel.lock\n{}",
        resolved.len().saturating_sub(ws.members.len()),
        b.build()
    );
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
            versions.sort_by(|a, b| {
                let av = lpp_pm::Version::parse(&a.version)
                    .unwrap_or_else(|| lpp_pm::Version::new(0, 0, 0));
                let bv = lpp_pm::Version::parse(&b.version)
                    .unwrap_or_else(|| lpp_pm::Version::new(0, 0, 0));
                av.cmp(&bv)
            });
            Ok(lpp_pm::index::IndexEntry {
                name: name.to_string(),
                versions,
            })
        }
    }
}

/// Publish the current project (`./Keel.toml`) to the registry.
pub fn publish(reg: &Registry) -> Result<(), String> {
    // Ensure the registry clone exists first (publish commits into it).
    reg.sync().map_err(|e| e.to_string())?;
    let manifest_path = std::path::Path::new("Keel.toml");
    if !manifest_path.exists() {
        return Err("no Keel.toml in the current directory — run `keel init` first".to_string());
    }
    let manifest = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(manifest_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let name = manifest.name().to_string();
    let version = manifest.version().to_string();

    // Package the current directory into a tar.gz (excluding build + vcs).
    let tmp = std::env::temp_dir().join(format!("keel-publish-{name}-{version}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let artifact_path = tmp.join(format!("{name}-{version}.tar.gz"));
    let tar = std::process::Command::new("tar")
        .args([
            "-czf",
            artifact_path.to_str().unwrap(),
            "--exclude=target",
            "--exclude=.git",
            "-C",
            ".",
            ".",
        ])
        .output()
        .map_err(|e| format!("failed to run `tar`: {e}"))?;
    if !tar.status.success() {
        return Err(format!(
            "tar failed: {}",
            String::from_utf8_lossy(&tar.stderr)
        ));
    }
    let artifact = std::fs::read(&artifact_path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&tmp);

    let checksum = lpp_pm::ContentAddress::of_bytes(&artifact).to_string();
    let deps: Vec<lpp_pm::index::DepSpec> = manifest
        .dependencies
        .iter()
        .map(|(n, d)| lpp_pm::index::DepSpec {
            name: n.clone(),
            req: d.version().to_string(),
            optional: false,
            features: Vec::new(),
        })
        .collect();
    let new_version = lpp_pm::index::VersionEntry {
        version: version.clone(),
        deps,
        features: manifest.features.clone(),
        checksum: checksum.clone(),
        targets: Vec::new(),
        yanked: false,
    };
    // Merge into the existing index entry: published versions are
    // immutable, and a new publish appends rather than clobbering the
    // package's history (clobbering would silently "remove" old versions
    // from every consumer's lockfile).
    let existing = reg.lookup(&name).ok();
    let entry = merge_publish(existing.as_ref(), &name, new_version).map_err(|e| e.to_string())?;

    reg.publish(&entry, &artifact, &format!("publish {name} {version}"))
        .map_err(|e| e.to_string())?;
    reg.push().map_err(|e| e.to_string())?;
    println!(
        "published {name} {version} (sha256 {checksum}) → {}",
        reg.remote()
    );
    Ok(())
}
