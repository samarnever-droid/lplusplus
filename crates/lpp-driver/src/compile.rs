//! The real compile pipeline behind the driver contract (Phase 7).
//!
//! This is the engine the Phase 1 `CompilerEngine` contract was always meant
//! to grow into. It wires the pipeline that the backend gates proved, stage by
//! stage, into a reusable library:
//!
//! ```text
//! .lpp entry file
//!   -> OsFileSystem dependency graph        (lpp-hir GraphBuilder)
//!   -> HIR                                  (lower_package)
//!   -> typed HIR                            (infer_hir_package)
//!   -> MIR                                  (build_mir)
//!   -> object                               (a Backend: Cranelift / WASM / LLVM)
//!   -> executable                           (link against the Rust runtime)
//! ```
//!
//! The final link uses the **Rust runtime cdylib** (`liblpp_runtime.so`), the
//! same artifact the 6B.3e drop-in capstone proved is a byte-identical
//! replacement for the v1 C `c_shim.c`. Native modules also pass their declared
//! external libraries through to the host linker.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{
    Backend, CodegenError, CodegenOptions, CompiledModule, DebugSourceResolver, NameResolver,
    OptLevel, Target,
};
use lpp_codegen_cranelift::CraneliftBackend;
use lpp_codegen_llvm::LlvmBackend;
use lpp_codegen_wasm::WasmBackend;
use lpp_common::OptimizationLevel;
use lpp_hir::{
    GraphBuilder, GraphRequest, OsFileSystem, PackageSpec, ResolutionMode, StringInterner, Symbol,
    lower_package,
};
use lpp_mir::{MirBuildOptions, MirFunctionId, build_mir, mir_snapshot, verify_mir};
use lpp_ownership::{compute_ownership_plan, verify_ownership_balance, verify_ownership_plan};
use lpp_passes::run_optimization_with_profile;
use lpp_types::{ShadowInferenceOptions, infer_hir_package};

/// Adapts the HIR string interner to the codegen `NameResolver` contract.
struct Names<'a>(&'a StringInterner);

impl NameResolver for Names<'_> {
    fn resolve(&self, symbol_raw: u32) -> Option<&str> {
        self.0.resolve(Symbol::from_raw(symbol_raw))
    }
}

/// Which code generator drives the back half of the pipeline. All three
/// backends are real; Cranelift is the complete native one, so it is the
/// default for `X86_64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    /// The native Cranelift backend (full data surface).
    Cranelift,
    /// The `wasm32-wasi` backend.
    Wasm,
    /// The native LLVM-IR backend (scalar + float surface today).
    Llvm,
}

impl BackendChoice {
    fn codegen(self) -> &'static dyn Backend {
        match self {
            Self::Cranelift => &CraneliftBackend,
            Self::Wasm => &WasmBackend,
            Self::Llvm => &LlvmBackend,
        }
    }

    /// The target selected when the caller did not pass `--target`.
    #[must_use]
    pub fn default_target(self) -> Target {
        match self {
            Self::Wasm => Target::Wasm32Wasi,
            Self::Cranelift | Self::Llvm => Target::host(),
        }
    }

    /// Parse a `--backend` value.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "cranelift" => Some(Self::Cranelift),
            "wasm" => Some(Self::Wasm),
            "llvm" => Some(Self::Llvm),
            _ => None,
        }
    }
}

/// Maps HIR `OriginId` indices to `(directory, filename, line, column)` for DWARF emission.
struct OriginDebugResolver {
    origins: Vec<Option<(usize, u32, u32)>>,
    files: Vec<(String, String)>,
}

impl OriginDebugResolver {
    fn from_package(package: &lpp_hir::HirPackage, sources: &lpp_common::SourceMap) -> Self {
        let mut files = Vec::new();
        let mut file_index_by_id = BTreeMap::new();
        let mut origins = Vec::new();

        for (_, origin) in package.origins.enumerate() {
            let span = origin.span;
            let mapped = sources.get(span.file).map(|file| {
                let slot = *file_index_by_id.entry(span.file.raw()).or_insert_with(|| {
                    let path = Path::new(file.name());
                    let dir = path
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| ".".to_string());
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| file.name().to_string());
                    let s = files.len();
                    files.push((dir, name));
                    s
                });
                let (line, col) = file.line_column(span.start).unwrap_or((1, 1));
                (slot, line as u32, col as u32)
            });
            origins.push(mapped);
        }

        Self { origins, files }
    }
}

impl DebugSourceResolver for OriginDebugResolver {
    fn resolve_origin(&self, origin_raw: u32) -> Option<(&str, &str, u32, u32)> {
        let (slot, line, col) = (*self.origins.get(origin_raw as usize)?)?;
        let (dir, name) = self.files.get(slot)?;
        Some((dir.as_str(), name.as_str(), line, col))
    }

    fn entry_file(&self) -> Option<(&str, &str)> {
        self.files.first().map(|(d, n)| (d.as_str(), n.as_str()))
    }
}

/// Session choices that affect the validated MIR and emitted module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOptions {
    pub target: Target,
    pub optimization: OptimizationLevel,
    /// Enable DWARF/CodeView source debug metadata (`-g` / `--debug`).
    pub debug_info: bool,
    /// Enable the safe whole-program MIR inliner (not object-file LTO).
    pub lto: bool,
    /// Instrument Cranelift function entries and write an execution profile.
    pub pgo_generate: bool,
    /// Profile consumed for hot-callee inlining, if present.
    pub pgo_use: Option<PathBuf>,
    /// Reject any `unsafe` blocks, `unsafe def`, pointer ops, casts, or kernel intrinsics (`[E4501]`).
    pub deny_unsafe: bool,
    /// Optional explicit virtual/physical base address when linking freestanding bare-metal ELF images.
    pub image_base: Option<u64>,
    /// Prepend a Multiboot2 header block when linking a bare-metal kernel ELF image.
    pub multiboot2: bool,
}

impl CompileOptions {
    #[must_use]
    pub fn for_backend(backend: BackendChoice) -> Self {
        Self {
            target: backend.default_target(),
            optimization: OptimizationLevel::O0,
            debug_info: false,
            lto: false,
            pgo_generate: false,
            pgo_use: None,
            deny_unsafe: false,
            image_base: None,
            multiboot2: false,
        }
    }
}

/// A failure anywhere in the source -> object -> executable pipeline.
#[derive(Debug)]
pub enum CompileError {
    /// The dependency graph could not be built (I/O, cycles, missing files).
    Graph(String),
    /// HIR lowering failed.
    Lower(String),
    /// Type inference / checking failed.
    Types(String),
    /// MIR construction or structural verification failed.
    Mir(String),
    /// A MIR optimization pass failed or produced invalid MIR.
    Optimize(String),
    /// Ownership planning or its independent proofs failed.
    Ownership(String),
    /// The selected backend cannot emit the requested target.
    Target(String),
    /// The backend rejected the program (e.g. an unsupported construct).
    Codegen(CodegenError),
    /// Filesystem failure writing the object or executable.
    Io(String),
    /// The link step failed.
    Link(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Graph(error) => write!(formatter, "dependency graph: {error}"),
            Self::Lower(error) => write!(formatter, "HIR lowering: {error}"),
            Self::Types(error) => write!(formatter, "type stage: {error}"),
            Self::Mir(error) => write!(formatter, "MIR: {error}"),
            Self::Optimize(error) => write!(formatter, "MIR optimization: {error}"),
            Self::Ownership(error) => write!(formatter, "ownership: {error}"),
            Self::Target(error) => write!(formatter, "target: {error}"),
            Self::Codegen(error) => write!(formatter, "codegen: {error}"),
            Self::Io(error) => write!(formatter, "i/o: {error}"),
            Self::Link(error) => write!(formatter, "link: {error}"),
        }
    }
}

impl std::error::Error for CompileError {}

/// Compile a single L++ entry file to an object module with the chosen
/// backend — the source -> object half of the pipeline. The entry's parent
/// directory is the package source root.
pub fn compile_entry(
    entry: &Path,
    package_name: &str,
    backend: BackendChoice,
) -> Result<CompiledModule, CompileError> {
    compile_entry_with_options(
        entry,
        package_name,
        backend,
        CompileOptions::for_backend(backend),
    )
}

pub(crate) fn bundled_stdlib_dir() -> Option<PathBuf> {
    if let Ok(env_dir) = std::env::var("LPP_STDLIB_DIR") {
        let p = PathBuf::from(env_dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin_dir) = exe.parent()
    {
        for candidate in [
            bin_dir.join("../lib/stdlib"),
            bin_dir.join("../../stdlib"),
            bin_dir.join("stdlib"),
        ] {
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    let manifest_stdlib = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("stdlib");
    if manifest_stdlib.is_dir() {
        return Some(manifest_stdlib);
    }
    None
}

pub(crate) fn discover_package_dependencies(entry: &Path, package_name: &str) -> Vec<PackageSpec> {
    let mut deps = Vec::new();
    let mut seen_names = std::collections::BTreeSet::new();
    seen_names.insert(package_name.to_string());

    if package_name != "stdlib"
        && let Some(stdlib) = bundled_stdlib_dir()
    {
        seen_names.insert("stdlib".to_string());
        deps.push(PackageSpec::new("stdlib", stdlib));
    }

    let abs_entry = if entry.is_absolute() {
        entry.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(entry)
    };

    let mut search_dir = abs_entry.parent().map(Path::to_path_buf);
    for _ in 0..8 {
        let Some(dir) = search_dir else { break };
        let pkgs_dir = dir.join(".lpp_packages");
        if pkgs_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&pkgs_dir) {
                let mut sorted: Vec<_> = entries.flatten().collect();
                sorted.sort_by_key(|e| e.file_name());
                for entry in sorted {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') || !seen_names.insert(name.clone()) {
                        continue;
                    }
                    let mut source_root = path.join("src");
                    if let Ok(toml_text) = std::fs::read_to_string(path.join("lpp.toml")) {
                        for line in toml_text.lines() {
                            let trimmed = line.trim();
                            if let Some(rest) = trimmed.strip_prefix("entry") {
                                let rest = rest.trim_start();
                                if let Some(rest) = rest.strip_prefix('=') {
                                    let val = rest.trim().trim_matches('"').trim_matches('\'');
                                    let entry_path = path.join(val);
                                    if let Some(parent) = entry_path.parent()
                                        && parent.is_dir()
                                    {
                                        source_root = parent.to_path_buf();
                                    }
                                }
                            }
                        }
                    }
                    if !source_root.is_dir() {
                        source_root = path;
                    }
                    deps.push(PackageSpec::new(name, source_root));
                }
            }
            break;
        }
        if dir.join("Keel.toml").is_file() {
            break;
        }
        search_dir = dir.parent().map(Path::to_path_buf);
    }
    deps
}

/// Compile with explicit target and optimization choices. Every backend sees
/// MIR only after structural verification, bounded optimization, and
/// independent ownership-plan and balance proofs.
pub fn compile_entry_with_options(
    entry: &Path,
    package_name: &str,
    backend: BackendChoice,
    options: CompileOptions,
) -> Result<CompiledModule, CompileError> {
    let codegen = backend.codegen();
    if !codegen.targets().contains(&options.target) {
        return Err(CompileError::Target(format!(
            "backend '{}' does not support {}",
            codegen.name(),
            options.target.triple()
        )));
    }
    let filesystem = OsFileSystem;
    let source_root = entry
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut request = GraphRequest::new(
        entry.to_path_buf(),
        PackageSpec::new(package_name, source_root),
    )
    .with_allow_cycles(true);
    for dep in discover_package_dependencies(entry, package_name) {
        request = request.with_dependency(dep);
    }
    let graph = GraphBuilder::new(&filesystem)
        .build(request)
        .map_err(|error| CompileError::Graph(format!("{error:?}")))?;
    // The corpus uses flat cross-module semantics: `import math` makes math's
    // definitions callable unqualified (e.g. `add(3, 4)`), rather than requiring
    // `math.add`. LegacyFlat is the resolution mode that implements exactly this
    // (see crates/lpp-hir/tests/module_graph_shadow.rs), and it also enforces a
    // global no-duplicate-name rule (E3004) across a program's module set.
    let profile = std::env::var("LPP_PROFILE").is_ok();
    let t0 = std::time::Instant::now();

    let package = lower_package(&graph, ResolutionMode::LegacyFlat)
        .map_err(|error| CompileError::Lower(format!("{error:?}")))?;
    if profile {
        eprintln!("[profile] HIR lowering: {:?}", t0.elapsed());
    }
    let t1 = std::time::Instant::now();

    let work_units = package.expressions.len().saturating_mul(10).max(1_000_000);
    let inference_options = ShadowInferenceOptions {
        work_units,
        ..Default::default()
    };
    let mut inference = infer_hir_package(&package, inference_options)
        .map_err(|error| CompileError::Types(format!("{error}")))?;
    if profile {
        eprintln!("[profile] Type check: {:?}", t1.elapsed());
    }
    let t2 = std::time::Instant::now();

    let mir_options = MirBuildOptions {
        max_functions: package.items.len().max(100_000),
        max_aggregates: package.items.len().max(100_000),
        ..Default::default()
    };
    let mut program = build_mir(&package, &graph.sources, &mut inference, mir_options)
        .map_err(|error| CompileError::Mir(format!("{error:?}")))?;
    if options.deny_unsafe && !program.unsafe_sites().is_empty() {
        let first = program.unsafe_sites()[0];
        return Err(CompileError::Mir(format!(
            "[E4501] --deny-unsafe rejected {} unsafe site(s) (first: {:?} at origin {:?})",
            program.unsafe_sites().len(),
            first.kind,
            first.origin
        )));
    }
    if profile {
        eprintln!("[profile] MIR build: {:?}", t2.elapsed());
    }
    let t3 = std::time::Instant::now();

    let host_target = if cfg!(target_arch = "aarch64") {
        Target::Aarch64
    } else {
        Target::X86_64
    };
    if options.pgo_generate
        && (backend != BackendChoice::Cranelift || options.target != host_target)
    {
        return Err(CompileError::Target(
            "PGO generation currently requires Cranelift for the compiler host target".to_string(),
        ));
    }
    let fingerprint =
        (options.pgo_generate || options.pgo_use.is_some()).then(|| profile_fingerprint(&program));
    if options.pgo_generate {
        let has_main = program.functions().any(|(_, function)| {
            function
                .name
                .as_ref()
                .and_then(|symbol| package.names.symbols.resolve(*symbol))
                == Some("main")
        });
        if !has_main {
            return Err(CompileError::Optimize(
                "PGO generation requires a source-level main function to flush counters"
                    .to_string(),
            ));
        }
    }
    let profile_counts = match (&options.pgo_use, fingerprint) {
        (Some(path), Some(fingerprint)) => Some(read_pgo_profile(path, fingerprint)?),
        _ => None,
    };
    let hot_callees = profile_counts
        .as_ref()
        .map(|counts| select_hot_callees(&program, counts))
        .unwrap_or_default();

    if options.optimization != OptimizationLevel::O0 || options.lto || !hot_callees.is_empty() {
        verify_program(&program, &inference.interner, "after MIR construction")?;
        prove_ownership(&program, &inference.interner, "before optimization")?;
        if profile {
            eprintln!("[profile] Pre-opt verify + ownership: {:?}", t3.elapsed());
        }
        let t4 = std::time::Instant::now();

        run_optimization_with_profile(
            &mut program,
            &inference.interner,
            options.optimization,
            options.lto,
            hot_callees,
        )
        .map_err(|error| CompileError::Optimize(format!("{error:?}")))?;
        if profile {
            eprintln!("[profile] Optimization: {:?}", t4.elapsed());
        }
    }
    let t5 = std::time::Instant::now();
    verify_program(&program, &inference.interner, "final MIR")?;
    prove_ownership(&program, &inference.interner, "final MIR")?;
    if profile {
        eprintln!("[profile] Verify + ownership: {:?}", t5.elapsed());
    }
    let t6 = std::time::Instant::now();

    let backend_opt = match options.optimization {
        OptimizationLevel::O0 => OptLevel::None,
        OptimizationLevel::O1 => OptLevel::O1,
        OptimizationLevel::O2 => OptLevel::O2,
        OptimizationLevel::O3 => OptLevel::O3,
        OptimizationLevel::Os => OptLevel::Os,
        OptimizationLevel::Oz => OptLevel::Oz,
    };
    let names = Names(&package.names.symbols);
    let dbg_resolver = options
        .debug_info
        .then(|| OriginDebugResolver::from_package(&package, &graph.sources));
    let mut codegen_options = CodegenOptions::new(options.target, &names)
        .with_opt_level(backend_opt)
        .with_debug_info(options.debug_info)
        .with_debug_sources(dbg_resolver.as_ref().map(|r| r as &dyn DebugSourceResolver));
    if options.pgo_generate
        && let Some(fingerprint) = fingerprint
    {
        codegen_options = codegen_options.with_pgo_fingerprint(fingerprint);
    }
    let compiled = codegen
        .compile_module(&program, &inference.interner, &codegen_options)
        .map_err(CompileError::Codegen)?;
    if profile {
        eprintln!("[profile] Cranelift codegen: {:?}", t6.elapsed());
    }
    Ok(compiled)
}

fn profile_fingerprint(program: &lpp_mir::MirProgram) -> u64 {
    // FNV-1a over deterministic MIR text ties a profile to this exact, pre-opt
    // program without making profile IDs depend on optimization decisions.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in mir_snapshot(program).bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn read_pgo_profile(
    path: &Path,
    expected_fingerprint: u64,
) -> Result<BTreeMap<u32, u64>, CompileError> {
    let contents = std::fs::read_to_string(path).map_err(|error| {
        CompileError::Optimize(format!(
            "cannot read PGO profile {}: {error}",
            path.display()
        ))
    })?;
    let mut lines = contents.lines();
    let header = lines.next().unwrap_or_default();
    let mut fields = header.split_whitespace();
    let format = fields.next().unwrap_or_default();
    let fingerprint_text = fields.next().unwrap_or_default();
    let fingerprint = (fingerprint_text.len() == 16
        && fingerprint_text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()))
    .then(|| u64::from_str_radix(fingerprint_text, 16).ok())
    .flatten();
    if format != "LPP-PGO-1" || fields.next().is_some() || fingerprint.is_none() {
        return Err(CompileError::Optimize(format!(
            "invalid PGO profile header in {} (expected `LPP-PGO-1 <16-hex-digit fingerprint>`)",
            path.display()
        )));
    }
    let fingerprint = fingerprint.expect("validated above");
    if fingerprint != expected_fingerprint {
        return Err(CompileError::Optimize(format!(
            "PGO profile fingerprint mismatch for {}: profile has {fingerprint:016x}, source has {expected_fingerprint:016x}",
            path.display()
        )));
    }

    let mut counts = BTreeMap::new();
    for (line_index, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let values: Vec<_> = line.split_whitespace().collect();
        if values.len() != 2 {
            return Err(CompileError::Optimize(format!(
                "invalid PGO profile record at {}:{}",
                path.display(),
                line_index + 2
            )));
        }
        let function_id = values[0].parse::<u32>().map_err(|_| {
            CompileError::Optimize(format!(
                "invalid function ID in PGO profile at {}:{}",
                path.display(),
                line_index + 2
            ))
        })?;
        let count = values[1].parse::<u64>().map_err(|_| {
            CompileError::Optimize(format!(
                "invalid execution count in PGO profile at {}:{}",
                path.display(),
                line_index + 2
            ))
        })?;
        if counts.insert(function_id, count).is_some() {
            return Err(CompileError::Optimize(format!(
                "duplicate function ID {function_id} in PGO profile {}",
                path.display()
            )));
        }
    }
    Ok(counts)
}

fn select_hot_callees(
    program: &lpp_mir::MirProgram,
    counts: &BTreeMap<u32, u64>,
) -> BTreeSet<MirFunctionId> {
    let hottest = counts.values().copied().max().unwrap_or_default();
    if hottest == 0 {
        return BTreeSet::new();
    }
    let threshold = (hottest / 100).max(1);
    program
        .functions()
        .filter_map(|(id, _)| {
            counts
                .get(&id.raw())
                .is_some_and(|count| *count >= threshold)
                .then_some(id)
        })
        .collect()
}

fn verify_program(
    program: &lpp_mir::MirProgram,
    types: &lpp_types::TypeInterner,
    stage: &str,
) -> Result<(), CompileError> {
    let t = std::time::Instant::now();
    let errors = verify_mir(program, types);
    if std::env::var("LPP_PROFILE").is_ok() {
        eprintln!("[profile]   verify_mir({stage}): {:?}", t.elapsed());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CompileError::Mir(format!("{stage}: {errors:#?}")))
    }
}

fn prove_ownership(
    program: &lpp_mir::MirProgram,
    types: &lpp_types::TypeInterner,
    stage: &str,
) -> Result<(), CompileError> {
    let prof = std::env::var("LPP_PROFILE").is_ok();
    let t0 = std::time::Instant::now();
    let plan = compute_ownership_plan(program, types)
        .map_err(|error| CompileError::Ownership(format!("{stage}: {error}")))?;
    if prof {
        eprintln!(
            "[profile]   compute_ownership_plan({stage}): {:?}",
            t0.elapsed()
        );
    }
    let t1 = std::time::Instant::now();
    let plan_errors = verify_ownership_plan(program, types, &plan);
    if prof {
        eprintln!(
            "[profile]   verify_ownership_plan({stage}): {:?}",
            t1.elapsed()
        );
    }
    if !plan_errors.is_empty() {
        return Err(CompileError::Ownership(format!(
            "{stage}: ownership-plan proof failed: {plan_errors:#?}"
        )));
    }
    let t2 = std::time::Instant::now();
    let balance_errors = verify_ownership_balance(program, types, &plan);
    if prof {
        eprintln!(
            "[profile]   verify_ownership_balance({stage}): {:?}",
            t2.elapsed()
        );
    }
    if !balance_errors.is_empty() {
        return Err(CompileError::Ownership(format!(
            "{stage}: ownership-balance proof failed: {balance_errors:#?}"
        )));
    }
    Ok(())
}

/// The glibc dynamic loader (`PT_INTERP`) for the host architecture.
#[cfg(not(target_os = "windows"))]
fn host_dynamic_linker() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "/lib64/ld-linux-x86-64.so.2",
        "aarch64" => "/lib/ld-linux-aarch64.so.1",
        _ => "/lib64/ld-linux-x86-64.so.2",
    }
}

/// Link a compiled object into an executable against the Rust runtime cdylib
/// living in `runtime_lib_dir` (the directory holding `liblpp_runtime.so`).
///
/// The primary path is the **in-process `lpp-linker`** — no external toolchain
/// is invoked. It emits a hosted-glibc ELF: `_start` bootstraps through
/// `__libc_start_main`, undefined `lpp_*`/libc symbols resolve through the PLT
/// at load time against `liblpp_runtime.so` (+ auto-derived `libc`/`libm`), and
/// a `DT_RUNPATH` into `runtime_lib_dir` lets the image find the runtime cdylib
/// without `LD_LIBRARY_PATH`.
///
/// Images are emitted non-PIE (`ET_EXEC`) for now: PIE requires `R_*_RELATIVE`
/// GOT relocations that the direct linker does not yet emit (tracked as the
/// next hardening step). If the direct link fails for any reason, we fall back
/// to `cc` and print a warning to stderr so the gap is visible rather than
/// silently masked. This convenience wrapper has no user libraries; modules
/// containing external calls should use [`link_executable_with_libraries`].
pub fn link_executable(
    object: &[u8],
    runtime_lib_dir: &Path,
    output: &Path,
) -> Result<(), CompileError> {
    let configured = std::env::var("LPP_LINKER").ok();
    link_executable_with_libraries(object, &[], runtime_lib_dir, output, configured.as_deref())
}

/// Link with an explicit implementation preference (`direct` or `cc`).
/// This compatibility wrapper has no user libraries; use
/// [`link_executable_with_libraries`] for external calls.
pub fn link_executable_with(
    object: &[u8],
    runtime_lib_dir: &Path,
    output: &Path,
    linker: Option<&str>,
) -> Result<(), CompileError> {
    link_executable_with_libraries(object, &[], runtime_lib_dir, output, linker)
}

/// Link an object with the system libraries required by its external calls.
pub fn link_executable_with_libraries(
    object: &[u8],
    libraries: &[String],
    runtime_lib_dir: &Path,
    output: &Path,
    linker: Option<&str>,
) -> Result<(), CompileError> {
    for library in libraries {
        if library.is_empty()
            || library.starts_with('-')
            || library == "."
            || library == ".."
            || !library.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'.')
            })
        {
            return Err(CompileError::Link(format!(
                "invalid external library name {library:?}"
            )));
        }
    }

    let dir = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let stem = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("program");
    let object_extension = if cfg!(target_os = "windows") {
        "obj"
    } else {
        "o"
    };
    let object_path = dir.join(format!("{stem}.{object_extension}"));
    std::fs::write(&object_path, object).map_err(|error| CompileError::Io(error.to_string()))?;

    let force_direct = linker == Some("direct");
    if force_direct && !cfg!(any(target_os = "linux", target_os = "windows")) {
        let _ = std::fs::remove_file(&object_path);
        return Err(CompileError::Link(
            "the rewrite direct linker is currently supported on Linux and Windows only; use `--linker cc`"
                .to_string(),
        ));
    }
    let force_cc = linker == Some("cc")
        || std::env::consts::ARCH == "aarch64"
        || !cfg!(any(target_os = "linux", target_os = "windows"))
        // PE import-library lookup and DLL symbol ownership are delegated to
        // the platform linker; the in-process PE linker only supports its
        // explicit import map today.
        || (cfg!(target_os = "windows") && !libraries.is_empty() && !force_direct);

    let result = if force_cc {
        link_executable_cc(&object_path, libraries, runtime_lib_dir, output)
    } else {
        match link_executable_direct(&object_path, libraries, runtime_lib_dir, output) {
            Ok(()) => Ok(()),
            // A genuine input/user error (missing `main`, bad object, bad
            // arguments) is reported as-is. Fall back only for limitations,
            // and never when the user explicitly requested `direct`.
            Err(err) => {
                use lpp_linker::LinkErrorKind::*;
                let is_limitation = matches!(err.kind, Internal | UnsupportedFormat);
                if is_limitation && !force_direct {
                    eprintln!(
                        "[lpp-linker] warning: direct link hit a limitation ({}); falling back to cc",
                        err.message
                    );
                    link_executable_cc(&object_path, libraries, runtime_lib_dir, output)
                } else {
                    Err(CompileError::Link(err.message))
                }
            }
        }
    };

    let _ = std::fs::remove_file(&object_path);
    result
}

/// The in-process link (no external tools). Returns the typed [`LinkError`] so
/// the caller can decide, by error kind, whether to fall back to `cc`.
fn link_executable_direct(
    object_path: &Path,
    libraries: &[String],
    runtime_lib_dir: &Path,
    output: &Path,
) -> Result<(), lpp_linker::LinkError> {
    #[cfg(target_os = "windows")]
    use lpp_linker::OutputFormat;
    use lpp_linker::{DynamicMode, LinkOptions};

    #[cfg(target_os = "windows")]
    let options = LinkOptions {
        format: Some(OutputFormat::Pe),
        dynamic: DynamicMode::Force,
        search_paths: vec![runtime_lib_dir.to_path_buf()],
        libraries: libraries.to_vec(),
        ..Default::default()
    };

    #[cfg(not(target_os = "windows"))]
    let options = LinkOptions {
        dynamic: DynamicMode::Force, // resolve lpp_*/libc through the PLT at load time
        pie: false,                  // ET_EXEC until RELATIVE GOT relocs land
        dynamic_linker: Some(host_dynamic_linker().to_string()),
        // The runtime cdylib is a load-time dependency; libc/libm are derived
        // from undefined symbols, and user-declared extern libraries become
        // additional DT_NEEDED entries.
        needed: vec!["liblpp_runtime.so".to_string()],
        search_paths: vec![runtime_lib_dir.to_path_buf()],
        // Find the runtime cdylib at run time without LD_LIBRARY_PATH.
        rpath: vec![runtime_lib_dir.display().to_string()],
        libraries: libraries.to_vec(),
        ..Default::default()
    };

    lpp_linker::link_typed(&[object_path.to_path_buf()], output, &options)?;

    #[cfg(target_os = "windows")]
    {
        let runtime = runtime_library_path()
            .unwrap_or_else(|| runtime_lib_dir.join(runtime_library_filename()));
        let destination = output
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(runtime_library_filename());
        if runtime.is_file() && runtime != destination {
            let _ = std::fs::copy(&runtime, &destination);
        }
    }

    Ok(())
}

/// The legacy external-toolchain link, kept as a fallback.
#[cfg(not(target_os = "windows"))]
fn link_executable_cc(
    object_path: &Path,
    libraries: &[String],
    runtime_lib_dir: &Path,
    output: &Path,
) -> Result<(), CompileError> {
    let compiler = std::env::var("LPP_HOST_CC").unwrap_or_else(|_| "cc".to_string());
    let rpath = format!("-Wl,-rpath,{}", runtime_lib_dir.display());
    let mut link_cmd = Command::new(&compiler);
    link_cmd
        .arg(object_path)
        .arg("-o")
        .arg(output)
        .arg("-L")
        .arg(runtime_lib_dir)
        .arg(rpath);
    for library in libraries {
        link_cmd.arg(format!("-l{library}"));
    }
    // Keep runtime/math libraries after user archives so their undefined
    // references can be resolved in the normal left-to-right linker model.
    link_cmd.arg("-llpp_runtime").arg("-lm");
    let link = link_cmd
        .output()
        .map_err(|error| CompileError::Io(format!("failed to run {compiler}: {error}")))?;
    if !link.status.success() {
        return Err(CompileError::Link(
            String::from_utf8_lossy(&link.stderr).into_owned(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn link_executable_cc(
    object_path: &Path,
    libraries: &[String],
    runtime_lib_dir: &Path,
    output: &Path,
) -> Result<(), CompileError> {
    let compiler = std::env::var("LPP_HOST_CC").unwrap_or_else(|_| "cl.exe".to_string());
    let import_library = ["lpp_runtime.dll.lib", "lpp_runtime.lib"]
        .into_iter()
        .map(|name| runtime_lib_dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            CompileError::Link(format!(
                "runtime import library not found in {}",
                runtime_lib_dir.display()
            ))
        })?;
    let mut link_cmd = if compiler.ends_with(".cmd") || compiler.ends_with(".bat") {
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/c").arg(&compiler);
        cmd
    } else {
        Command::new(&compiler)
    };
    link_cmd
        .arg("/nologo")
        .arg(object_path)
        .arg(format!("/Fe:{}", output.display()))
        .arg("/link")
        .arg(format!("/LIBPATH:{}", runtime_lib_dir.display()))
        .arg(import_library)
        .arg("msvcrt.lib")
        .arg("legacy_stdio_definitions.lib");
    for library in libraries {
        let path = if library.to_ascii_lowercase().ends_with(".lib") {
            library.clone()
        } else {
            format!("{library}.lib")
        };
        link_cmd.arg(path);
    }
    let link = link_cmd
        .output()
        .map_err(|error| CompileError::Io(format!("failed to run {compiler}: {error}")))?;
    if !link.status.success() {
        let mut err = String::from_utf8_lossy(&link.stderr).into_owned();
        if err.trim().is_empty() {
            err = String::from_utf8_lossy(&link.stdout).into_owned();
        }
        return Err(CompileError::Link(err));
    }
    let runtime = runtime_lib_dir.join(runtime_library_filename());
    let destination = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(runtime_library_filename());
    if runtime != destination
        && let Err(error) = std::fs::copy(&runtime, &destination)
        && !destination.is_file()
    {
        return Err(CompileError::Io(format!(
            "copy runtime {} to {}: {error}",
            runtime.display(),
            destination.display()
        )));
    }
    Ok(())
}

/// Link a compiled freestanding kernel object (`x86_64-unknown-none` /
/// `aarch64-unknown-none`) statically into a standalone ELF64 kernel image via
/// `lpp-linker`, bundling the pure-Rust `lpp_arch_*` machine-code support object
/// and an optional Multiboot2 header without external toolchains.
pub fn link_freestanding_kernel(
    module: &CompiledModule,
    options: &CompileOptions,
    output: &Path,
) -> Result<(), CompileError> {
    use lpp_linker::{
        DynamicMode, LinkOptions, Machine, OutputFormat, freestanding_arch_object,
        multiboot2_header_object,
    };

    let (machine, default_base) = match module.target {
        Target::X86_64None => (Machine::X86_64, 0x0010_0000_u64),
        Target::Aarch64None => (Machine::Aarch64, 0x4008_0000_u64),
        other => {
            return Err(CompileError::Target(format!(
                "target {} is not freestanding",
                other.triple()
            )));
        }
    };

    let dir = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| CompileError::Io(e.to_string()))?;
    let pid = std::process::id();
    let kernel_obj = dir.join(format!(".lpp_kernel_{pid}.o"));
    let arch_obj = dir.join(format!(".lpp_arch_{pid}.o"));
    let mb2_obj = dir.join(format!(".lpp_mb2_{pid}.o"));

    std::fs::write(&kernel_obj, &module.object).map_err(|e| CompileError::Io(e.to_string()))?;
    std::fs::write(&arch_obj, freestanding_arch_object(machine))
        .map_err(|e| CompileError::Io(e.to_string()))?;

    let mut inputs = Vec::new();
    if options.multiboot2 {
        std::fs::write(&mb2_obj, multiboot2_header_object(machine))
            .map_err(|e| CompileError::Io(e.to_string()))?;
        inputs.push(mb2_obj.clone());
    }
    inputs.push(kernel_obj.clone());
    inputs.push(arch_obj.clone());

    let link_opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(machine),
        entry: module.entry.clone().or_else(|| Some("_start".to_string())),
        image_base: Some(options.image_base.unwrap_or(default_base)),
        pie: false,
        dynamic: DynamicMode::Static,
        ..Default::default()
    };

    let result = lpp_linker::link_typed(&inputs, output, &link_opts)
        .map(|_| ())
        .map_err(|err| CompileError::Link(err.message));

    let _ = std::fs::remove_file(&kernel_obj);
    let _ = std::fs::remove_file(&arch_obj);
    let _ = std::fs::remove_file(&mb2_obj);
    result
}

/// Compile and statically link a bare-metal L++ kernel image using the selected backend.
pub fn build_freestanding_kernel_with_backend(
    entry: &Path,
    package_name: &str,
    backend: BackendChoice,
    options: CompileOptions,
    output: &Path,
) -> Result<CompiledModule, CompileError> {
    let module = compile_entry_with_options(entry, package_name, backend, options.clone())?;
    link_freestanding_kernel(&module, &options, output)?;
    Ok(module)
}

/// Compile and statically link a bare-metal L++ kernel image.
pub fn build_freestanding_kernel(
    entry: &Path,
    package_name: &str,
    options: CompileOptions,
    output: &Path,
) -> Result<CompiledModule, CompileError> {
    build_freestanding_kernel_with_backend(
        entry,
        package_name,
        BackendChoice::Cranelift,
        options,
        output,
    )
}

/// Compile an L++ entry file all the way to a runnable native executable.
pub fn build_executable(
    entry: &Path,
    package_name: &str,
    backend: BackendChoice,
    output: &Path,
    runtime_lib_dir: &Path,
) -> Result<(), CompileError> {
    build_executable_with_options(
        entry,
        package_name,
        backend,
        CompileOptions::for_backend(backend),
        output,
        runtime_lib_dir,
    )
}

pub fn build_executable_with_options(
    entry: &Path,
    package_name: &str,
    backend: BackendChoice,
    options: CompileOptions,
    output: &Path,
    runtime_lib_dir: &Path,
) -> Result<(), CompileError> {
    let debug_info = options.debug_info;
    let module = compile_entry_with_options(entry, package_name, backend, options)?;
    let configured = std::env::var("LPP_LINKER").ok();
    let effective = if debug_info && configured.as_deref() != Some("direct") {
        Some("cc")
    } else {
        configured.as_deref()
    };
    link_executable_with_libraries(
        &module.object,
        &module.link_libraries,
        runtime_lib_dir,
        output,
        effective,
    )
}

pub fn build_executable_configured(
    entry: &Path,
    package_name: &str,
    backend: BackendChoice,
    options: CompileOptions,
    output: &Path,
    runtime_lib_dir: &Path,
    linker: Option<&str>,
) -> Result<(), CompileError> {
    let debug_info = options.debug_info;
    let module = compile_entry_with_options(entry, package_name, backend, options)?;
    let effective = if debug_info && linker != Some("direct") {
        Some("cc")
    } else {
        linker
    };
    link_executable_with_libraries(
        &module.object,
        &module.link_libraries,
        runtime_lib_dir,
        output,
        effective,
    )
}

/// Platform filename of the rewrite runtime dynamic library.
#[must_use]
pub const fn runtime_library_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "lpp_runtime.dll"
    } else if cfg!(target_os = "macos") {
        "liblpp_runtime.dylib"
    } else {
        "liblpp_runtime.so"
    }
}

/// Locate the rewrite runtime in an explicit override, an installed toolchain
/// layout (`bin/lpp` beside `lib/<runtime>`), or this source tree's Cargo
/// output. The ordered search is deterministic and never scans arbitrary
/// directories.
#[must_use]
pub fn runtime_library_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("LPP_RUNTIME_LIB").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }

    let filename = runtime_library_filename();
    let mut directories = Vec::new();
    if let Some(directory) = std::env::var_os("LPP_RUNTIME_DIR").map(PathBuf::from) {
        directories.push(directory);
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(bin) = executable.parent()
    {
        directories.push(bin.to_path_buf());
        directories.push(bin.join("lib"));
        if let Some(prefix) = bin.parent() {
            directories.push(prefix.join("lib"));
            // Cargo integration tests execute from target/<profile>/deps.
            if bin.file_name().is_some_and(|name| name == "deps") {
                directories.push(prefix.to_path_buf());
            }
        }
    }
    if let Some(workspace) = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
    {
        directories.push(workspace.join("target").join("debug"));
        directories.push(workspace.join("target").join("release"));
    }

    directories
        .into_iter()
        .map(|directory| directory.join(filename))
        .find(|path| path.is_file())
}

/// Runtime directory used by native linking. If discovery fails, retain the
/// source-tree debug path so the eventual linker diagnostic names the expected
/// location rather than silently selecting an unrelated library.
#[must_use]
pub fn default_runtime_lib_dir() -> PathBuf {
    runtime_library_path()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .map(|workspace| workspace.join("target").join("debug"))
        })
        .unwrap_or_else(|| PathBuf::from("target/debug"))
}

/// Spawn a newly-linked executable with bounded retries if another thread's
/// fork briefly holds a writable file descriptor (`ETXTBSY` on Linux).
pub fn exec_status_retry(exe: &Path) -> std::io::Result<std::process::ExitStatus> {
    let mut attempt = 0u64;
    loop {
        match Command::new(exe).status() {
            Ok(status) => return Ok(status),
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 12 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(attempt * 2));
            }
            Err(err) => return Err(err),
        }
    }
}

/// Capture stdout/stderr of a newly-linked executable with bounded `ETXTBSY` retries.
pub fn exec_output_retry(exe: &Path) -> std::io::Result<std::process::Output> {
    let mut attempt = 0u64;
    loop {
        match Command::new(exe).output() {
            Ok(output) => return Ok(output),
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 12 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(attempt * 2));
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_staged_keel_packages_for_import_resolution() {
        let root = std::env::temp_dir().join(format!(
            "lpp_stage_discover_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let src_dir = root.join("src");
        let dep_src = root.join(".lpp_packages/lpp-collections/src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&dep_src).unwrap();
        std::fs::write(
            root.join("Keel.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".lpp_packages/lpp-collections/lpp.toml"),
            "[package]\nname = \"lpp-collections\"\nversion = \"0.1.0\"\nentry = \"src/lib.lpp\"\n",
        )
        .unwrap();
        std::fs::write(
            dep_src.join("lib.lpp"),
            "def answer() -> Int:\n    return 42\n",
        )
        .unwrap();
        let entry = src_dir.join("main.lpp");
        std::fs::write(
            &entry,
            "import lpp_collections\ndef main():\n    print(answer())\n",
        )
        .unwrap();

        let deps = discover_package_dependencies(&entry, "app");
        assert!(
            deps.iter()
                .any(|d| d.name == "lpp-collections" && d.source_root == dep_src),
            "expected lpp-collections in {deps:?}"
        );

        let compiled = compile_entry_with_options(
            &entry,
            "app",
            BackendChoice::Cranelift,
            CompileOptions::for_backend(BackendChoice::Cranelift),
        );
        let _ = std::fs::remove_dir_all(&root);
        assert!(compiled.is_ok(), "compile failed: {:?}", compiled.err());
    }
}
