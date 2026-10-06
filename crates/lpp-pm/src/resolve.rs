//! Backtracking dependency resolution for one package or a workspace.

use std::collections::BTreeMap;

use crate::error::{PmError, Result};
use crate::semver::{Req, Version};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkg {
    pub name: String,
    pub version: Version,
    pub checksum: Option<String>,
    /// `"root"`, `"path"`, or `"registry"`.
    pub source: String,
    pub deps: Vec<(String, Req)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub version: Version,
    pub checksum: Option<String>,
    pub source: String,
    pub deps: Vec<(String, Req)>,
}

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

/// Resolve a package graph using highest-compatible-first backtracking.
///
/// Unlike the earlier greedy loop, a later constraint can force the solver to
/// reconsider an earlier high version and select a lower compatible candidate.
pub fn resolve(root: &Pkg, available: &dyn Fn(&str) -> Option<Vec<Candidate>>) -> Result<Resolved> {
    resolve_roots(std::slice::from_ref(root), available)
}

pub fn resolve_workspace(
    roots: &[Pkg],
    available: &dyn Fn(&str) -> Option<Vec<Candidate>>,
) -> Result<Resolved> {
    resolve_roots(roots, available)
}

fn resolve_roots(
    roots: &[Pkg],
    available: &dyn Fn(&str) -> Option<Vec<Candidate>>,
) -> Result<Resolved> {
    let mut resolved = BTreeMap::new();
    let mut requirements = Vec::new();
    for root in roots {
        if let Some(existing) = resolved.insert(root.name.clone(), root.clone()) {
            return Err(PmError::ResolveConflict {
                name: root.name.clone(),
                chosen: existing.version.to_string(),
                required: format!("workspace root {}", root.version),
            });
        }
        requirements.extend(root.deps.iter().cloned());
    }
    let packages = solve(resolved, requirements, available)?;
    Ok(Resolved { packages })
}

fn solve(
    resolved: BTreeMap<String, Pkg>,
    requirements: Vec<(String, Req)>,
    available: &dyn Fn(&str) -> Option<Vec<Candidate>>,
) -> Result<BTreeMap<String, Pkg>> {
    // Every requirement for an already selected package must remain true.
    for (name, requirement) in &requirements {
        if let Some(existing) = resolved.get(name)
            && !requirement.matches(&existing.version)
        {
            return Err(PmError::ResolveConflict {
                name: name.clone(),
                chosen: existing.version.to_string(),
                required: requirement.to_string(),
            });
        }
    }

    let Some(next_name) = requirements
        .iter()
        .map(|(name, _)| name)
        .find(|name| !resolved.contains_key(name.as_str()))
        .cloned()
    else {
        return Ok(resolved);
    };

    let constraints: Vec<&Req> = requirements
        .iter()
        .filter(|(name, _)| name == &next_name)
        .map(|(_, requirement)| requirement)
        .collect();
    let offered = available(&next_name).unwrap_or_default();
    let mut candidates: Vec<Candidate> = offered
        .iter()
        .filter(|candidate| {
            constraints
                .iter()
                .all(|requirement| requirement.matches(&candidate.version))
        })
        .cloned()
        .collect();
    candidates.sort_by(|left, right| right.version.cmp(&left.version));

    if candidates.is_empty() {
        let required = constraints
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        if constraints.len() > 1 && !offered.is_empty() {
            return Err(PmError::ResolveConflict {
                name: next_name,
                chosen: "no single candidate".to_string(),
                required,
            });
        }
        return Err(PmError::NoMatchingVersion {
            name: next_name,
            req: required,
        });
    }

    let mut last_error = None;
    for candidate in candidates {
        let mut next_resolved = resolved.clone();
        next_resolved.insert(
            next_name.clone(),
            Pkg {
                name: next_name.clone(),
                version: candidate.version,
                checksum: candidate.checksum,
                source: candidate.source,
                deps: candidate.deps.clone(),
            },
        );
        let mut next_requirements = requirements.clone();
        next_requirements.extend(candidate.deps);
        match solve(next_resolved, next_requirements, available) {
            Ok(solution) => return Ok(solution),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| PmError::NoMatchingVersion {
        name: next_name,
        req: "no candidate produced a complete solution".to_string(),
    }))
}
