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
//! replacement for the v1 C `c_shim.c`. The cdylib is self-contained, so a
//! native executable needs nothing but `libm` at link time.

use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{
    Backend, CodegenError, CodegenOptions, CompiledModule, NameResolver, OptLevel, Target,
};
use lpp_codegen_cranelift::CraneliftBackend;
use lpp_codegen_llvm::LlvmBackend;
use lpp_codegen_wasm::WasmBackend;
use lpp_common::OptimizationLevel;
use lpp_hir::{
    GraphBuilder, GraphRequest, OsFileSystem, PackageSpec, ResolutionMode, StringInterner, Symbol,
    lower_package,
};
use lpp_mir::{MirBuildOptions, build_mir, verify_mir};
use lpp_ownership::{compute_ownership_plan, verify_ownership_balance, verify_ownership_plan};
use lpp_passes::run_optimization;
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
            Self::Cranelift if std::env::consts::ARCH == "aarch64" => Target::Aarch64,
            Self::Cranelift | Self::Llvm => Target::X86_64,
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

/// Session choices that affect the validated MIR and emitted module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileOptions {
    pub target: Target,
    pub optimization: OptimizationLevel,
}

impl CompileOptions {
    #[must_use]
    pub fn for_backend(backend: BackendChoice) -> Self {
        Self {
            target: backend.default_target(),
            optimization: OptimizationLevel::O0,
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
    );
    // Treat the bundled stdlib as a declared dependency of standalone compiler
    // invocations. This lets `from stdlib.math import ...` resolve exactly like
    // a package dependency while preserving local-file precedence for ordinary
    // `import math` cases.
    let bundled_stdlib = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("stdlib");
    if package_name != "stdlib" && bundled_stdlib.is_dir() {
        request = request.with_dependency(PackageSpec::new("stdlib", bundled_stdlib));
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
    if profile { eprintln!("[profile] HIR lowering: {:?}", t0.elapsed()); }
    let t1 = std::time::Instant::now();

    let work_units = package.expressions.len().saturating_mul(10).max(1_000_000);
    let inference_options = ShadowInferenceOptions {
        work_units,
        ..Default::default()
    };
    let mut inference = infer_hir_package(&package, inference_options)
        .map_err(|error| CompileError::Types(format!("{error:?}")))?;
    if profile { eprintln!("[profile] Type check: {:?}", t1.elapsed()); }
    let t2 = std::time::Instant::now();

    let mir_options = MirBuildOptions {
        max_functions: package.items.len().max(100_000),
        max_aggregates: package.items.len().max(100_000),
        ..Default::default()
    };
    let mut program = build_mir(
        &package,
        &graph.sources,
        &mut inference,
        mir_options,
    )
    .map_err(|error| CompileError::Mir(format!("{error:?}")))?;
    if profile { eprintln!("[profile] MIR build: {:?}", t2.elapsed()); }
    let t3 = std::time::Instant::now();

    if options.optimization != OptimizationLevel::O0 {
        verify_program(&program, &inference.interner, "after MIR construction")?;
        prove_ownership(&program, &inference.interner, "before optimization")?;
        if profile { eprintln!("[profile] Pre-opt verify + ownership: {:?}", t3.elapsed()); }
        let t4 = std::time::Instant::now();

        run_optimization(&mut program, &inference.interner, options.optimization)
            .map_err(|error| CompileError::Optimize(format!("{error:?}")))?;
        if profile { eprintln!("[profile] Optimization: {:?}", t4.elapsed()); }
    }
    let t5 = std::time::Instant::now();
    verify_program(&program, &inference.interner, "final MIR")?;
    prove_ownership(&program, &inference.interner, "final MIR")?;
    if profile { eprintln!("[profile] Verify + ownership: {:?}", t5.elapsed()); }
    let t6 = std::time::Instant::now();

    let backend_opt = match options.optimization {
        OptimizationLevel::O0 => OptLevel::None,
        OptimizationLevel::O1 => OptLevel::O1,
        OptimizationLevel::O2 | OptimizationLevel::Os | OptimizationLevel::Oz => OptLevel::O2,
        OptimizationLevel::O3 => OptLevel::O3,
    };
    let names = Names(&package.names.symbols);
    let compiled = codegen
        .compile_module(
            &program,
            &inference.interner,
            &CodegenOptions::new(options.target, &names).with_opt_level(backend_opt),
        )
        .map_err(CompileError::Codegen)?;
    if profile { eprintln!("[profile] Cranelift codegen: {:?}", t6.elapsed()); }
    Ok(compiled)
}

fn verify_program(
    program: &lpp_mir::MirProgram,
    types: &lpp_types::TypeInterner,
    stage: &str,
) -> Result<(), CompileError> {
    let errors = verify_mir(program, types);
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
    let plan = compute_ownership_plan(program, types)
        .map_err(|error| CompileError::Ownership(format!("{stage}: {error}")))?;
    let plan_errors = verify_ownership_plan(program, types, &plan);
    if !plan_errors.is_empty() {
        return Err(CompileError::Ownership(format!(
            "{stage}: ownership-plan proof failed: {plan_errors:#?}"
        )));
    }
    let balance_errors = verify_ownership_balance(program, types, &plan);
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
/// silently masked.
pub fn link_executable(
    object: &[u8],
    runtime_lib_dir: &Path,
    output: &Path,
) -> Result<(), CompileError> {
    let configured = std::env::var("LPP_LINKER").ok();
    link_executable_with(object, runtime_lib_dir, output, configured.as_deref())
}

/// Link with an explicit implementation preference (`direct` or `cc`).
pub fn link_executable_with(
    object: &[u8],
    runtime_lib_dir: &Path,
    output: &Path,
    linker: Option<&str>,
) -> Result<(), CompileError> {
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
        || !cfg!(any(target_os = "linux", target_os = "windows"));

    let result = if force_cc {
        link_executable_cc(&object_path, runtime_lib_dir, output)
    } else {
        match link_executable_direct(&object_path, runtime_lib_dir, output) {
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
                    link_executable_cc(&object_path, runtime_lib_dir, output)
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
    runtime_lib_dir: &Path,
    output: &Path,
) -> Result<(), lpp_linker::LinkError> {
    use lpp_linker::{DynamicMode, LinkOptions, OutputFormat};

    #[cfg(target_os = "windows")]
    let options = LinkOptions {
        format: Some(OutputFormat::Pe),
        dynamic: DynamicMode::Force,
        search_paths: vec![runtime_lib_dir.to_path_buf()],
        ..Default::default()
    };

    #[cfg(not(target_os = "windows"))]
    let options = LinkOptions {
        dynamic: DynamicMode::Force, // resolve lpp_*/libc through the PLT at load time
        pie: false,                  // ET_EXEC until RELATIVE GOT relocs land
        dynamic_linker: Some(host_dynamic_linker().to_string()),
        // The runtime cdylib must be a load-time dependency; libc/libm are derived
        // from the object's undefined symbols by the linker.
        needed: vec!["liblpp_runtime.so".to_string()],
        search_paths: vec![runtime_lib_dir.to_path_buf()],
        // Find the runtime cdylib at run time without LD_LIBRARY_PATH.
        rpath: vec![runtime_lib_dir.display().to_string()],
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
    runtime_lib_dir: &Path,
    output: &Path,
) -> Result<(), CompileError> {
    let compiler = std::env::var("LPP_HOST_CC").unwrap_or_else(|_| "cc".to_string());
    let rpath = format!("-Wl,-rpath,{}", runtime_lib_dir.display());
    let link = Command::new(&compiler)
        .arg(object_path)
        .arg("-o")
        .arg(output)
        .arg("-L")
        .arg(runtime_lib_dir)
        .arg("-llpp_runtime")
        .arg(rpath)
        .arg("-lm")
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
    let link = link_cmd
        .arg("/nologo")
        .arg(object_path)
        .arg(format!("/Fe:{}", output.display()))
        .arg("/link")
        .arg(format!("/LIBPATH:{}", runtime_lib_dir.display()))
        .arg(import_library)
        .arg("msvcrt.lib")
        .arg("legacy_stdio_definitions.lib")
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
    let module = compile_entry_with_options(entry, package_name, backend, options)?;
    link_executable(&module.object, runtime_lib_dir, output)
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
    let module = compile_entry_with_options(entry, package_name, backend, options)?;
    link_executable_with(&module.object, runtime_lib_dir, output, linker)
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
