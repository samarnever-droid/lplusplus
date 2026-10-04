//! Phase 5E gate — the `x86_64` LLVM-IR backend's data-surface slice 1 is
//! correct and deterministic.
//!
//! 1. **Differential execution (data-surface corpus).** Integers (wrapping
//!    arithmetic, signed div/rem, bitwise, shifts including large/negative
//!    amounts, all comparisons), booleans (`&&`/`||`/`!`, eq/ne), chars
//!    (comparisons), floats (arithmetic, modulo, NaN, comparisons, and
//!    `print_float` — the 5E2 float-output slice), control flow (`if`/`else`, nested
//!    `if`, `while`, nested loops), direct calls (multi-parameter,
//!    recursion), and the string surface (`print_str`, `str_len`, content
//!    `==`/`!=`, `write_str` without a newline) plus the `print` dispatch
//!    (int / bool / str / char). The object's stdout, linked against the 5B
//!    `c_shim.c` runtime and run natively, must equal the Phase 4 reference
//!    oracle (`execute_mir_with_stats`) — slice 1 has no managed types, so
//!    the plain reference semantics is the differential target. No `fail_*`
//!    marker, `all_ok` present.
//! 2. **Determinism.** Two compiles produce byte-identical objects and
//!    identical symbol censuses.
//! 3. **Float printing (5E2).** A `print_float` program's stdout, linked
//!    against `c_shim.c`, equals the oracle's `{:.6}` formatting.
//! 4. **Compile-fail.** A struct program still fails with the exact `E5001`
//!    (the managed data surface remains a 5E2 lift).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use lpp_codegen_api::{
    Backend, CodegenErrorKind, CodegenOptions, CompiledModule, NameResolver, Target,
};
use lpp_codegen_llvm::LlvmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{ExecutionOutcome, InterpreterLimits, MirFunctionId, MirProgram, build_mir};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

// ── scaffolding (mirrors the 5B/5C/5D gates) ───────────────────────────────

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
    let filesystem = MemoryFileSystem::new(source, "/p5e");
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/p5e/main.lpp",
            PackageSpec::new("p5e", "/p5e"),
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

fn execute_mir(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    lpp_mir::execute_mir_with_stats(program, types, entry, &[], InterpreterLimits::default())
        .unwrap_or_else(|error| panic!("oracle execution failed: {error}"))
}

fn try_compile(
    program: &MirProgram,
    types: &TypeInterner,
    names: &Names<'_>,
) -> Result<CompiledModule, lpp_codegen_api::CodegenError> {
    let backend = LlvmBackend;
    let options = CodegenOptions::new(Target::X86_64, names);
    backend.compile_module(program, types, &options)
}

fn compile(program: &MirProgram, types: &TypeInterner, names: &Names<'_>) -> CompiledModule {
    try_compile(program, types, names).unwrap_or_else(|e| panic!("compile_module failed: {e:?}"))
}

fn workdir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp5e_{}_{}", std::process::id(), test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write the object, link it against the 5B `c_shim.c` runtime, and run it
/// natively; returns (stdout, exit status).
fn link_and_run(object: &[u8], test_name: &str) -> (String, i32) {
    let dir = workdir(test_name);
    let module = dir.join("module.o");
    let shim = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("lpp-codegen-cranelift")
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
        .unwrap_or_else(|e| panic!("cc spawn: {e}"));
    assert!(
        link.status.success(),
        "link failed:\n{}",
        String::from_utf8_lossy(&link.stderr)
    );

    let run: Output = Command::new(&bin).output().unwrap();
    (
        String::from_utf8_lossy(&run.stdout).to_string(),
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

// ── data-surface corpus (the same scalar surface as the 5D gate) ────────────

const DATA_SURFACE_CORPUS: &str = r#"
def factorial(n: Int) -> Int:
    if n <= 1:
        return 1
    return n * factorial(n - 1)

def combine(a: Int, b: Int, c: Int) -> Int:
    return (a + b) * c - c

def check_int():
    if (9223372036854775806 + 1) != 9223372036854775807:
        print_str("fail_int_add")
    if (7 - 10) != -3:
        print_str("fail_int_sub")
    if (6 * 7) != 42:
        print_str("fail_int_mul")
    if (17 / 5) != 3:
        print_str("fail_int_div")
    if (-17 / 5) != -3:
        print_str("fail_int_sdiv")
    if (17 % 5) != 2:
        print_str("fail_int_rem")
    if (-17 % 5) != -2:
        print_str("fail_int_srem")
    if (9223372036854775807 + 1) >= 0:
        print_str("fail_int_wrap")
    if (255 & 160) != 160:
        print_str("fail_int_band")
    if (255 | 160) != 255:
        print_str("fail_int_bor")
    if (255 ^ 160) != 95:
        print_str("fail_int_bxor")
    if (1 << 10) != 1024:
        print_str("fail_int_shl")
    if (123 << 70) != 7872:
        print_str("fail_int_shl_wrap")
    if (123 << -2) != -4611686018427387904:
        print_str("fail_int_shl_neg")
    if ((1 << 63) >> 63) != -1:
        print_str("fail_int_shr_sign")
    if (-8 >> 1) != -4:
        print_str("fail_int_shr_neg")
    if (9223372036854775807 >> 62) != 1:
        print_str("fail_int_shr_max")
    if 3 < 7 == false:
        print_str("fail_int_lt")
    if 7 <= 7 == false:
        print_str("fail_int_le")
    if 7 > 3 == false:
        print_str("fail_int_gt")
    if 8 >= 7 == false:
        print_str("fail_int_ge")
    if 7 == 7 == false:
        print_str("fail_int_eq")
    if 7 != 8 == false:
        print_str("fail_int_ne")
    if -(5) != -5:
        print_str("fail_int_neg")

def check_bool_char():
    t := true
    f := false
    if (t && f) != false:
        print_str("fail_bool_and")
    if (t || f) == false:
        print_str("fail_bool_or")
    if !(t && f) == false:
        print_str("fail_bool_not")
    if (t == f) != false:
        print_str("fail_bool_eq")
    if (t != f) == false:
        print_str("fail_bool_ne")
    c := 'B'
    if c < 'C' == false:
        print_str("fail_char_lt")
    if c == 'A' != false:
        print_str("fail_char_ne")
    if 'a' > 'A' == false:
        print_str("fail_char_gt")

def check_float():
    if (1.5 + 2.25) != 3.75:
        print_str("fail_float_add")
    if (10.0 / 4.0) != 2.5:
        print_str("fail_float_div")
    if (7.5 % 2.0) != 1.5:
        print_str("fail_float_mod")
    if (-3.5 * 2.0) != -7.0:
        print_str("fail_float_mul")
    if (9.0 - 4.0) != 5.0:
        print_str("fail_float_sub")
    if 1.5 < 2.5 == false:
        print_str("fail_float_lt")
    if 2.5 <= 2.5 == false:
        print_str("fail_float_le")
    nan := 0.0 / 0.0
    if nan == nan:
        print_str("fail_float_nan_eq")
    if nan != nan == false:
        print_str("fail_float_nan_ne")

def check_cfg():
    mut i := 0
    mut total := 0
    while i < 10:
        total = total + i
        i = i + 1
    if total != 45:
        print_str("fail_cfg_sum")
    mut k := 0
    mut s := 0
    while k < 4:
        s = s + k * 2
        k = k + 1
    if s != 12:
        print_str("fail_cfg_k")
    x := 7
    mut acc := s
    if x > 5:
        if x > 6:
            acc = acc + 100
        else:
            acc = acc + 200
    else:
        acc = acc + 300
    if acc != 112:
        print_str("fail_cfg_nested_if")
    mut n := 0
    i = 0
    mut j := 0
    while i < 3:
        j = 0
        while j < 3:
            n = n + 1
            j = j + 1
        i = i + 1
    if n != 9:
        print_str("fail_cfg_nested_loop")

def check_calls():
    if factorial(5) != 120:
        print_str("fail_call_fact")
    if factorial(0) != 1:
        print_str("fail_call_fact0")
    if combine(3, 4, 5) != 30:
        print_str("fail_call_multi")
    if combine(10, 20, 30) != 870:
        print_str("fail_call_multi2")

def check_strings():
    s := "hello"
    if s == "hello" == false:
        print_str("fail_str_eq")
    if s == "world" != false:
        print_str("fail_str_ne")
    if "abc" == "abc" == false:
        print_str("fail_str_eq_lit")
    if "abc" == "abd" != false:
        print_str("fail_str_ne_lit")
    if str_len("hello") != 5:
        print_str("fail_str_len")
    if str_len("") != 0:
        print_str("fail_str_len_empty")

def check_print():
    print_str("str_ok")
    print_int(12345)
    print_int(-42)
    print_bool(true)
    print_bool(false)
    print('A')
    print(0)
    s := "printed"
    print(s)
    write_str("raw")
    print_int(7)

def main() -> Int:
    check_int()
    check_bool_char()
    check_float()
    check_cfg()
    check_calls()
    check_strings()
    check_print()
    print_str("all_ok")
    return 0
"#;

// ── tests ─────────────────────────────────────────────────────────────────

#[test]
fn data_surface_matches_oracle() {
    let (program, types, package) = pipeline(DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let oracle = execute_mir(&program, &types, entry);
    let expected = oracle.output.concat();

    let module = compile(&program, &types, &names);
    assert_eq!(module.target, Target::X86_64);
    let (stdout, status) = link_and_run(&module.object, "data_surface");
    assert_eq!(status, 0, "object exited non-zero:\n{stdout}");
    expect_success_markers(&stdout, "data_surface");
    assert_eq!(
        stdout, expected,
        "object stdout diverged from the reference oracle"
    );
}

#[test]
fn deterministic_object() {
    let (program, types, package) = pipeline(DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let a = compile(&program, &types, &names);
    let b = compile(&program, &types, &names);
    assert_eq!(a.object, b.object, "two compiles differ byte-for-byte");
    assert_eq!(
        a.exported_symbols, b.exported_symbols,
        "export censuses differ"
    );
    assert_eq!(
        a.imported_symbols, b.imported_symbols,
        "import censuses differ"
    );
}

#[test]
fn struct_is_typed_rejection() {
    let source = r#"
struct Point:
    x: Int
    y: Int
def main() -> Int:
    p := Point(1, 2)
    print_int(p.x + p.y)
    return 0
"#;
    let (program, types, package) = pipeline(source);
    let names = Names(&package.names.symbols);
    let error = try_compile(&program, &types, &names)
        .expect_err("a struct program must be rejected on x86_64 (5E slice 1)");
    match error.kind {
        CodegenErrorKind::UnsupportedConstruct { .. } => {}
        other => panic!("expected UnsupportedConstruct (E5001), got {other:?}"),
    }
}

#[test]
fn float_print_matches_oracle() {
    // 5E2 float-output slice: `print_float` now lowers to a `lpp_print_float`
    // call (C `printf("%f\n", …)`, 6 decimals). The object's stdout must equal
    // the oracle's `{:.6}` formatting for float constants and a float local.
    let source = r#"
def main() -> Int:
    print_float(3.5)
    print_float(-0.5)
    print_float(0.0)
    print_float(3.75)
    x := 2.5
    print_float(x)
    print_str("all_ok")
    return 0
"#;
    let (program, types, package) = pipeline(source);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let oracle = execute_mir(&program, &types, entry);
    let expected = oracle.output.concat();

    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "float_print");
    assert_eq!(status, 0, "object exited non-zero:\n{stdout}");
    expect_success_markers(&stdout, "float_print");
    assert_eq!(
        stdout, expected,
        "print_float stdout diverged from the reference oracle"
    );
}
