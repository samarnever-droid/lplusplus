//! The workspace model: one model for single-repo AND monorepo.
//!
//! A **workspace** is 1..N packages (members) sharing a root + one
//! `Keel.lock`. A single repo is a 1-member workspace; a monorepo is an
//! N-member workspace with path dependencies. See `docs/rewrite/WORKSPACE.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{PmError, Result};
use crate::manifest::{Dependency, Manifest};

/// One workspace member: a package + its directory.
#[derive(Debug, Clone)]
pub struct Member {
    /// Absolute (or as-discovered) directory of the member package.
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// The member's path-dependency target member NAMES (validated at plan
    /// time), in manifest order.
    pub path_deps: Vec<String>,
}

impl Member {
    pub fn name(&self) -> &str {
        self.manifest.name()
    }
}

/// A discovered workspace: root dir + ordered members (root package first,
/// then `[workspace].members` declaration order).
#[derive(Debug, Clone)]
pub struct Workspace {
    /// The workspace root directory (the dir of the root `Keel.toml`).
    pub root: PathBuf,
    /// Members in declaration order (root package first, if any).
    pub members: Vec<Member>,
    /// True when the root manifest has no `[package]` (a virtual workspace).
    pub virtual_root: bool,
}

impl Workspace {
    /// Find the workspace containing `start`.
    ///
    /// Walks UP from `start` to the nearest `Keel.toml` with a `[workspace]`
    /// section; if none exists, `start` itself must be a package manifest
    /// (a 1-member workspace — the single-repo case).
    pub fn discover(start: &Path) -> Result<Workspace> {
        let absolute = if start.is_absolute() {
            start.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| PmError::Io(e.to_string()))?
                .join(start)
        };
        let start = absolute
            .canonicalize()
            .map_err(|e| PmError::Io(format!("cannot resolve {}: {e}", absolute.display())))?;
        let search_start = if start.is_file() {
            start.parent().unwrap_or(&start).to_path_buf()
        } else {
            start.clone()
        };

        // Remember the nearest package in case there is no enclosing
        // workspace. This also lets commands run from `src/` and other
        // package subdirectories.
        let mut nearest_package: Option<PathBuf> = None;
        let mut cur: Option<PathBuf> = Some(search_start.clone());
        while let Some(dir) = cur {
            let manifest_path = dir.join("Keel.toml");
            if manifest_path.is_file() {
                let doc = std::fs::read_to_string(&manifest_path)
                    .map_err(|e| PmError::Io(e.to_string()))?;
                let root: RootManifest =
                    toml::from_str(&doc).map_err(|e| PmError::ManifestParse(e.to_string()))?;
                if root.workspace.is_some() {
                    let ws = Self::load(
                        root,
                        dir.canonicalize().map_err(|e| PmError::Io(e.to_string()))?,
                    )?;
                    let belongs = start == ws.root
                        || ws
                            .members
                            .iter()
                            .any(|member| start == member.dir || start.starts_with(&member.dir));
                    if !belongs {
                        return Err(PmError::MemberNotFound {
                            member: format!(
                                "{} (not a member of the workspace rooted at {})",
                                start.display(),
                                ws.root.display()
                            ),
                        });
                    }
                    return Ok(ws);
                }
                if root.package.is_some() && nearest_package.is_none() {
                    nearest_package = Some(dir.clone());
                }
            }
            cur = dir.parent().map(Path::to_path_buf);
        }

        if let Some(package_dir) = nearest_package {
            let doc = std::fs::read_to_string(package_dir.join("Keel.toml"))
                .map_err(|e| PmError::Io(e.to_string()))?;
            let manifest = Manifest::parse(&doc)?;
            let members = vec![Member {
                dir: package_dir.clone(),
                path_deps: path_dep_names(&manifest),
                manifest,
            }];
            return Ok(Workspace {
                root: package_dir,
                virtual_root: false,
                members,
            });
        }
        Err(PmError::MemberNotFound {
            member: format!("{} (no Keel.toml found up the tree)", start.display()),
        })
    }

    /// Load a workspace from an already-parsed root manifest.
    fn load(root: RootManifest, root_dir: PathBuf) -> Result<Workspace> {
        let mut members: Vec<Member> = Vec::new();
        let virtual_root = root.package.is_none();

        if let Some(pkg) = root.package {
            let manifest = Manifest {
                package: pkg,
                dependencies: root.dependencies.clone(),
                features: root.features.clone(),
                targets: root.targets.clone(),
                workspace: root.workspace.clone(),
            };
            manifest.validate()?;
            members.push(Member {
                dir: root_dir.clone(),
                path_deps: path_dep_names(&manifest).to_vec(),
                manifest,
            });
        }

        for pattern in root
            .workspace
            .as_ref()
            .map(|w| w.members.clone())
            .unwrap_or_default()
        {
            for dir in expand_members(&root_dir, &pattern)? {
                let manifest = load_member(&dir)?;
                members.push(Member {
                    dir,
                    path_deps: path_dep_names(&manifest).to_vec(),
                    manifest,
                });
            }
        }

        if members.is_empty() {
            return Err(PmError::MemberNotFound {
                member: "workspace has no package".to_string(),
            });
        }

        // Duplicate names.
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for m in &members {
            let rel = m
                .dir
                .strip_prefix(&root_dir)
                .unwrap_or(&m.dir)
                .display()
                .to_string();
            let who = if rel.is_empty() { ".".to_string() } else { rel };
            if let Some(first) = seen.get(m.name()) {
                return Err(PmError::DuplicateMember {
                    name: format!("{} ({} and {})", m.name(), first, who),
                });
            }
            seen.insert(m.name().to_string(), who);
        }

        Ok(Workspace {
            virtual_root,
            root: root_dir,
            members,
        })
    }

    /// The topological build plan: LAYERS of member indices (deps first).
    /// Layer `i` may build only after all layers `< i` are done; members
    /// WITHIN a layer are independent and may build concurrently.
    ///
    /// Edges: member → its path-dep members. Cycles → E6016.
    pub fn build_plan(&self) -> Result<Vec<Vec<usize>>> {
        let index: BTreeMap<&str, usize> = self
            .members
            .iter()
            .enumerate()
            .map(|(i, m)| (m.name(), i))
            .collect();

        // out-degree = number of path deps; adjacency dep → dependents.
        let mut indegree: Vec<usize> = vec![0; self.members.len()];
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); self.members.len()];
        for (i, member) in self.members.iter().enumerate() {
            for (dependency_name, dependency) in &member.manifest.dependencies {
                let Some(declared_path) = dependency.path() else {
                    continue;
                };
                let target = member.dir.join(declared_path).canonicalize().map_err(|_| {
                    PmError::PathDepOutsideWorkspace {
                        dep: format!("{dependency_name} ({declared_path})"),
                        from: member.name().to_string(),
                    }
                })?;
                let target_index = self
                    .members
                    .iter()
                    .position(|candidate| candidate.dir == target);
                let Some(target_index) = target_index else {
                    return Err(PmError::PathDepOutsideWorkspace {
                        dep: format!("{dependency_name} ({declared_path})"),
                        from: member.name().to_string(),
                    });
                };
                if self.members[target_index].name() != dependency_name {
                    return Err(PmError::PathDepOutsideWorkspace {
                        dep: format!(
                            "{dependency_name} ({declared_path} points to package '{}')",
                            self.members[target_index].name()
                        ),
                        from: member.name().to_string(),
                    });
                }
                let requirement = crate::validation::requirement(dependency.version())?;
                let target_version =
                    crate::validation::version(self.members[target_index].manifest.version())?;
                if !requirement.matches(&target_version) {
                    return Err(PmError::ResolveConflict {
                        name: dependency_name.clone(),
                        chosen: target_version.to_string(),
                        required: requirement.to_string(),
                    });
                }
                debug_assert_eq!(index.get(dependency_name.as_str()), Some(&target_index));
                indegree[i] += 1;
                dependents[target_index].push(i);
            }
        }

        // Kahn's algorithm, level by level (deterministic: sorted by name).
        let mut ready: Vec<usize> = (0..self.members.len())
            .filter(|&i| indegree[i] == 0)
            .collect();
        let mut plan: Vec<Vec<usize>> = Vec::new();
        let mut placed: usize = 0;
        loop {
            if ready.is_empty() {
                if placed == self.members.len() {
                    break;
                }
                // A cycle: report the remaining members.
                let chain: Vec<String> = (0..self.members.len())
                    .filter(|&i| indegree[i] > 0)
                    .map(|i| self.members[i].name().to_string())
                    .collect();
                return Err(PmError::WorkspaceCycle {
                    chain: chain.join(" -> "),
                });
            }
            ready.sort_by(|&a, &b| self.members[a].name().cmp(self.members[b].name()));
            plan.push(std::mem::take(&mut ready));
            placed += plan.last().unwrap().len();
            for node in plan.last().unwrap().iter() {
                for &dep_of in &dependents[*node] {
                    indegree[dep_of] -= 1;
                    if indegree[dep_of] == 0 {
                        ready.push(dep_of);
                    }
                }
            }
        }
        Ok(plan)
    }

    /// Member indices by name.
    pub fn index(&self) -> BTreeMap<&str, usize> {
        self.members
            .iter()
            .enumerate()
            .map(|(i, m)| (m.name(), i))
            .collect()
    }

    /// The shared output directory: the workspace root's `target/` for
    /// multi-member workspaces, else the single member's own `target/`.
    pub fn out_dir(&self) -> PathBuf {
        if self.members.len() > 1 {
            self.root.join("target")
        } else {
            self.members[0].dir.join("target")
        }
    }
}

/// The root `Keel.toml` shape: `[package]` is optional (virtual workspaces).
#[derive(Debug, Clone, serde::Deserialize, Default)]
struct RootManifest {
    #[serde(default)]
    package: Option<crate::manifest::Package>,
    #[serde(default)]
    dependencies: BTreeMap<String, Dependency>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    targets: Option<crate::manifest::Targets>,
    #[serde(default)]
    workspace: Option<crate::manifest::WorkspaceSection>,
}

fn load_member(dir: &Path) -> Result<Manifest> {
    let p = dir.join("Keel.toml");
    if !p.is_file() {
        return Err(PmError::MemberNotFound {
            member: dir.display().to_string(),
        });
    }
    let doc = std::fs::read_to_string(&p).map_err(|e| PmError::Io(e.to_string()))?;
    Manifest::parse(&doc)
}

/// The path-dependency names declared by a manifest (in manifest order).
fn path_dep_names(m: &Manifest) -> Vec<String> {
    m.dependencies
        .iter()
        .filter_map(|(n, d)| d.path().is_some().then(|| n.clone()))
        .collect()
}

/// Expand a `[workspace].members` pattern to concrete directories.
/// Plain paths match exactly (if they exist); one `*` spans a single
/// segment (`crates/*`). Anything else → E6020.
fn expand_members(root: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(PmError::BadMemberPattern(pattern.to_string()));
    }
    if pattern.contains("**") {
        return Err(PmError::BadMemberPattern(pattern.to_string()));
    }
    let stars = pattern.matches('*').count();
    if stars == 0 {
        let dir = root.join(pattern);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let dir = dir.canonicalize().map_err(|e| PmError::Io(e.to_string()))?;
        if !dir.starts_with(root) {
            return Err(PmError::BadMemberPattern(pattern.to_string()));
        }
        return Ok(vec![dir]);
    }
    if stars > 1 {
        return Err(PmError::BadMemberPattern(pattern.to_string()));
    }
    // Exactly one `*`: prefix/*suffix within one segment.
    let (prefix, suffix) = pattern.split_once('*').unwrap();
    let parent = root.join(prefix);
    if !parent.is_dir() {
        return Ok(Vec::new());
    }
    let parent = parent
        .canonicalize()
        .map_err(|e| PmError::Io(e.to_string()))?;
    if !parent.starts_with(root) {
        return Err(PmError::BadMemberPattern(pattern.to_string()));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for e in std::fs::read_dir(&parent).map_err(|e| PmError::Io(e.to_string()))? {
        let e = e.map_err(|e| PmError::Io(e.to_string()))?;
        if !e.path().is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name.is_empty() {
            continue;
        }
        let suffix = suffix.trim_matches('/');
        let candidate = if suffix.is_empty() {
            parent.join(&name)
        } else {
            parent.join(&name).join(suffix)
        };
        if candidate.is_dir() {
            let candidate = candidate
                .canonicalize()
                .map_err(|e| PmError::Io(e.to_string()))?;
            if !candidate.starts_with(root) {
                return Err(PmError::BadMemberPattern(pattern.to_string()));
            }
            out.push(candidate);
        }
    }
    out.sort();
    Ok(out)
}
