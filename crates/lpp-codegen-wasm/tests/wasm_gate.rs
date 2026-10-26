//! Rewrite WASM gate — executable and ownership parity for the direct
//! `wasm32-wasip1` backend.
//!
//! # Capability matrix enforced by this test module
//!
//! | Surface | Persistent gate |
//! | --- | --- |
//! | Scalars, control flow, direct calls, and deterministic objects | `data_surface_matches_oracle`, `deterministic_object` |
//! | Structs, enums, lists, nested managed values, and ARC traffic | `managed_corpus_matches_the_reference_oracle`, `arc_stress_corpus_matches_the_arc_oracle` |
//! | Dynamic strings and legacy numeric/string builtins | `string_builtin_corpus_matches_the_arc_oracle`, `extended_legacy_builtins_and_borrowed_calls_match_the_arc_oracle` |
//! | Closures and first-class function values | `closure_corpus_matches_the_arc_oracle`, `function_value_corpus_matches_the_arc_oracle` |
//! | Tasks, async values, and restricted eager spawn | `task_corpus_matches_the_arc_oracle` |
//! | Tabled destructors and object shape | the executable ARC corpora plus `object_census_matches_the_contract` |
//!
//! Tuple, slice, and map execution have dedicated sibling gates. The driver
//! gate compiles, validates, executes, and byte-compares the complete positive
//! legacy WASM corpus while checking the intentional host/SIMD/FFI rejection
//! boundary. Arbitrary FFI, SIMD, and WASI preview-1 capabilities that have no
//! direct backend implementation remain structured compile-time rejections.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{Backend, CodegenOptions, CompiledModule, NameResolver, Target};
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{
    ExecutionOutcome, InterpreterLimits, MirAggregateId, MirFunctionId, MirProgram, build_mir,
};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

// ── scaffolding (mirrors the 5B/5C gates) ──────────────────────────────────

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str, root: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from(format!("{root}/main.lpp")), source.to_owned())]),
        }
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.files.contains_key(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.files.contains_key(path) || self.files.keys().any(|file| file.starts_with(path)) {
            Ok(path.to_owned())
        } else {
            Err(FileSystemError::new("canonicalize", path, "path not found"))
        }
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| FileSystemError::new("read", path, "path not found"))
    }
}

struct Names<'a>(&'a StringInterner);

impl NameResolver for Names<'_> {
    fn resolve(&self, symbol_raw: u32) -> Option<&str> {
        self.0.resolve(Symbol::from_raw(symbol_raw))
    }
}

fn pipeline(source: &str) -> (MirProgram, TypeInterner, lpp_hir::HirPackage) {
    let filesystem = MemoryFileSystem::new(source, "/p5d");
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/p5d/main.lpp",
            PackageSpec::new("p5d", "/p5d"),
        ))
        .unwrap_or_else(|e| panic!("graph: {e:?}"));
    let package = lower_package(&graph, ResolutionMode::Namespaced)
        .unwrap_or_else(|e| panic!("lower: {e:?}"));
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|e| panic!("type stage: {e:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        lpp_mir::MirBuildOptions::default(),
    )
    .unwrap_or_else(|e| panic!("build: {e:?}"));
    (program, types.interner, package)
}

fn main_function(program: &MirProgram, names: &StringInterner) -> MirFunctionId {
    program
        .functions()
        .find(|(_, function)| {
            function
                .name
                .as_ref()
                .and_then(|symbol| names.resolve(*symbol))
                == Some("main")
        })
        .map(|(id, _)| id)
        .expect("corpus programs define main")
}

fn run_oracle(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    execute_mir(program, types, entry)
}

fn execute_mir(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    lpp_mir::execute_mir_with_stats(program, types, entry, &[], InterpreterLimits::default())
        .unwrap_or_else(|error| panic!("oracle execution failed: {error}"))
}

/// The ARC-mode oracle: the 4D plan's pinned set (cycle members) tells
/// the interpreter which types may legitimately survive the run; the
/// end-of-run balance proof validates the interpreter's ARC model.
fn run_arc_oracle(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    let plan = compute_ownership_plan(program, types)
        .unwrap_or_else(|error| panic!("ownership plan failed: {error:?}"));
    let pinned = plan.pinned_types();
    lpp_mir::execute_mir_arc(
        program,
        types,
        entry,
        &[],
        InterpreterLimits::default(),
        &pinned,
    )
    .unwrap_or_else(|error| panic!("arc oracle execution failed: {error}"))
}

fn try_compile(
    program: &MirProgram,
    types: &TypeInterner,
    names: &Names<'_>,
) -> Result<CompiledModule, lpp_codegen_api::CodegenError> {
    let backend = WasmBackend;
    let options = CodegenOptions::new(Target::Wasm32Wasi, names);
    backend.compile_module(program, types, &options)
}

fn compile(program: &MirProgram, types: &TypeInterner, names: &Names<'_>) -> CompiledModule {
    try_compile(program, types, names).unwrap_or_else(|e| panic!("compile_module failed: {e:?}"))
}

fn workdir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp5d_{}_{}", std::process::id(), test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write the object to a `.wasm` and run it under Node's `node:wasi`;
/// returns (stdout, exit status).
/// Structural validation of a generated module (inspired by the
/// Bytecode Alliance's wasm-tools: catch malformed binaries with a
/// precise wasmparser error BEFORE V8 can trap cryptically at runtime).
fn validate_wasm(object: &[u8], label: &str) {
    for payload in wasmparser::Parser::new(0).parse_all(object) {
        if let Err(e) = payload {
            panic!("{label}: malformed wasm binary: {e}");
        }
    }
}

/// Print the module as WAT (wasmprinter) — the hex-archaeology killer
/// for structural debugging (tables, element segments, call_indirect
/// operand order, local classes).
fn dump_wat(object: &[u8], label: &str) {
    if let Ok(wat) = wasmprinter::print_bytes(object) {
        eprintln!("=== WAT ({label}) ===\n{wat}");
    }
}

fn run_wasm_memory_pages(object: &[u8], test_name: &str) -> (String, i32, u32) {
    validate_wasm(object, test_name);
    let dir = workdir(test_name);
    let wasm = dir.join("module.wasm");
    let host = dir.join("memory_probe.mjs");
    std::fs::write(&wasm, object).unwrap();
    std::fs::write(
        &host,
        r#"import { readFileSync } from "node:fs";
import { WASI } from "node:wasi";
const file = process.argv[2];
const wasi = new WASI({ version: "preview1", args: [file], env: {}, returnOnExit: true });
const module = await WebAssembly.compile(readFileSync(file));
const instance = await WebAssembly.instantiate(module, { wasi_snapshot_preview1: wasi.wasiImport });
const status = wasi.start(instance);
process.stderr.write(`MEMORY_PAGES:${instance.exports.memory.buffer.byteLength / 65536}\n`);
process.exitCode = status;
"#,
    )
    .unwrap();
    let run = Command::new("node")
        .arg(&host)
        .arg(&wasm)
        .output()
        .unwrap_or_else(|e| panic!("node failed to start ({e}); is Node.js on PATH?"));
    let stderr = String::from_utf8_lossy(&run.stderr);
    let pages = stderr
        .lines()
        .find_map(|line| line.strip_prefix("MEMORY_PAGES:")?.parse().ok())
        .unwrap_or_else(|| panic!("missing memory-page marker: {stderr}"));
    (
        String::from_utf8_lossy(&run.stdout).into_owned(),
        run.status.code().unwrap_or(-1),
        pages,
    )
}

fn run_wasm(object: &[u8], test_name: &str) -> (String, i32) {
    validate_wasm(object, test_name);
    let dir = workdir(test_name);
    let wasm = dir.join("module.wasm");
    let host = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("wasm_run.mjs");
    std::fs::write(&wasm, object).unwrap();

    let run = Command::new("node")
        .arg(&host)
        .arg(&wasm)
        .output()
        .unwrap_or_else(|e| panic!("node failed to start ({e}); is Node.js on PATH?"));
    let status = run.status.code().unwrap_or(-1);
    if !run.status.success() {
        eprintln!(
            "--- wasm stderr ---\n{}\n--- wasm stdout ---\n{}",
            String::from_utf8_lossy(&run.stderr),
            String::from_utf8_lossy(&run.stdout)
        );
        // A runtime failure is the exact moment a WAT dump pays off:
        // dump it automatically, and always under ZZZ_WAT.
        dump_wat(object, test_name);
    }
    if std::env::var("ZZZ_WAT").is_ok() {
        dump_wat(object, test_name);
    }
    (String::from_utf8_lossy(&run.stdout).into_owned(), status)
}

fn expect_success_markers(stdout: &str, test_name: &str) {
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("fail_"))
            .count(),
        0,
        "{test_name}: fail marker present:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line == "all_ok"),
        "{test_name}: all_ok marker missing:\n{stdout}"
    );
}

// ── slice 1: data-surface corpus ───────────────────────────────────────────
//
// Follows the 5B style: a condition that a *correct* compiler must satisfy
// prints a `fail_*` marker on violation. The object and the oracle therefore
// print the same markers; any miscompile changes which markers appear.

#[path = "wasm_gate/builtins.rs"]
mod builtins;
#[path = "wasm_gate/closures.rs"]
mod closures;
#[path = "wasm_gate/data_surface.rs"]
mod data_surface;
#[path = "wasm_gate/edge_cases.rs"]
mod edge_cases;
#[path = "wasm_gate/strings.rs"]
mod strings;
#[path = "wasm_gate/tasks.rs"]
mod tasks;
