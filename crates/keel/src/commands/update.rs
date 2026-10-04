//! `keel update [package]` — refresh `Keel.lock` (the `cargo update` analog).
//!
//! Contract: `docs/rewrite/UPDATE_TREE.md`.
//!
//! - `keel update` re-resolves the whole graph to the latest compatible
//!   versions; `keel update <name>` updates only `<name>` and pins every
//!   other locked package to its locked version (exactly one candidate).
//! - Yanked versions are never offered.
//! - Offline (no registry configured): locked packages are offered back at
//!   their locked version (idempotent); new registry deps fail with the
//!   usual resolver error.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use tabled::builder::Builder;

use lpp_pm::registry::Registry;
use lpp_pm::{Candidate, Pkg};

/// All non-yanked candidates for `name` (or `None` if unknown/unreadable).
fn latest_candidates(reg: &Registry, name: &str) -> Option<Vec<Candidate>> {
    reg.lookup(name).ok().map(|entry| {
        entry
            .versions
            .into_iter()
            .filter(|ve| !ve.yanked)
            .map(|ve| Candidate {
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

/// The single non-yanked candidate for `name` at exactly `version`, if
/// the registry still offers it.
fn pinned_candidate(reg: &Registry, name: &str, version: &str) -> Option<Candidate> {
    reg.lookup(name).ok().and_then(|entry| {
        entry
            .versions
            .into_iter()
            .find(|ve| !ve.yanked && ve.version == version)
            .map(|ve| Candidate {
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
    })
}

/// A pinned candidate rebuilt from the lockfile itself (offline mode):
/// the locked version, checksum, and dependency names (requirements
/// relaxed to `Any` — the resolver checks the requirement at each edge).
fn candidate_from_lock(p: &lpp_pm::LockedPkg) -> Candidate {
    Candidate {
        version: lpp_pm::Version::parse(&p.version).unwrap_or(lpp_pm::Version::new(0, 0, 0)),
        checksum: p.checksum.clone(),
        source: p.source.clone(),
        deps: p
            .deps
            .iter()
            .map(|n| (n.clone(), lpp_pm::Req::Any))
            .collect(),
    }
}

/// Names reachable from `roots` through the old lock's dependency edges
/// (BFS; the old lock is the reachability graph we must validate).
fn reachable_in(old: &BTreeMap<String, lpp_pm::LockedPkg>, roots: &[Pkg]) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for r in roots {
        for (n, _) in &r.deps {
            if seen.insert(n.clone()) {
                queue.push_back(n.clone());
            }
        }
    }
    while let Some(n) = queue.pop_front() {
        if let Some(p) = old.get(&n) {
            for dep in &p.deps {
                if seen.insert(dep.clone()) {
                    queue.push_back(dep.clone());
                }
            }
        }
    }
    seen
}

/// The `keel update` implementation. `reg` is `None` when no registry is
/// configured (offline mode). `target` selects single-package mode.
pub fn update(reg: Option<&Registry>, dir: &Path, target: Option<&str>) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    let lock_path = ws.root.join("Keel.lock");

    let old: BTreeMap<String, lpp_pm::LockedPkg> = std::fs::read_to_string(&lock_path)
        .ok()
        .map(|doc| lpp_pm::Lock::parse(&doc).map(|l| l.packages))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.name.clone(), p))
        .collect();

    let roots = super::registry::workspace_roots(&ws)?;
    let member_names: BTreeSet<String> = ws.members.iter().map(|m| m.name().to_string()).collect();

    // Single-package mode against a workspace member: nothing to update.
    if let Some(t) = target {
        if member_names.contains(t) {
            return Err(format!(
                "'{t}' is a workspace member — members are never locked"
            ));
        }
    }

    // Sync the registry once, eagerly, when one is configured.
    let reg = match reg {
        Some(r) => {
            r.sync().map_err(|e| e.to_string())?;
            Some(r)
        }
        None => None,
    };

    // Single-package mode: validate the pinned entries up front (the
    // resolver's candidate provider cannot report errors), and offer the
    // target its full candidate set while every other reachable locked
    // package is pinned to its locked version.
    if let Some(t) = target {
        if reg.is_none() {
            return Err(
                "no registry configured: pass --registry <git-url> or set KEEL_REGISTRY"
                    .to_string(),
            );
        }
        let reg = reg.as_ref().unwrap();
        let reach = reachable_in(&old, &roots);
        for (name, p) in &old {
            if name == t || !reach.contains(name) || p.source != "registry" {
                continue;
            }
            let gone = match reg.lookup(name) {
                Ok(entry) => entry
                    .versions
                    .iter()
                    .find(|v| v.version == p.version && !v.yanked)
                    .is_none(),
                Err(_) => true,
            };
            if gone {
                return Err(lpp_pm::PmError::LockedVersionGone {
                    name: name.to_string(),
                    version: p.version.clone(),
                }
                .to_string());
            }
        }
    }

    // The closure owns its copy of the lock map; the original stays for
    // the diff after the resolver returns.
    let old_for_provider = old.clone();
    let available: Box<dyn Fn(&str) -> Option<Vec<Candidate>>> = Box::new(move |name: &str| {
        let single_pin = match target {
            Some(t) if name != t => old_for_provider
                .get(name)
                .filter(|p| p.source == "registry"),
            _ => None,
        };
        if let Some(pinned) = single_pin {
            // Registry-backed pin (fidelity: real deps + checksum);
            // offline, rebuild from the lockfile itself.
            if let Some(reg) = &reg {
                return pinned_candidate(reg, name, &pinned.version).map(|c| vec![c]);
            }
            return Some(vec![candidate_from_lock(pinned)]);
        }
        match &reg {
            Some(r) => latest_candidates(r, name),
            None => old_for_provider
                .get(name)
                .filter(|p| p.source == "registry")
                .map(|p| vec![candidate_from_lock(p)]),
        }
    });

    let resolved = lpp_pm::resolve_workspace(&roots, &available).map_err(|e| e.to_string())?;
    let new_lock = lpp_pm::Lock::from_resolved(&resolved);
    std::fs::write(&lock_path, new_lock.to_toml().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    // Diff (registry-sourced entries only; members never change).
    let new_map: BTreeMap<&str, &lpp_pm::LockedPkg> = new_lock
        .packages
        .iter()
        .filter(|p| p.source == "registry")
        .map(|p| (p.name.as_str(), p))
        .collect();
    let old_reg: BTreeMap<&str, &lpp_pm::LockedPkg> = old
        .iter()
        .filter(|(_, p)| p.source == "registry")
        .map(|(n, p)| (n.as_str(), p))
        .collect();

    let mut rows: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    let names: BTreeSet<&str> = old_reg.keys().chain(new_map.keys()).copied().collect();
    for name in names {
        let o = old_reg.get(name).map(|p| p.version.clone());
        let n = new_map.get(name).map(|p| p.version.clone());
        if o != n {
            rows.push((name.to_string(), o, n));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    if rows.is_empty() {
        println!("Keel.lock is up to date.");
    } else {
        println!("updated Keel.lock:");
        let mut b = Builder::default();
        b.push_record(["package".to_string(), "old".to_string(), "new".to_string()]);
        for (name, o, n) in rows {
            b.push_record([
                name,
                o.unwrap_or_else(|| "—".to_string()),
                n.unwrap_or_else(|| "—".to_string()),
            ]);
        }
        println!("{}", b.build());
    }
    Ok(())
}
