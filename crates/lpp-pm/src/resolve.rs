//! The dependency resolver: turns a manifest's dependency graph (against the
//! registry index) into an exact resolved set, detecting conflicts and cycles.

use std::collections::BTreeMap;

use crate::error::{PmError, Result};
use crate::semver::{Req, Version};

/// A package (a name at one version) with its dependency edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkg {
    pub name: String,
    pub version: Version,
    /// `Some(checksum)` for registry packages; `None` for the root/path deps.
    pub checksum: Option<String>,
    /// `"root"`, `"registry"`, or a path.
    pub source: String,
    pub deps: Vec<(String, Req)>,
}

/// A candidate version of a package, as offered by the registry index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub version: Version,
    pub checksum: Option<String>,
    pub source: String,
    pub deps: Vec<(String, Req)>,
}

/// The fully resolved dependency set (deterministic: keyed by name).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub packages: BTreeMap<String, Pkg>,
}

impl Resolved {
    pub fn get(&self, name: &str) -> Option<&Pkg> {
        self.packages.get(name)
    }
    pub fn len(&self) -> usize {
        self.packages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}

/// Resolve `root`'s full dependency graph.
///
/// `available` returns the candidates for a package by name (or `None` if the
/// package is unknown). Strategy: greedy highest-version-first, with conflict
/// detection (two incompatible requirements for one package) and cycle safety
/// (an already-resolved package is reused, never re-expanded).
pub fn resolve(root: &Pkg, available: &dyn Fn(&str) -> Option<Vec<Candidate>>) -> Result<Resolved> {
    let mut resolved = Resolved::default();
    resolved.packages.insert(root.name.clone(), root.clone());
    let mut worklist: Vec<(String, Req)> = root.deps.clone();

    while let Some((name, req)) = worklist.pop() {
        if let Some(existing) = resolved.packages.get(&name) {
            if !req.matches(&existing.version) {
                return Err(PmError::ResolveConflict {
                    name: name.clone(),
                    chosen: existing.version.to_string(),
                    required: req.to_string(),
                });
            }
            continue;
        }
        let cands = available(&name)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| req.matches(&c.version));
        let pick = cands
            .max_by(|a, b| a.version.cmp(&b.version))
            .ok_or_else(|| PmError::NoMatchingVersion {
                name: name.clone(),
                req: req.to_string(),
            })?;
        worklist.extend(pick.deps.iter().cloned());
        resolved.packages.insert(
            name.clone(),
            Pkg {
                name: name.clone(),
                version: pick.version,
                checksum: pick.checksum,
                source: pick.source,
                deps: pick.deps,
            },
        );
    }
    Ok(resolved)
}

/// Resolve the UNION of several roots' dependency graphs (workspace
/// resolution: one `Keel.lock` for all members).
///
/// This feeds the roots' requirements into a single resolution pass, so
/// compatible requirements across members unify to one version and
/// incompatible ones surface as the usual [`PmError::ResolveConflict`].
pub fn resolve_workspace(
    roots: &[Pkg],
    available: &dyn Fn(&str) -> Option<Vec<Candidate>>,
) -> Result<Resolved> {
    if roots.is_empty() {
        return Ok(Resolved::default());
    }
    let synthetic = Pkg {
        name: "<workspace>".to_string(),
        version: Version::new(0, 0, 0),
        checksum: None,
        source: "workspace".to_string(),
        deps: roots.iter().flat_map(|r| r.deps.iter().cloned()).collect(),
    };
    let mut resolved = resolve(&synthetic, available)?;
    // Replace the synthetic root with the real roots (one entry per member,
    // deterministic: by name).
    let mut by_name: BTreeMap<&str, Pkg> =
        roots.iter().map(|r| (r.name.as_str(), r.clone())).collect();
    let root_names: Vec<String> = roots.iter().map(|r| r.name.clone()).collect();
    resolved.packages.remove("<workspace>");
    for name in root_names {
        if let Some(r) = by_name.remove(name.as_str()) {
            resolved.packages.insert(name, r);
        }
    }
    Ok(resolved)
}
