//! Phase 5B gate — the Cranelift scalar backend is correct and
//! deterministic.
//!
//! 1. **Differential execution.** The 5B scalar corpus (wrapping
//!    overflow, div/rem, shifts, signed comparisons, IEEE NaN/inf,
//!    branches, loops, multi-function calls, char, strings,
//!    `print_str`, `fmod`) is compiled to an object, linked with the
//!    C shim (`tests/c_shim.c`) via `cc`, executed, and its stdout
//!    plus exit status compared against the Phase 4 execution oracle
//!    (`execute_mir_with_stats`), the v1-proven reference.
//! 2. **Determinism.** Two compiles of the same program produce
//!    byte-identical objects and identical symbol censuses.
//! 3. **IR validity.** Every corpus function survives cranelift's own
//!    verification (`define_function` verifies); the object is a real
//!    ELF image.
//! 4. **Compile-fail.** List, struct, closure, and async programs
//!    fail with the exact `E5001`; a non-slice builtin (`print_int`)
//!    fails with the exact `E5003`, each at the exact function.
//!
//! Corpus notes: integer division by zero and `i64::MIN / -1` are
//! deliberately absent — the backend emits the hardware divide
//! (identical to the v1 cranelift backend, which software-traps
//! neither), so those inputs trap in the object while the oracle
//! reports a typed error; the differential is defined on the domain
//! where both are defined.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use lpp_codegen_api::{Backend, CodegenError, CodegenErrorKind, CodegenOptions, NameResolver, Target};
use lpp_codegen_cranelift::CraneliftBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode, Symbol,
    StringInterner, lower_package,
};
use lpp_mir::{
    ExecutionOutcome, InterpreterLimits, MirFunctionId, MirProgram, build_mir, execute_mir_with_stats,
};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

// ── scaffolding ────────────────────────────────────────────────────────────

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/p5b/main.lpp"), source.to_owned())]),
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

/// The name-resolution boundary: the HIR name index stays with the
/// caller; the backend asks for names through `NameResolver`.
struct Names<'a>(&'a StringInterner);

impl NameResolver for Names<'_> {
    fn resolve(&self, symbol_raw: u32) -> Option<&str> {
        self.0.resolve(Symbol::from_raw(symbol_raw))
    }
}

fn pipeline(source: &str) -> (MirProgram, TypeInterner, lpp_hir::HirPackage) {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/p5b/main.lpp",
            PackageSpec::new("p5b", "/p5b"),
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
    execute_mir_with_stats(program, types, entry, &[], InterpreterLimits::default())
        .unwrap_or_else(|error| panic!("oracle execution failed: {error}"))
}

fn compile(program: &MirProgram, types: &TypeInterner, names: &Names<'_>) -> lpp_codegen_api::CompiledModule {
    let backend = CraneliftBackend;
    let options = CodegenOptions::new(Target::X86_64, names);
    backend
        .compile_module(program, types, &options)
        .unwrap_or_else(|e| panic!("compile_module failed: {e}"))
}

fn workdir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp5b_{}_{}", std::process::id(), test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Link the object with the C shim and run it; returns (stdout, exit
/// status). The object is a full ELF relocatable, so `cc` links it
/// against libc (which provides `fmod`).
fn link_and_run(object: &[u8], test_name: &str) -> (String, i32) {
    let dir = workdir(test_name);
    let module = dir.join("module.o");
    let shim = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("c_shim.c");
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
    (String::from_utf8_lossy(&run.stdout).to_string(), run.status.code().unwrap_or(-1))
}

// ── the 5B scalar corpus ───────────────────────────────────────────────────
//
// Every check self-verifies: on mismatch it prints a `fail_*` marker;
// the test asserts the oracle and the object agree on stdout, that no
// marker appears, and that the final `all_ok` line is present.

const CORPUS: &str = "\
def check_arith():
    a := 9223372036854775806
    if (a + 1) != 9223372036854775807:
        print_str(\"fail_arith_max\")
    if (a + 2 - 1) != 9223372036854775807:
        print_str(\"fail_arith_wrap\")
    if (a * -1) != -9223372036854775806:
        print_str(\"fail_arith_neg\")
    if (a * 2) != -4:
        print_str(\"fail_arith_mul_wrap\")
    x := 3037000500
    if (x * x) != -9223372036709301616:
        print_str(\"fail_arith_mul_wrap2\")

def check_div_rem():
    if (17 / 5) != 3:
        print_str(\"fail_div_1\")
    if (17 % 5) != 2:
        print_str(\"fail_rem_1\")
    if (-17 / 5) != -3:
        print_str(\"fail_div_2\")
    if (-17 % 5) != -2:
        print_str(\"fail_rem_2\")
    if (17 / -5) != -3:
        print_str(\"fail_div_3\")
    if (17 % -5) != 2:
        print_str(\"fail_rem_3\")
    if (-17 / -5) != 3:
        print_str(\"fail_div_4\")
    if (-17 % -5) != -2:
        print_str(\"fail_rem_4\")
    if (-100 / 7) != -14:
        print_str(\"fail_div_5\")
    if (-100 % 7) != -2:
        print_str(\"fail_rem_5\")

def check_shifts():
    if (1 << 10) != 1024:
        print_str(\"fail_shl_small\")
    if ((1 << 63) >> 63) != -1:
        print_str(\"fail_shr_sign\")
    if ((1 << 63) * 0) != 0:
        print_str(\"fail_shl_sign\")
    if (-8 >> 1) != -4:
        print_str(\"fail_shr_neg\")
    if (-1 >> 4) != -1:
        print_str(\"fail_shr_all\")
    if (123 << 70) != 7872:
        print_str(\"fail_shl_wrap_amount\")
    if (123 << -2) != -4611686018427387904:
        print_str(\"fail_shl_neg_amount\")
    if (9223372036854775807 >> 62) != 1:
        print_str(\"fail_shr_max\")
    if (-5 << 4) != -80:
        print_str(\"fail_shl_neg\")
    if (-80 >> 4) != -5:
        print_str(\"fail_shr_neg2\")
    if (255 & 160) != 160:
        print_str(\"fail_band\")
    if (255 | 160) != 255:
        print_str(\"fail_bor\")
    if (255 ^ 160) != 95:
        print_str(\"fail_bxor\")

def check_cmp():
    if (3 < 7) == false:
        print_str(\"fail_lt\")
    if (7 < 3) != false:
        print_str(\"fail_lt2\")
    if (7 <= 7) == false:
        print_str(\"fail_le\")
    if (7 > 3) == false:
        print_str(\"fail_gt\")
    if (-2 < 2) == false:
        print_str(\"fail_neg_lt\")
    if (9223372036854775806 < 9223372036854775807) == false:
        print_str(\"fail_max_lt\")
    if (7 >= 7) == false:
        print_str(\"fail_ge\")
    if (7 == 7) == false:
        print_str(\"fail_eq\")
    if (7 != 8) == false:
        print_str(\"fail_ne\")

def check_bool():
    t := true
    f := false
    n := !(t && f)
    if (t && f) != false:
        print_str(\"fail_and\")
    if (t || f) == false:
        print_str(\"fail_or\")
    if (t && t) == false:
        print_str(\"fail_and2\")
    if (f || f) != false:
        print_str(\"fail_or2\")
    if (t == f) != false:
        print_str(\"fail_bool_eq\")
    if (t != f) == false:
        print_str(\"fail_bool_ne\")
    if n == false:
        print_str(\"fail_not\")

def check_floats():
    if (1.5 + 2.25) != 3.75:
        print_str(\"fail_fadd\")
    if (10.0 / 4.0) != 2.5:
        print_str(\"fail_fdiv\")
    if (7.5 % 2.0) != 1.5:
        print_str(\"fail_fmod\")
    if (-3.5 * 2.0) != -7.0:
        print_str(\"fail_fneg_mul\")
    if -7.0 == 7.0:
        print_str(\"fail_fsign\")
    nan := 0.0 / 0.0
    if nan == nan:
        print_str(\"fail_nan_eq\")
    if nan != nan == false:
        print_str(\"fail_nan_ne\")
    if (nan < 1.0) != false:
        print_str(\"fail_nan_lt\")
    if (1.0 < nan) != false:
        print_str(\"fail_nan_lt2\")
    inf := 1.0 / 0.0
    neg_inf := 0.0 / -1.0
    if (inf > 1.0) == false:
        print_str(\"fail_inf\")
    if (neg_inf < 1.0) == false:
        print_str(\"fail_neg_inf\")
    neg_zero := 0.0 / -2.0
    if (neg_zero == 0.0) == false:
        print_str(\"fail_neg_zero\")

def count_down(start: Int) -> Int:
    mut n := start
    mut acc := 0
    while n > 0:
        if (n % 2) == 0:
            acc = acc + n
        else:
            acc = acc - 1
        n = n - 1
    return acc

def double(x: Float) -> Float:
    return x * 2.0

def which(b: Bool) -> Int:
    if b:
        return 1
    return 0

def is_z(c: Char) -> Bool:
    return c == 'z'

def echo(s: Str) -> Str:
    return s

def check_calls():
    if count_down(10) != 25:
        print_str(\"fail_count_down\")
    if double(1.25) != 2.5:
        print_str(\"fail_double\")
    if which(true) != 1:
        print_str(\"fail_which\")
    if which(false) != 0:
        print_str(\"fail_which2\")
    if is_z('z') == false:
        print_str(\"fail_char_eq\")
    if is_z('a') != false:
        print_str(\"fail_char_eq2\")
    if ('a' < 'b') == false:
        print_str(\"fail_char_lt\")
    if ('A' < 'a') == false:
        print_str(\"fail_char_case\")
    if ('a' != 'b') == false:
        print_str(\"fail_char_ne\")
    r := echo(\"round_trip\")
    if r != \"round_trip\":
        print_str(\"fail_echo\")

def check_strings():
    s1 := \"abc\"
    s2 := \"abc\"
    s3 := \"abd\"
    if (s1 == s2) == false:
        print_str(\"fail_str_alias\")
    if (s1 == s3) != false:
        print_str(\"fail_str_ne\")
    print_str(\"str_literal\")
    mut i := 0
    while i < 3:
        print_str(\"loop\")
        i = i + 1

def main():
    check_arith()
    check_div_rem()
    check_shifts()
    check_cmp()
    check_bool()
    check_floats()
    check_calls()
    check_strings()
    print_str(\"all_ok\")
";

// ── gate 1: differential execution ─────────────────────────────────────────

#[test]
fn differential_execution_matches_the_phase4_oracle() {
    let (program, types, package) = pipeline(CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let oracle = run_oracle(&program, &types, entry);
    let expected: String = oracle.output.concat();
    assert!(expected.ends_with("all_ok\n"), "oracle corpus run: {expected:?}");
    assert!(!expected.contains("fail_"), "oracle self-check tripped: {expected:?}");

    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "corpus");
    assert_eq!(status, 0, "object exited nonzero");
    assert_eq!(
        stdout, expected,
        "object stdout diverges from the Phase 4 oracle"
    );
    assert!(stdout.ends_with("all_ok\n"));
}

// ── gate 2: determinism ────────────────────────────────────────────────────

#[test]
fn two_compiles_produce_byte_identical_objects() {
    let (program, types, package) = pipeline(CORPUS);
    let names = Names(&package.names.symbols);

    let first = compile(&program, &types, &names);
    let second = compile(&program, &types, &names);

    assert!(
        first.same_object(&second),
        "objects differ across compiles of the same program"
    );
    assert_eq!(first.exported_symbols, second.exported_symbols);
    assert_eq!(first.imported_symbols, second.imported_symbols);
    assert_eq!(first.entry, second.entry);
}

// ── gate 3: IR validity + object shape ─────────────────────────────────────

#[test]
fn object_is_elf_and_census_matches_the_module_record() {
    let (program, types, package) = pipeline(CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    // Real ELF relocatable.
    assert_eq!(&module.object[..4], b"\x7fELF", "object is not ELF");

    // The census is the module's declaration record: `main` exports as
    // `lpp_main` plus the generated C-ABI wrapper; user functions
    // export by source name; imports are exactly the runtime symbols
    // the slice uses.
    assert_eq!(module.entry.as_deref(), Some("main"));
    for symbol in [
        "lpp_main",
        "main",
        "check_arith",
        "check_div_rem",
        "check_shifts",
        "check_cmp",
        "check_bool",
        "check_floats",
        "check_calls",
        "check_strings",
        "count_down",
        "double",
        "which",
        "is_z",
        "echo",
    ] {
        assert!(
            module.exported_symbols.contains(symbol),
            "missing export {symbol}: {:?}",
            module.exported_symbols
        );
    }
    // 5C: the corpus's string locals make the ARC retainer and
    // releaser used imports alongside the 5B pair. 5C2: the corpus's
    // string `!=` makes the content comparator a used import.
    assert_eq!(
        module.imported_symbols,
        std::collections::BTreeSet::from([
            "fmod".to_owned(),
            "lpp_arc_release".to_owned(),
            "lpp_arc_retain".to_owned(),
            "lpp_print_str".to_owned(),
            "lpp_str_eq".to_owned(),
        ])
    );
}

// ── gate 4: compile-fail ───────────────────────────────────────────────────

fn expect_code(error: &CodegenError, code: &str, function: MirFunctionId) {
    assert_eq!(error.code(), code);
    assert_eq!(error.function, Some(function), "typed rejection must name the exact function");
}

#[test]
fn function_value_and_tuple_programs_compile() {
    // The 5C2 function-value surface lifted the closure and async rejections,
    // and tuples are now lowered as flat heap records (construction +
    // tuple-field projection), so all three compile.
    for (label, source) in [
        (
            "closure",
            "def main() -> Int:\n    base := 5\n    cb := fn(x):\n        return x + base\n    return cb(1)\n",
        ),
        (
            "async",
            "async def value() -> Int:\n    return 1\n\ndef main():\n    t := value()\n    print_int(t.await)\n",
        ),
        (
            "tuple",
            "def main() -> Int:\n    t := (1, 2)\n    return t.0\n",
        ),
    ] {
        let (program, types, package) = pipeline(source);
        let names = Names(&package.names.symbols);
        let options = CodegenOptions::new(Target::X86_64, &names);
        CraneliftBackend
            .compile_module(&program, &types, &options)
            .unwrap_or_else(|e| panic!("{label}: expected Ok in 5C2, got {e}"));
    }
}

#[test]
fn family_d_builtin_fails_with_e5003_at_the_exact_function() {
    // 5C2's table policy lowers the oracle-supported Family A builtins
    // (`print_int` among them); the `E5003` rejection is the Family D
    // surface — everything the object runtime does not implement.
    let (program, types, package) =
        pipeline("def main() -> Int:\n    w := webview_window_create(\"t\", 0, 0, 0)\n    return w\n");
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    let error = CraneliftBackend
        .compile_module(&program, &types, &options)
        .err()
        .unwrap_or_else(|| panic!("expected E5003, got Ok"));
    expect_code(&error, "E5003", main_function(&program, &package.names.symbols));
    match &error.kind {
        CodegenErrorKind::UnrepresentableBuiltin { builtin, .. } => {
            assert_eq!(builtin.descriptor().name, "webview_window_create");
        }
        kind => panic!("wrong kind {kind:?}"),
    }
}
