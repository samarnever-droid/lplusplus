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
    Backend, CodegenError, CodegenOptions, CompiledModule, NameResolver, Target,
};
use lpp_codegen_cranelift::CraneliftBackend;
use lpp_codegen_llvm::LlvmBackend;
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    GraphBuilder, GraphRequest, OsFileSystem, PackageSpec, ResolutionMode, StringInterner, Symbol,
    lower_package,
};
use lpp_mir::{MirBuildOptions, build_mir};
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
    /// The target triple this backend emits, and the backend itself.
    fn resolve(self) -> (Target, &'static dyn Backend) {
        match self {
            Self::Cranelift => (Target::X86_64, &CraneliftBackend),
            Self::Wasm => (Target::Wasm32Wasi, &WasmBackend),
            Self::Llvm => (Target::X86_64, &LlvmBackend),
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

/// A failure anywhere in the source -> object -> executable pipeline.
#[derive(Debug)]
pub enum CompileError {
    /// The dependency graph could not be built (I/O, cycles, missing files).
    Graph(String),
    /// HIR lowering failed.
    Lower(String),
    /// Type inference / checking failed.
    Types(String),
    /// MIR construction failed.
    Mir(String),
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
            Self::Mir(error) => write!(formatter, "MIR build: {error}"),
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
    let (target, codegen) = backend.resolve();
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
    let package = lower_package(&graph, ResolutionMode::LegacyFlat)
        .map_err(|error| CompileError::Lower(format!("{error:?}")))?;
    let mut inference = infer_hir_package(&package, ShadowInferenceOptions::default())
        .map_err(|error| CompileError::Types(format!("{error:?}")))?;
    let program = build_mir(
        &package,
        &graph.sources,
        &mut inference,
        MirBuildOptions::default(),
    )
    .map_err(|error| CompileError::Mir(format!("{error:?}")))?;

    let names = Names(&package.names.symbols);
    codegen
        .compile_module(
            &program,
            &inference.interner,
            &CodegenOptions::new(target, &names),
        )
        .map_err(CompileError::Codegen)
}

/// The glibc dynamic loader (`PT_INTERP`) for the host architecture.
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
    let dir = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let stem = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("program");
    let object_path = dir.join(format!("{stem}.o"));
    std::fs::write(&object_path, object).map_err(|error| CompileError::Io(error.to_string()))?;

    // `LPP_LINKER=cc` forces the external toolchain; anything else (default)
    // uses the in-process linker first.
    let force_cc = std::env::var("LPP_LINKER").ok().as_deref() == Some("cc");

    let result = if force_cc {
        link_executable_cc(&object_path, runtime_lib_dir, output)
    } else {
        match link_executable_direct(&object_path, runtime_lib_dir, output) {
            Ok(()) => Ok(()),
            // A genuine input/user error (missing `main`, bad object, bad
            // arguments) is reported as-is: `cc` would reject it too, and the
            // in-process message is cleaner than a `collect2` dump. Only fall
            // back for errors that signal an in-process *limitation* (an
            // unexpected internal failure or an unsupported construct), where
            // the mature external toolchain may still succeed.
            Err(err) => {
                use lpp_linker::LinkErrorKind::*;
                let is_limitation = matches!(err.kind, Internal | UnsupportedFormat);
                if is_limitation {
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
    use lpp_linker::{DynamicMode, LinkOptions};

    let mut options = LinkOptions::default();
    options.dynamic = DynamicMode::Force; // resolve lpp_*/libc through the PLT at load time
    options.pie = false; // ET_EXEC until RELATIVE GOT relocs land
    options.dynamic_linker = Some(host_dynamic_linker().to_string());
    // The runtime cdylib must be a load-time dependency; libc/libm are derived
    // from the object's undefined symbols by the linker.
    options.needed = vec!["liblpp_runtime.so".to_string()];
    options.search_paths = vec![runtime_lib_dir.to_path_buf()];
    // Find the runtime cdylib at run time without LD_LIBRARY_PATH.
    options.rpath = vec![runtime_lib_dir.display().to_string()];

    lpp_linker::link_typed(&[object_path.to_path_buf()], output, &options).map(|_report| ())
}

/// The legacy external-toolchain link, kept as a fallback.
fn link_executable_cc(
    object_path: &Path,
    runtime_lib_dir: &Path,
    output: &Path,
) -> Result<(), CompileError> {
    let link = Command::new("cc")
        .arg(object_path)
        .arg("-o")
        .arg(output)
        .arg("-L")
        .arg(runtime_lib_dir)
        .arg("-llpp_runtime")
        .arg("-Wl,-rpath")
        .arg(runtime_lib_dir)
        .arg("-lm")
        .output()
        .map_err(|error| CompileError::Io(error.to_string()))?;
    if !link.status.success() {
        return Err(CompileError::Link(
            String::from_utf8_lossy(&link.stderr).into_owned(),
        ));
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
    let module = compile_entry(entry, package_name, backend)?;
    link_executable(&module.object, runtime_lib_dir, output)
}

/// The default runtime library directory: the workspace `target/debug`, where
/// `cargo build -p lpp-runtime` places `liblpp_runtime.so`.
#[must_use]
pub fn default_runtime_lib_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|workspace| workspace.join("target").join("debug"))
        .unwrap_or_else(|| PathBuf::from("target/debug"))
}
