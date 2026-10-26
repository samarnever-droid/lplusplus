//! Phase 5C2 gate — the Cranelift function-value surface is correct,
//! ARC-balanced, and deterministic.
//!
//! 1. **Differential (closures + tasks).** Counter closures, closures
//!    in lists, capture mutation, nested captures, managed captures,
//!    a closure-capture cycle (4D `pinned_types()` non-empty),
//!    sync/async function values, async/await chains, double await,
//!    spawn of a capturing closure, and the async `main` auto-drain:
//!    object stdout equals the ARC-mode oracle
//!    (`execute_mir_arc` with the 4D `pinned_types()`), no `fail_*`
//!    marker, `all_ok` present.
//! 2. **Differential (builtin families).** The Family A surface —
//!    print family (incl. the consuming `print`/`write_str`), string
//!    ops, conversions (incl. the 3-digit-exponent `float_to_str`),
//!    integer helpers, checked/wrap arithmetic, float ops, and the
//!    list family across every element class: object stdout equals
//!    the ARC-mode oracle.
//! 3. **Self-check (slices + SIMD).** Family B (native 128-bit SIMD)
//!    and C (slice handles) corpora with exact hand-computed stdout —
//!    the interpreter reports `UnsupportedBuiltin` for these families,
//!    so they cannot be oracle-verified.
//! 4. **Determinism + census.** Two compiles of each corpus produce
//!    byte-identical objects; exports include `lpp_main`, every user
//!    function, every closure function (`lpp_c{n}`), every generated
//!    destructor, every generated cycle-collector trace (for the 4D
//!    `Shared` types), and the used thunks; imports stay inside the
//!    closed runtime-symbol universe.
//! 5. **Table census + rejections.** The registry enumerates to
//!    518 = Family A ∪ B ∪ C ∪ D, disjoint; a Family D builtin fails
//!    with the exact `E5003` at the exact function; tuples stay the
//!    only `E5001` non-builtin rejection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{
    Backend, CodegenError, CodegenErrorKind, CodegenOptions, NameResolver, Target,
};
use lpp_codegen_cranelift::CraneliftBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{
    ExecutionOutcome, InterpreterLimits, MirFunctionId, MirFunctionKind, MirProgram, build_mir,
    execute_mir_arc,
};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

#[path = "phase5c2_gate/builtins.rs"]
mod builtins;
#[path = "phase5c2_gate/census.rs"]
mod census;
#[path = "phase5c2_gate/closures.rs"]
mod closures;
#[path = "phase5c2_gate/simd_slice.rs"]
mod simd_slice;

use builtins::BUILTIN_CORPUS;
use closures::CLOSURE_TASK_CORPUS;
use simd_slice::SIMD_SLICE_CORPUS;

// ── scaffolding (mirrors the 5B/5C gates) ─────────────────────────────────

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

    fn with_bindings(main: &str, bindings: &str, root: &str) -> Self {
        Self {
            files: BTreeMap::from([
                (PathBuf::from(format!("{root}/main.lpp")), main.to_owned()),
                (
                    PathBuf::from(format!("{root}/bindings.lpp")),
                    bindings.to_owned(),
                ),
            ]),
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
    let filesystem = MemoryFileSystem::new(source, "/p5c2");
    pipeline_from_filesystem(filesystem)
}

fn pipeline_with_bindings(
    main: &str,
    bindings: &str,
) -> (MirProgram, TypeInterner, lpp_hir::HirPackage) {
    let filesystem = MemoryFileSystem::with_bindings(main, bindings, "/p5c2");
    pipeline_from_filesystem_mode(filesystem, ResolutionMode::LegacyFlat)
}

fn pipeline_from_filesystem(
    filesystem: MemoryFileSystem,
) -> (MirProgram, TypeInterner, lpp_hir::HirPackage) {
    pipeline_from_filesystem_mode(filesystem, ResolutionMode::Namespaced)
}

fn pipeline_from_filesystem_mode(
    filesystem: MemoryFileSystem,
    resolution_mode: ResolutionMode,
) -> (MirProgram, TypeInterner, lpp_hir::HirPackage) {
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/p5c2/main.lpp",
            PackageSpec::new("p5c2", "/p5c2"),
        ))
        .unwrap_or_else(|e| panic!("graph: {e:?}"));
    let package = lower_package(&graph, resolution_mode).unwrap_or_else(|e| panic!("lower: {e:?}"));
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

/// The ARC-mode oracle: the 4D plan's collectible (`Shared`) set
/// tells the interpreter which types may sit on a reference cycle;
/// trial deletion must reclaim them, and the end-of-run balance proof
/// validates the interpreter's ARC model.
fn run_arc_oracle(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    let plan = compute_ownership_plan(program, types)
        .unwrap_or_else(|error| panic!("ownership plan failed: {error:?}"));
    let pinned = plan.pinned_types();
    execute_mir_arc(
        program,
        types,
        entry,
        &[],
        InterpreterLimits::default(),
        &pinned,
    )
    .unwrap_or_else(|error| panic!("arc oracle execution failed: {error}"))
}

fn compile(
    program: &MirProgram,
    types: &TypeInterner,
    names: &Names<'_>,
) -> lpp_codegen_api::CompiledModule {
    let backend = CraneliftBackend;
    let options = CodegenOptions::new(Target::host(), names);
    backend
        .compile_module(program, types, &options)
        .unwrap_or_else(|e| panic!("compile_module failed: {e}"))
}

fn workdir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp5c2_{}_{}", std::process::id(), test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn link_and_run(object: &[u8], test_name: &str) -> (String, i32) {
    let dir = workdir(test_name);
    let module = dir.join("module.o");
    let shim = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("c_shim.c");
    let bin = dir.join("program");
    std::fs::write(&module, object).unwrap();

    let link = Command::new("cc")
        .arg(&module)
        .arg(&shim)
        .arg("-o")
        .arg(&bin)
        .arg("-lm")
        .output()
        .unwrap_or_else(|e| panic!("cc failed to start: {e}"));
    if !link.status.success() {
        panic!("link failed:\n{}", String::from_utf8_lossy(&link.stderr));
    }

    let run = Command::new(&bin)
        .output()
        .unwrap_or_else(|e| panic!("run failed to start: {e}"));
    (
        String::from_utf8_lossy(&run.stdout).into_owned(),
        run.status.code().unwrap_or(-1),
    )
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

// ── corpus 1: closures + tasks ────────────────────────────────────────────
//
// Expected marker values (hand-computed):
//   counter 5, list_closure 42, nested 35, managed_capture 19,
//   sync_value 42, async_value 5, cycle 108, task_chain 10,
//   spawn 1, managed_read_only 13, minimal 3, call_twice 6,
//   no_closure 3, async_chain 5.
//
// Corpus-domain rule (from the contract): the cycle closure's
// function type is shared by no other closure in the program (two
// parameters), so the 4D-pinned type cannot pin an unrelated capsule.

#[test]
fn cli_fs_json_and_ergonomic_builtins_compile_cleanly() {
    let src = r#"
def main() -> Int:
    argc := args_count()
    arg0 := args_get(0)
    all_args := args()
    first_arg := get(all_args, 0)
    joined := str_join(all_args, ",")
    lines := str_lines("a\nb\nc")
    padded_l := str_pad_left("42", 5, "0")
    padded_r := str_pad_right("42", 5, " ")
    esc := json_escape("hello \"world\"")
    h := json_parse("{\"count\": 7, \"name\": \"lpp\"}")
    cnt := json_get_int(h, "count")
    nm := json_get_str(h, "name")
    json_free(h)
    cur := cwd()
    ok_dir := is_dir(cur)
    ok_file := is_file(cur)
    ns := time_ns()
    mem := sys_mem_total()
    free_m := sys_mem_free()
    up := sys_uptime()
    print_str(arg0 + first_arg + joined + padded_l + padded_r + esc + nm + cur)
    if ok_dir and not ok_file:
        return argc + cnt + list_len(lines) + (ns - ns) + (mem - mem) + (free_m - free_m) + (up - up)
    return 0
"#;
    let (program, types, package) = pipeline(src);
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    let module = CraneliftBackend
        .compile_module(&program, &types, &options)
        .unwrap_or_else(|e| panic!("cli/fs/json builtins compile failed: {e}"));
    for sym in [
        "lpp_args_count",
        "lpp_args_get",
        "lpp_args",
        "lpp_str_join",
        "lpp_str_lines",
        "lpp_str_pad_left",
        "lpp_str_pad_right",
        "lpp_json_escape",
        "lpp_json_parse",
        "lpp_json_get_int",
        "lpp_json_get_str",
        "lpp_json_free",
        "lpp_cwd",
        "lpp_is_dir",
        "lpp_is_file",
        "lpp_time_ns",
        "lpp_sys_mem_total",
        "lpp_sys_mem_free",
        "lpp_sys_uptime",
    ] {
        assert!(
            module.imported_symbols.contains(sym),
            "expected imported symbol {sym}"
        );
    }
}

#[test]
fn imported_external_declarations_resolve_and_record_native_dependencies() {
    let bindings = concat!(
        "pub extern \"C\" link \"sqlite3\":\n",
        "    def sqlite3_libversion() -> Str\n",
        "    def sqlite3_libversion_number() -> Int\n",
    );
    let main = concat!(
        "import bindings\n",
        "def main():\n",
        "    print(sqlite3_libversion())\n",
        "    print(sqlite3_libversion_number())\n",
    );
    let (program, types, package) = pipeline_with_bindings(main, bindings);

    assert_eq!(program.external_function_count(), 2);
    assert_eq!(program.link_libraries(), vec!["sqlite3".to_string()]);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    assert!(module.imported_symbols.contains("sqlite3_libversion"));
    assert!(
        module
            .imported_symbols
            .contains("sqlite3_libversion_number")
    );
    assert_eq!(module.link_libraries, vec!["sqlite3".to_string()]);
}

#[test]
fn imported_extern_declaration_links_and_runs_across_modules() {
    let bindings = concat!(
        "pub extern \"C\":\n",
        "    def lpp_test_imported_ffi_add(left: Int, right: Int) -> Int\n",
    );
    let main = concat!(
        "import bindings\n",
        "def main():\n",
        "    print(lpp_test_imported_ffi_add(19, 23))\n",
    );
    let (program, types, package) = pipeline_with_bindings(main, bindings);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    assert!(
        module
            .imported_symbols
            .contains("lpp_test_imported_ffi_add")
    );
    let (stdout, status) = link_and_run(&module.object, "imported-extern-cross-module");
    assert_eq!(status, 0);
    assert_eq!(stdout, "42\n");
}

#[test]
fn external_call_symbols_and_link_libraries_are_reported() {
    let source = concat!(
        "extern \"C\" link \"z\":\n",
        "    def zlibVersion() -> Str\n",
        "extern \"C\" link \"m\":\n",
        "    def cos(value: Float) -> Float\n",
        "    def sin(value: Float) -> Float\n",
        "def main():\n",
        "    print(zlibVersion())\n",
        "    print(cos(0.0))\n",
        "    print(sin(0.0))\n",
    );
    let (program, types, package) = pipeline(source);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    assert_eq!(
        module.link_libraries,
        vec!["m".to_string(), "z".to_string()]
    );
    assert!(module.imported_symbols.contains("zlibVersion"));
    assert!(module.imported_symbols.contains("cos"));
    assert!(module.imported_symbols.contains("sin"));
    assert!(module.imported_symbols.contains("lpp_empty_str"));
    assert!(module.imported_symbols.contains("lpp_str_concat"));
}

#[test]
fn external_c_string_arguments_are_borrowed_from_managed_payloads() {
    let source = concat!(
        "extern \"C\":\n",
        "    def lpp_test_external_string_length(value: Str) -> Int\n",
        "def main():\n",
        "    value := \"foreign-\" + \"borrowed\"\n",
        "    print(lpp_test_external_string_length(value))\n",
    );
    let (program, types, package) = pipeline(source);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    assert!(
        module
            .imported_symbols
            .contains("lpp_test_external_string_length")
    );

    let (stdout, status) = link_and_run(&module.object, "external-c-string-argument");
    assert_eq!(status, 0);
    assert_eq!(stdout, "16\n");
}

#[test]
fn external_c_string_returns_are_copied_into_managed_strings() {
    let source = concat!(
        "extern \"C\":\n",
        "    def lpp_test_external_string() -> Str\n",
        "def main():\n",
        "    value := lpp_test_external_string()\n",
        "    print(value)\n",
    );
    let (program, types, package) = pipeline(source);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    assert!(module.link_libraries.is_empty());
    assert!(module.imported_symbols.contains("lpp_test_external_string"));
    assert!(module.imported_symbols.contains("lpp_empty_str"));
    assert!(module.imported_symbols.contains("lpp_str_concat"));
    let (stdout, status) = link_and_run(&module.object, "external-c-string");
    assert_eq!(status, 0);
    assert_eq!(stdout, "foreign-owned\n");
}
