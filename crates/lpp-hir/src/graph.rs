use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use lpp_common::{Diagnostic, FileId, SourceMap, Span};
use lpp_frontend::{ImportKind, ModulePath, ParsedModule, parse_shared};

use crate::fs::{FileSystem, FileSystemError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(u32);

impl PackageId {
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(u32);

impl ModuleId {
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSpec {
    pub name: String,
    pub source_root: PathBuf,
}

impl PackageSpec {
    #[must_use]
    pub fn new(name: impl Into<String>, source_root: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            source_root: source_root.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRequest {
    pub entry: PathBuf,
    pub package: PackageSpec,
    pub dependencies: Vec<PackageSpec>,
}

impl GraphRequest {
    #[must_use]
    pub fn new(entry: impl Into<PathBuf>, package: PackageSpec) -> Self {
        Self {
            entry: entry.into(),
            package,
            dependencies: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_dependency(mut self, dependency: PackageSpec) -> Self {
        self.dependencies.push(dependency);
        self
    }
}

#[derive(Debug)]
pub struct PackageGraph {
    pub entry: ModuleId,
    pub packages: Vec<Package>,
    pub modules: Vec<Module>,
    pub edges: Vec<ImportEdge>,
    pub sources: SourceMap,
}

impl PackageGraph {
    #[must_use]
    pub fn module(&self, id: ModuleId) -> Option<&Module> {
        self.modules.get(id.0 as usize)
    }

    pub fn dependencies(&self, id: ModuleId) -> impl Iterator<Item = &ImportEdge> {
        self.edges.iter().filter(move |edge| edge.importer == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: PackageId,
    pub name: String,
    pub source_root: PathBuf,
}

#[derive(Debug)]
pub struct Module {
    pub id: ModuleId,
    pub package: PackageId,
    pub path: PathBuf,
    pub file: FileId,
    pub syntax: ParsedModule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEdge {
    pub importer: ModuleId,
    pub imported: ModuleId,
    pub path: ModulePath,
    pub kind: ImportKind,
    pub span: Span,
}

pub struct GraphBuilder<'fs, Fs: ?Sized> {
    filesystem: &'fs Fs,
}

impl<'fs, Fs: FileSystem + ?Sized> GraphBuilder<'fs, Fs> {
    #[must_use]
    pub const fn new(filesystem: &'fs Fs) -> Self {
        Self { filesystem }
    }

    pub fn build(&self, request: GraphRequest) -> Result<PackageGraph, GraphError> {
        let packages = self.prepare_packages(request.package, request.dependencies)?;
        if !self.filesystem.is_file(&request.entry)? {
            return Err(GraphError::MissingEntry(request.entry));
        }
        let entry_name = request.entry.file_name().map(ToOwned::to_owned);
        let entry_path = self.filesystem.canonicalize(&request.entry)?;
        if entry_path.file_name() != entry_name.as_deref() {
            return Err(GraphError::CaseMismatch {
                requested: request.entry,
                actual: entry_path,
            });
        }

        let mut pending = BTreeSet::from([(entry_path.clone(), PackageId(0))]);
        let mut discovered: BTreeMap<PathBuf, DiscoveredModule> = BTreeMap::new();
        while let Some((path, package)) = pending.pop_first() {
            if discovered.contains_key(&path) {
                continue;
            }
            let source = self.filesystem.read_to_string(&path)?.into();
            let parsed = parse_shared(FileId::from_raw(0), source).map_err(|diagnostics| {
                GraphError::Frontend {
                    path: path.clone(),
                    diagnostics,
                }
            })?;
            let mut resolved = Vec::with_capacity(parsed.imports.len());
            for import in &parsed.imports {
                let target = self.resolve_import(&packages, package, &path, &import.path)?;
                pending.insert((target.path.clone(), target.package));
                resolved.push(target);
            }
            discovered.insert(
                path,
                DiscoveredModule {
                    package,
                    syntax: parsed,
                    resolved,
                },
            );
        }

        let path_to_id: BTreeMap<_, _> = discovered
            .keys()
            .enumerate()
            .map(|(index, path)| {
                (
                    path.clone(),
                    ModuleId(u32::try_from(index).expect("module count fits in u32")),
                )
            })
            .collect();
        let mut sources = SourceMap::new();
        let mut modules = Vec::with_capacity(discovered.len());
        let mut edges = Vec::new();
        for (path, mut discovered_module) in discovered {
            let file = sources
                .add_file(
                    path.to_string_lossy().into_owned(),
                    discovered_module.syntax.source.clone(),
                )
                .map_err(|error| GraphError::Source(error.to_string()))?;
            discovered_module.syntax.remap_file(file);
            let id = path_to_id[&path];
            for (import, target) in discovered_module
                .syntax
                .imports
                .iter()
                .zip(&discovered_module.resolved)
            {
                edges.push(ImportEdge {
                    importer: id,
                    imported: path_to_id[&target.path],
                    path: import.path.clone(),
                    kind: import.kind.clone(),
                    span: import.span,
                });
            }
            modules.push(Module {
                id,
                package: discovered_module.package,
                path,
                file,
                syntax: discovered_module.syntax,
            });
        }
        edges.sort_by(|left, right| {
            (left.importer, left.imported, &left.path).cmp(&(
                right.importer,
                right.imported,
                &right.path,
            ))
        });

        if let Some(cycle) = find_cycle(modules.len(), &edges) {
            return Err(GraphError::ImportCycle(
                cycle
                    .into_iter()
                    .map(|id| modules[id.0 as usize].path.clone())
                    .collect(),
            ));
        }

        Ok(PackageGraph {
            entry: path_to_id[&entry_path],
            packages,
            modules,
            edges,
            sources,
        })
    }

    fn prepare_packages(
        &self,
        package: PackageSpec,
        mut dependencies: Vec<PackageSpec>,
    ) -> Result<Vec<Package>, GraphError> {
        if package.name.is_empty()
            || dependencies
                .iter()
                .any(|dependency| dependency.name.is_empty())
        {
            return Err(GraphError::InvalidPackageName);
        }
        dependencies.sort_by(|left, right| {
            (&left.name, &left.source_root).cmp(&(&right.name, &right.source_root))
        });
        let mut seen = BTreeSet::from([package.name.clone()]);
        for dependency in &dependencies {
            if !seen.insert(dependency.name.clone()) {
                return Err(GraphError::DuplicatePackage(dependency.name.clone()));
            }
        }

        std::iter::once(package)
            .chain(dependencies)
            .enumerate()
            .map(|(index, package)| {
                Ok(Package {
                    id: PackageId(u32::try_from(index).expect("package count fits in u32")),
                    name: package.name,
                    source_root: self.filesystem.canonicalize(&package.source_root)?,
                })
            })
            .collect()
    }

    fn resolve_import(
        &self,
        packages: &[Package],
        importer_package: PackageId,
        importer: &Path,
        module: &ModulePath,
    ) -> Result<ResolvedModule, GraphError> {
        let relative = module_file_path(module);
        let importer_directory = importer.parent().unwrap_or_else(|| Path::new("."));
        let current_package = &packages[importer_package.0 as usize];
        let local_candidates = [
            (importer_directory.join(&relative), importer_package),
            (
                current_package.source_root.join(&relative),
                importer_package,
            ),
        ];
        let mut searched = Vec::new();
        let mut seen = BTreeSet::new();
        for (candidate, package) in local_candidates {
            if seen.insert(candidate.clone()) {
                searched.push(candidate.clone());
                if let Some(path) = self.exact_file(&candidate, &relative)? {
                    return Ok(ResolvedModule { path, package });
                }
            }
        }

        let mut dependency_matches = BTreeMap::new();
        for package in packages.iter().skip(1) {
            let mut relatives = vec![relative.clone()];
            if module.components().first().map(String::as_str) == Some(package.name.as_str())
                && module.components().len() > 1
            {
                relatives.push(module_file_path_components(&module.components()[1..]));
            }
            for dependency_relative in relatives {
                let candidate = package.source_root.join(&dependency_relative);
                if !seen.insert(candidate.clone()) {
                    continue;
                }
                searched.push(candidate.clone());
                if let Some(path) = self.exact_file(&candidate, &dependency_relative)? {
                    dependency_matches.insert(path, package.id);
                }
            }
        }
        match dependency_matches.len() {
            0 => Err(GraphError::MissingModule {
                importer: importer.to_owned(),
                module: module.clone(),
                searched,
            }),
            1 => {
                let (path, package) = dependency_matches
                    .into_iter()
                    .next()
                    .expect("one dependency match");
                Ok(ResolvedModule { path, package })
            }
            _ => Err(GraphError::AmbiguousModule {
                importer: importer.to_owned(),
                module: module.clone(),
                matches: dependency_matches.into_keys().collect(),
            }),
        }
    }

    fn exact_file(
        &self,
        candidate: &Path,
        requested_suffix: &Path,
    ) -> Result<Option<PathBuf>, GraphError> {
        if !self.filesystem.is_file(candidate)? {
            return Ok(None);
        }
        let canonical = self.filesystem.canonicalize(candidate)?;
        if !canonical.ends_with(requested_suffix) {
            return Err(GraphError::CaseMismatch {
                requested: candidate.to_owned(),
                actual: canonical,
            });
        }
        Ok(Some(canonical))
    }
}

#[derive(Debug)]
struct DiscoveredModule {
    package: PackageId,
    syntax: ParsedModule,
    resolved: Vec<ResolvedModule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedModule {
    path: PathBuf,
    package: PackageId,
}

fn module_file_path(module: &ModulePath) -> PathBuf {
    module_file_path_components(module.components())
}

fn module_file_path_components(components: &[String]) -> PathBuf {
    let mut path = PathBuf::new();
    for component in components {
        path.push(component);
    }
    path.set_extension("lpp");
    path
}

fn find_cycle(module_count: usize, edges: &[ImportEdge]) -> Option<Vec<ModuleId>> {
    let mut adjacency = vec![Vec::new(); module_count];
    for edge in edges {
        adjacency[edge.importer.0 as usize].push(edge.imported);
    }
    for targets in &mut adjacency {
        targets.sort_unstable();
        targets.dedup();
    }
    let mut states = vec![VisitState::Unvisited; module_count];
    let mut stack = Vec::new();
    for raw in 0..module_count {
        let module = ModuleId(u32::try_from(raw).expect("module count fits in u32"));
        if states[raw] == VisitState::Unvisited
            && let Some(cycle) = visit(module, &adjacency, &mut states, &mut stack)
        {
            return Some(cycle);
        }
    }
    None
}

fn visit(
    module: ModuleId,
    adjacency: &[Vec<ModuleId>],
    states: &mut [VisitState],
    stack: &mut Vec<ModuleId>,
) -> Option<Vec<ModuleId>> {
    states[module.0 as usize] = VisitState::Visiting;
    stack.push(module);
    for target in &adjacency[module.0 as usize] {
        match states[target.0 as usize] {
            VisitState::Unvisited => {
                if let Some(cycle) = visit(*target, adjacency, states, stack) {
                    return Some(cycle);
                }
            }
            VisitState::Visiting => {
                let start = stack
                    .iter()
                    .position(|candidate| candidate == target)
                    .expect("visiting target is on the DFS stack");
                let mut cycle = stack[start..].to_vec();
                cycle.push(*target);
                return Some(cycle);
            }
            VisitState::Visited => {}
        }
    }
    stack.pop();
    states[module.0 as usize] = VisitState::Visited;
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Unvisited,
    Visiting,
    Visited,
}

#[derive(Debug)]
pub enum GraphError {
    FileSystem(FileSystemError),
    MissingEntry(PathBuf),
    InvalidPackageName,
    DuplicatePackage(String),
    Frontend {
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    MissingModule {
        importer: PathBuf,
        module: ModulePath,
        searched: Vec<PathBuf>,
    },
    AmbiguousModule {
        importer: PathBuf,
        module: ModulePath,
        matches: Vec<PathBuf>,
    },
    CaseMismatch {
        requested: PathBuf,
        actual: PathBuf,
    },
    ImportCycle(Vec<PathBuf>),
    Source(String),
}

impl From<FileSystemError> for GraphError {
    fn from(error: FileSystemError) -> Self {
        Self::FileSystem(error)
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileSystem(error) => error.fmt(formatter),
            Self::MissingEntry(path) => {
                write!(formatter, "entry module '{}' is not a file", path.display())
            }
            Self::InvalidPackageName => formatter.write_str("package names cannot be empty"),
            Self::DuplicatePackage(name) => write!(formatter, "duplicate package name '{name}'"),
            Self::Frontend { path, diagnostics } => {
                let diagnostic = diagnostics.first();
                write!(formatter, "frontend rejected '{}'", path.display())?;
                if let Some(diagnostic) = diagnostic {
                    write!(formatter, ": {} {}", diagnostic.code, diagnostic.message)?;
                }
                Ok(())
            }
            Self::MissingModule {
                importer, module, ..
            } => write!(
                formatter,
                "module '{}' imported by '{}' was not found with exact casing",
                module.components().join("."),
                importer.display()
            ),
            Self::AmbiguousModule {
                importer,
                module,
                matches,
            } => write!(
                formatter,
                "module '{}' imported by '{}' is ambiguous across {} files",
                module.components().join("."),
                importer.display(),
                matches.len()
            ),
            Self::CaseMismatch { requested, actual } => write!(
                formatter,
                "module path casing mismatch: requested '{}', actual '{}'",
                requested.display(),
                actual.display()
            ),
            Self::ImportCycle(paths) => write!(
                formatter,
                "import cycle: {}",
                paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            Self::Source(message) => write!(formatter, "source map error: {message}"),
        }
    }
}

impl std::error::Error for GraphError {}
