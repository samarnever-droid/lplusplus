//! `keel outdated` + `keel why` — read-only diagnostics (the "most loved"
//! command tier).
//!
//! Contract: `docs/rewrite/DIAGNOSTICS.md`.

use std::path::Path;

use tabled::builder::Builder;

use lpp_pm::registry::Registry;

const MAX_CHAINS: usize = 32;

/// Discover the workspace and load its `Keel.lock` (E6023 when absent).
fn load_lock(dir: &Path) -> Result<(lpp_pm::Workspace, lpp_pm::Lock), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    let doc = std::fs::read_to_string(ws.root.join("Keel.lock"))
        .map_err(|_| lpp_pm::PmError::NoLockFile.to_string())?;
    let lock = lpp_pm::Lock::parse(&doc).map_err(|e| e.to_string())?;
    Ok((ws, lock))
}

// ---------------------------------------------------------------- outdated

fn locked_registry_names(lock: &lpp_pm::Lock) -> Vec<&lpp_pm::LockedPkg> {
    lock.packages
        .iter()
        .filter(|p| p.source == "registry")
        .collect()
}

/// One status row per locked registry package (sorted by name). The reported
/// candidate must satisfy every requirement recorded in the lock graph; the
/// absolute registry tip is not mislabeled as an installable update.
fn outdated_rows(
    reg: &Registry,
    lock: &lpp_pm::Lock,
    filter: Option<&str>,
) -> Result<Vec<(String, String, String, String)>, String> {
    let mut rows: Vec<(String, String, String, String)> = Vec::new();
    for package in locked_registry_names(lock)
        .into_iter()
        .filter(|package| filter.map(|name| name == package.name).unwrap_or(true))
    {
        let (status, compatible) = match reg.lookup(&package.name) {
            Err(lpp_pm::PmError::PackageNotFound(_)) => ("removed".to_string(), "—".to_string()),
            Err(error) => return Err(error.to_string()),
            Ok(entry) => {
                let requirements = lock
                    .packages
                    .iter()
                    .filter_map(|dependent| {
                        dependent
                            .deps
                            .iter()
                            .find(|dependency| *dependency == &package.name)
                            .map(|dependency| {
                                dependent
                                    .dep_reqs
                                    .get(dependency)
                                    .map(String::as_str)
                                    .unwrap_or("*")
                            })
                    })
                    .map(lpp_pm::validation::requirement)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                let mut available = entry
                    .versions
                    .iter()
                    .filter(|version| !version.yanked)
                    .map(|version| {
                        lpp_pm::validation::version(&version.version)
                            .map(|parsed| (parsed, version.version.clone()))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                available.sort_by(|left, right| right.0.cmp(&left.0));
                let absolute_latest = available.first().map(|(_, text)| text.clone());
                let compatible = available
                    .iter()
                    .find(|(version, _)| {
                        requirements
                            .iter()
                            .all(|requirement| requirement.matches(version))
                    })
                    .map(|(_, text)| text.clone());
                let locked = entry
                    .versions
                    .iter()
                    .find(|version| version.version == package.version);
                let status = match locked {
                    None => "locked version removed".to_string(),
                    Some(version) if version.yanked => "yanked".to_string(),
                    Some(_) if compatible.as_deref() != Some(package.version.as_str()) => {
                        if compatible.is_some() {
                            "update available".to_string()
                        } else {
                            "no compatible version".to_string()
                        }
                    }
                    Some(_) if absolute_latest.as_deref() != Some(package.version.as_str()) => {
                        format!(
                            "up to date (latest {} is incompatible)",
                            absolute_latest.as_deref().unwrap_or("—")
                        )
                    }
                    Some(_) => "up to date".to_string(),
                };
                (status, compatible.unwrap_or_else(|| "—".to_string()))
            }
        };
        rows.push((
            package.name.clone(),
            package.version.clone(),
            compatible,
            status,
        ));
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rows)
}

/// Build the full `keel outdated` output. `reg` is `None` offline.
pub fn outdated_render(
    reg: Option<&Registry>,
    dir: &Path,
    filter: Option<&str>,
) -> Result<String, String> {
    let (_ws, lock) = load_lock(dir)?;

    if let Some(f) = filter {
        if !locked_registry_names(&lock)
            .into_iter()
            .any(|p| p.name == f)
        {
            return Err(lpp_pm::PmError::NotInLockFile(f.to_string()).to_string());
        }
    }
    if locked_registry_names(&lock).is_empty() {
        return Ok("nothing to check: no registry packages in Keel.lock\n".to_string());
    }

    let reg = match reg {
        Some(registry) => {
            if let Some(identity) = &lock.registry
                && identity != registry.remote()
            {
                return Err(format!(
                    "Keel.lock belongs to registry '{identity}', but '{}' was requested",
                    registry.remote()
                ));
            }
            registry.sync().map_err(|e| e.to_string())?;
            registry
        }
        None => {
            return Err(
                "no registry configured: pass --registry <git-url> or set KEEL_REGISTRY"
                    .to_string(),
            );
        }
    };

    let rows = outdated_rows(reg, &lock, filter)?;
    let mut b = Builder::default();
    b.push_record([
        "package".to_string(),
        "current".to_string(),
        "compatible".to_string(),
        "status".to_string(),
    ]);
    let mut interesting = 0usize;
    for (name, current, latest, status) in &rows {
        if !status.starts_with("up to date") {
            interesting += 1;
        }
        b.push_record([
            name.clone(),
            current.clone(),
            latest.clone(),
            status.clone(),
        ]);
    }
    let mut text = b.build().to_string();
    text.push('\n');
    if interesting > 0 {
        text.push_str(&format!(
            "{} package(s) need attention — run `keel update`\n",
            interesting
        ));
    }
    Ok(text)
}

/// The `keel outdated` implementation (render + print).
pub fn outdated(reg: Option<&Registry>, dir: &Path, filter: Option<&str>) -> Result<(), String> {
    let text = outdated_render(reg, dir, filter)?;
    print!("{text}");
    Ok(())
}

// ---------------------------------------------------------------- verify

/// Verify one locked registry package against the registry: fetch (which
/// verifies artifact == *index* checksum), then re-hash against the
/// **locked** checksum — the lockfile is the source of truth.
fn verify_one(reg: &Registry, name: &str, version: &str, locked: &str) -> &'static str {
    match reg.fetch_locked(name, version) {
        Ok((bytes, _entry)) => {
            let actual = lpp_pm::ContentAddress::of_bytes(&bytes).to_string();
            if actual == locked { "ok" } else { "mismatch" }
        }
        Err(lpp_pm::PmError::PackageNotFound(_)) => "removed",
        Err(lpp_pm::PmError::ChecksumMismatch { .. }) => "mismatch",
        Err(_) => "missing",
    }
}

/// Build the full `keel verify` output. Returns `Ok(None)` when everything
/// verified, `Ok(Some(summary))` when not (caller exits non-zero).
pub fn verify_render(reg: &Registry, dir: &Path) -> Result<(String, bool), String> {
    let (_ws, lock) = load_lock(dir)?;
    if let Some(identity) = &lock.registry
        && identity != reg.remote()
    {
        return Err(format!(
            "Keel.lock belongs to registry '{identity}', but '{}' was requested",
            reg.remote()
        ));
    }
    reg.sync().map_err(|e| e.to_string())?;

    let registry_pkgs: Vec<&lpp_pm::LockedPkg> = lock
        .packages
        .iter()
        .filter(|p| p.source == "registry" && p.checksum.is_some())
        .collect();
    let skipped = lock.packages.len() - registry_pkgs.len();

    let mut rows: Vec<(String, String, String, &str)> = Vec::new();
    let mut failed = 0usize;
    for p in &registry_pkgs {
        let checksum = p.checksum.as_deref().unwrap();
        let status = verify_one(reg, &p.name, &p.version, checksum);
        if status != "ok" {
            failed += 1;
        }
        rows.push((
            p.name.clone(),
            p.version.clone(),
            checksum[..12.min(checksum.len())].to_string(),
            status,
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut b = Builder::default();
    b.push_record([
        "package".to_string(),
        "version".to_string(),
        "sha256".to_string(),
        "status".to_string(),
    ]);
    for (name, version, short, status) in &rows {
        b.push_record([
            name.clone(),
            version.clone(),
            short.clone(),
            status.to_string(),
        ]);
    }
    let mut text = b.build().to_string();
    text.push('\n');
    if rows.is_empty() {
        text.push_str("nothing to verify: no registry packages in Keel.lock\n");
    } else if failed == 0 {
        text.push_str(&format!("{} package(s) verified OK\n", rows.len()));
    } else {
        text.push_str(&format!(
            "{} of {} package(s) FAILED verification\n",
            failed,
            rows.len()
        ));
    }
    if skipped > 0 {
        text.push_str(&format!(
            "({skipped} path/member package(s) skipped — not content-addressed)\n"
        ));
    }
    Ok((text, failed == 0))
}

/// The `keel verify` implementation (render + print; non-zero on failure —
/// CI-friendly).
pub fn verify(reg: &Registry, dir: &Path) -> Result<(), String> {
    let (text, ok) = verify_render(reg, dir)?;
    print!("{text}");
    if !ok {
        return Err("verification failed — inspect the table above".to_string());
    }
    Ok(())
}

// -------------------------------------------------------------------- why

/// DFS all distinct chains from `start` to `target` through the lock's
/// dependency edges (deterministic declaration order; cycle-safe; capped).
fn dfs_chains(
    path: &mut Vec<String>,
    by_name: &std::collections::BTreeMap<&str, &lpp_pm::LockedPkg>,
    target: &str,
    out: &mut Vec<Vec<String>>,
) {
    let last = path.last().unwrap().clone();
    if last == target {
        out.push(path.clone());
        return;
    }
    for d in by_name
        .get(last.as_str())
        .map(|p| p.deps.clone())
        .unwrap_or_default()
    {
        if out.len() >= MAX_CHAINS {
            return;
        }
        if !path.contains(&d) {
            path.push(d);
            dfs_chains(path, by_name, target, out);
            path.pop();
        }
    }
}

/// Collect distinct chains from workspace members to `target`.
fn chains_to(ws: &lpp_pm::Workspace, lock: &lpp_pm::Lock, target: &str) -> Vec<Vec<String>> {
    let by_name: std::collections::BTreeMap<&str, &lpp_pm::LockedPkg> =
        lock.packages.iter().map(|p| (p.name.as_str(), p)).collect();
    let mut out: Vec<Vec<String>> = Vec::new();
    for m in &ws.members {
        if out.len() >= MAX_CHAINS {
            break;
        }
        let mut path = vec![m.name().to_string()];
        dfs_chains(&mut path, &by_name, target, &mut out);
    }
    out
}

/// The `keel why` implementation (offline: lock arithmetic only).
pub fn why(dir: &Path, name: &str) -> Result<(), String> {
    let (ws, lock) = load_lock(dir)?;
    let target = lock
        .packages
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| lpp_pm::PmError::NotInLockFile(name.to_string()).to_string())?;
    let chains = chains_to(&ws, &lock, name);
    println!(
        "why: {} v{} ({})",
        target.name, target.version, target.source
    );
    if chains.is_empty() {
        println!("(no chain from any member — stale lock entry; run `keel fetch`)");
        return Ok(());
    }
    let shown = chains.len().min(MAX_CHAINS);
    for c in &chains[..shown] {
        // c = [member, ..., direct_parent_of_target]; render target-first.
        let mut parts = vec![name.to_string()];
        for step in c.iter().rev().skip(1) {
            parts.push(step.clone());
        }
        let mut line = parts.join(" ← ");
        if c.len() == 2 {
            line.push_str(" (direct)");
        }
        println!("{line}");
    }
    if chains.len() == MAX_CHAINS {
        println!("… additional dependency chains may be omitted (limit {MAX_CHAINS})");
    }
    Ok(())
}
