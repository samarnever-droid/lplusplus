//! In-memory frontend + typechecker + MIR checking pipeline shared across
//! `lpp repl`, `lpp test`, and `lpp-lsp`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_common::{FileId, SourceMap};
use lpp_frontend::parse_source;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, HirPackage, OsFileSystem, PackageSpec,
    ResolutionMode, lower_package,
};
use lpp_mir::{MirBuildOptions, MirProgram, build_mir};
use lpp_types::{ShadowInferenceOptions, ShadowTypeOutput, infer_hir_package};

/// Filesystem overlay that serves in-memory buffers before delegating to OS disk.
pub struct OverlayFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl OverlayFileSystem {
    #[must_use]
    pub fn from_single(path: &Path, source: &str) -> Self {
        let mut files = BTreeMap::new();
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        };
        files.insert(abs, source.to_owned());
        files.insert(path.to_path_buf(), source.to_owned());
        Self { files }
    }

    fn lookup(&self, path: &Path) -> Option<&String> {
        if let Some(s) = self.files.get(path) {
            return Some(s);
        }
        let target = path.to_string_lossy().replace('\\', "/");
        self.files
            .iter()
            .find(|(k, _)| k.to_string_lossy().replace('\\', "/") == target)
            .map(|(_, v)| v)
    }
}

impl FileSystem for OverlayFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        if self.lookup(path).is_some() {
            return Ok(true);
        }
        OsFileSystem.is_file(path)
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.lookup(path).is_some() {
            return Ok(if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path)
            });
        }
        OsFileSystem.canonicalize(path)
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        if let Some(content) = self.lookup(path) {
            return Ok(content.clone());
        }
        OsFileSystem.read_to_string(path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDiagnostic {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub code: String,
    pub message: String,
}

pub struct CheckedUnit {
    pub sources: SourceMap,
    pub package: HirPackage,
    pub inference: ShadowTypeOutput,
    pub mir: MirProgram,
}

fn span_to_range(sources: &SourceMap, span: lpp_common::Span) -> (usize, usize, usize, usize) {
    if let Some(file) = sources.get(span.file) {
        let (sl, sc) = file.line_column(span.start).unwrap_or((1, 1));
        let (el, ec) = file.line_column(span.end).unwrap_or((sl, sc + 1));
        return (sl, sc, el, ec.max(sc + 1));
    }
    (1, 1, 1, 5)
}

/// Run the full front-end -> HIR -> TypeCheck -> MIR pipeline against an
/// in-memory source buffer, returning structured diagnostics on error.
pub fn check_in_memory(
    source: &str,
    virtual_path: &Path,
) -> Result<CheckedUnit, Vec<SourceDiagnostic>> {
    let mut parse_sources = SourceMap::new();
    let fid = parse_sources
        .add_file(virtual_path.display().to_string(), source.to_owned())
        .unwrap_or(FileId::from_raw(1));
    if let Err(diags) = parse_source(fid, source) {
        let mapped = diags
            .into_iter()
            .map(|d| {
                let (line, col, end_line, end_col) = d
                    .primary_span
                    .or_else(|| d.labels.first().map(|l| l.span))
                    .map_or((1, 1, 1, 5), |span| span_to_range(&parse_sources, span));
                SourceDiagnostic {
                    line,
                    col,
                    end_line,
                    end_col,
                    code: d.code.as_str().to_owned(),
                    message: d.message,
                }
            })
            .collect();
        return Err(mapped);
    }

    let overlay = OverlayFileSystem::from_single(virtual_path, source);
    let source_root = virtual_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let package_name = virtual_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("main");
    let mut request = GraphRequest::new(
        virtual_path.to_path_buf(),
        PackageSpec::new(package_name, source_root),
    )
    .with_allow_cycles(true);
    for dep in crate::compile::discover_package_dependencies(virtual_path, package_name) {
        request = request.with_dependency(dep);
    }
    let graph = GraphBuilder::new(&overlay).build(request).map_err(|err| {
        vec![SourceDiagnostic {
            line: 1,
            col: 1,
            end_line: 1,
            end_col: 2,
            code: "E3001".to_owned(),
            message: format!("{err:?}"),
        }]
    })?;

    let package = lower_package(&graph, ResolutionMode::LegacyFlat).map_err(|err| {
        vec![SourceDiagnostic {
            line: 1,
            col: 1,
            end_line: 1,
            end_col: 2,
            code: "E3002".to_owned(),
            message: format!("{err:?}"),
        }]
    })?;

    let work_units = package.expressions.len().saturating_mul(10).max(1_000_000);
    let mut inference = infer_hir_package(
        &package,
        ShadowInferenceOptions {
            work_units,
            ..Default::default()
        },
    )
    .map_err(|err| {
        let span = package.origins[err.origin].span;
        let (line, col, end_line, end_col) = span_to_range(&graph.sources, span);
        vec![SourceDiagnostic {
            line,
            col,
            end_line,
            end_col,
            code: "E3100".to_owned(),
            message: err.to_string(),
        }]
    })?;

    let mir_options = MirBuildOptions {
        max_functions: package.items.len().max(100_000),
        max_aggregates: package.items.len().max(100_000),
        ..Default::default()
    };
    let mir = build_mir(&package, &graph.sources, &mut inference, mir_options).map_err(|err| {
        let span = package.origins[err.origin].span;
        let (line, col, end_line, end_col) = span_to_range(&graph.sources, span);
        vec![SourceDiagnostic {
            line,
            col,
            end_line,
            end_col,
            code: err.code().to_owned(),
            message: err.to_string(),
        }]
    })?;

    Ok(CheckedUnit {
        sources: graph.sources,
        package,
        inference,
        mir,
    })
}
