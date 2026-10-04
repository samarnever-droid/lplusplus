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

/// One status row per locked registry package (sorted by name).
fn outdated_rows(
    reg: &Registry,
    lock: &lpp_pm::Lock,
    filter: Option<&str>,
) -> Vec<(String, String, String, String)> {
    let mut rows: Vec<(String, String, String, String)> = Vec::new();
    for p in locked_registry_names(lock)
        .into_iter()
        .filter(|p| filter.map(|f| f == p.name).unwrap_or(true))
    {
        let (status, latest) = match reg.lookup(&p.name) {
            Err(_) => ("removed".to_string(), "—".to_string()),
            Ok(entry) => {
                let locked_yanked = entry
                    .versions
                    .iter()
                    .find(|v| v.version == p.version)
                    .map(|v| v.yanked)
                    .unwrap_or(true); // version gone from the index
                if locked_yanked {
                    ("yanked".to_string(), "—".to_string())
                } else {
                    let latest = entry
                        .versions
                        .iter()
                        .filter(|v| !v.yanked)
                        .max_by(|a, b| {
                            let av = lpp_pm::Version::parse(&a.version)
                                .unwrap_or_else(|| lpp_pm::Version::new(0, 0, 0));
                            let bv = lpp_pm::Version::parse(&b.version)
                                .unwrap_or_else(|| lpp_pm::Version::new(0, 0, 0));
                            av.cmp(&bv)
                        })
                        .map(|v| v.version.clone())
                        .expect("locked version is non-yanked, so the set is non-empty");
                    if latest == p.version {
                        ("up to date".to_string(), latest)
                    } else {
                        ("update available".to_string(), latest)
                    }
                }
            }
        };
        rows.push((p.name.clone(), p.version.clone(), latest, status));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

/// Build the full `keel outdated` output. `reg` is `None` offline.
pub fn outdated_render(
    reg: Option<&Registry>,
    dir: &Path,
    filter: Option<&str>,
) -> Result<String, String> {
    let (_ws, lock) = load_lock(dir)?;

    if let Some(f) = filter {
        if !locked_registry_names(&lock).into_iter().any(|p| p.name == f) {
            return Err(lpp_pm::PmError::NotInLockFile(f.to_string()).to_string());
        }
    }
    if locked_registry_names(&lock).is_empty() {
        return Ok("nothing to check: no registry packages in Keel.lock\n".to_string());
    }

    let reg = match reg {
        Some(r) => {
            r.sync().map_err(|e| e.to_string())?;
            r
        }
        None => {
            return Err(
                "no registry configured: pass --registry <git-url> or set KEEL_REGISTRY"
                    .to_string(),
            )
        }
    };

    let rows = outdated_rows(reg, &lock, filter);
    let mut b = Builder::default();
    b.push_record([
        "package".to_string(),
        "current".to_string(),
        "latest".to_string(),
        "status".to_string(),
    ]);
    let mut interesting = 0usize;
    for (name, current, latest, status) in &rows {
        if status != "up to date" {
            interesting += 1;
        }
        b.push_record([name.clone(), current.clone(), latest.clone(), status.clone()]);
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
    match reg.fetch(name, version) {
        Ok((bytes, _entry)) => {
            let actual = lpp_pm::ContentAddress::of_bytes(&bytes).to_string();
            if actual == locked {
                "ok"
            } else {
                "mismatch"
            }
        }
        Err(lpp_pm::PmError::PackageNotFound(_)) => "removed",
        Err(_) => "missing",
    }
}

/// Build the full `keel verify` output. Returns `Ok(None)` when everything
/// verified, `Ok(Some(summary))` when not (caller exits non-zero).
pub fn verify_render(
    reg: &Registry,
    dir: &Path,
) -> Result<(String, bool), String> {
    let (_ws, lock) = load_lock(dir)?;
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
        rows.push((p.name.clone(), p.version.clone(), checksum[..12.min(checksum.len())].to_string(), status));
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
        b.push_record([name.clone(), version.clone(), short.clone(), status.to_string()]);
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
        text.push_str(&format!("({skipped} path/member package(s) skipped — not content-addressed)\n"));
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
fn chains_to(
    ws: &lpp_pm::Workspace,
    lock: &lpp_pm::Lock,
    target: &str,
) -> Vec<Vec<String>> {
    let by_name: std::collections::BTreeMap<&str, &lpp_pm::LockedPkg> = lock
        .packages
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
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
    println!("why: {} v{} ({})", target.name, target.version, target.source);
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
    if chains.len() > MAX_CHAINS {
        println!("… and {} more", chains.len() - MAX_CHAINS);
    }
    Ok(())
}
