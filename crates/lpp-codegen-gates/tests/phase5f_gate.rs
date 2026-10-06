//! Phase 5F — the cross-backend safety exit.
//!
//! The final Phase 5 slice is a validation gate, not new lowering. It drives
//! all three backends (Cranelift `X86_64`, WASM `Wasm32Wasi`, LLVM `X86_64`)
//! on the **shared scalar corpus** — the data surface every slice-1 backend
//! supports — so a backend-specific divergence surfaces as a difference
//! against the reference oracle.
//!
//! 1. **Cross-backend corpus differential.** Each backend's stdout, executed
//!    natively (Cranelift/LLVM linked against the 5B `c_shim.c`; WASM under
//!    Node `node:wasi`), must equal the plain reference oracle
//!    (`execute_mir_with_stats`), carry no `fail_*` marker, and end in
//!    `all_ok`.
//! 2. **Deterministic object-size baseline.** Two compiles per backend
//!    produce byte-identical objects (and identical censuses); the object
//!    size is stable and non-empty.
//! 3. **Sanitizer run.** The native backends (Cranelift, LLVM) are re-linked
//!    with `-fsanitize=address,undefined` and executed; a clean exit with no
//!    sanitizer report is required.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use lpp_codegen_api::{Backend, CodegenOptions, CompiledModule, NameResolver, Target};
use lpp_codegen_cranelift::CraneliftBackend;
use lpp_codegen_llvm::LlvmBackend;
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{ExecutionOutcome, InterpreterLimits, MirFunctionId, MirProgram, build_mir};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

// ── scaffolding (mirrors the 5B/5D/5E gates) ───────────────────────────────

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
    let filesystem = MemoryFileSystem::new(source, "/p5f");
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/p5f/main.lpp",
            PackageSpec::new("p5f", "/p5f"),
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

// ── the three backends, driven through the common `Backend` trait ──────────

fn llvm_compiler_available() -> bool {
    let compiler = std::env::var("LPP_LLVM_CC")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "clang".to_string());
    Command::new(compiler)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn compile_cranelift(
    program: &MirProgram,
    types: &TypeInterner,
    names: &Names<'_>,
) -> CompiledModule {
    CraneliftBackend
        .compile_module(program, types, &CodegenOptions::new(Target::X86_64, names))
        .unwrap_or_else(|e| panic!("cranelift compile failed: {e}"))
}

fn compile_llvm(program: &MirProgram, types: &TypeInterner, names: &Names<'_>) -> CompiledModule {
    LlvmBackend
        .compile_module(program, types, &CodegenOptions::new(Target::X86_64, names))
        .unwrap_or_else(|e| panic!("llvm compile failed: {e}"))
}

fn compile_wasm(program: &MirProgram, types: &TypeInterner, names: &Names<'_>) -> CompiledModule {
    WasmBackend
        .compile_module(
            program,
            types,
            &CodegenOptions::new(Target::Wasm32Wasi, names),
        )
        .unwrap_or_else(|e| panic!("wasm compile failed: {e}"))
}

fn workdir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp5f_{}_{}", std::process::id(), test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn c_shim() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("lpp-codegen-cranelift")
        .join("tests")
        .join("c_shim.c")
}

/// Link a native object against the 5B `c_shim.c` runtime and run it;
/// returns (stdout, exit status).
fn link_and_run(object: &[u8], test_name: &str) -> (String, i32) {
    let dir = workdir(test_name);
    let module = dir.join("module.o");
    let bin = dir.join("program");
    std::fs::write(&module, object).unwrap();
    let link = Command::new("cc")
        .arg(&module)
        .arg(c_shim())
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

/// Run a wasm object under Node `node:wasi`; returns (stdout, exit status).
fn run_wasm(object: &[u8], test_name: &str) -> (String, i32) {
    let dir = workdir(test_name);
    let wasm = dir.join("module.wasm");
    let host = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("lpp-codegen-wasm")
        .join("tests")
        .join("wasm_run.mjs");
    std::fs::write(&wasm, object).unwrap();
    let run = Command::new("node")
        .arg(&host)
        .arg(&wasm)
        .output()
        .unwrap_or_else(|e| panic!("node failed to start ({e}); is Node.js on PATH?"));
    (
        String::from_utf8_lossy(&run.stdout).to_string(),
        run.status.code().unwrap_or(-1),
    )
}

fn check_markers(stdout: &str, backend: &str) {
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("fail_"))
            .count(),
        0,
        "{backend}: fail marker present:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line == "all_ok"),
        "{backend}: all_ok marker missing:\n{stdout}"
    );
}

// ── the shared scalar corpus (the same data surface as 5B/5D/5E) ────────────

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
    # 5E2a: float printing is now cross-backend (Cranelift/LLVM via the C
    # runtime's printf "%f", WASM via its {:.6} helper, oracle via {:.6}).
    # Exactly-representable values, so all three formats agree byte-for-byte.
    print_float(3.5)
    print_float(-0.5)
    print_float(0.0)
    print_float(2.25)

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
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "the experimental v0.1 LLVM object tier is gated on Linux"
)]
fn cross_backend_corpus_agrees() {
    if !llvm_compiler_available() {
        eprintln!("skipping cross-backend object gate: no configured clang executable");
        return;
    }
    let (program, types, package) = pipeline(DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);
    let expected = execute_mir(&program, &types, entry).output.concat();

    // Cranelift (native).
    let cl = compile_cranelift(&program, &types, &names);
    assert_eq!(cl.target, Target::X86_64);
    let (cl_out, cl_status) = link_and_run(&cl.object, "x_cl");
    assert_eq!(cl_status, 0, "cranelift exited non-zero:\n{cl_out}");
    check_markers(&cl_out, "cranelift");
    assert_eq!(
        cl_out, expected,
        "cranelift stdout diverged from the oracle"
    );

    // LLVM (native).
    let ll = compile_llvm(&program, &types, &names);
    assert_eq!(ll.target, Target::X86_64);
    let (ll_out, ll_status) = link_and_run(&ll.object, "x_ll");
    assert_eq!(ll_status, 0, "llvm exited non-zero:\n{ll_out}");
    check_markers(&ll_out, "llvm");
    assert_eq!(ll_out, expected, "llvm stdout diverged from the oracle");

    // WASM (node:wasi).
    let ws = compile_wasm(&program, &types, &names);
    assert_eq!(ws.target, Target::Wasm32Wasi);
    let (ws_out, ws_status) = run_wasm(&ws.object, "x_ws");
    assert_eq!(ws_status, 0, "wasm exited non-zero:\n{ws_out}");
    check_markers(&ws_out, "wasm");
    assert_eq!(ws_out, expected, "wasm stdout diverged from the oracle");

    // All three agree with each other (and with the oracle).
    assert_eq!(cl_out, ll_out, "cranelift and llvm diverge");
    assert_eq!(cl_out, ws_out, "cranelift and wasm diverge");
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "the experimental v0.1 LLVM object tier is gated on Linux"
)]
fn deterministic_object_sizes() {
    if !llvm_compiler_available() {
        eprintln!("skipping cross-backend object gate: no configured clang executable");
        return;
    }
    let (program, types, package) = pipeline(DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);

    // Each backend is compiled twice; the objects must be byte-identical and
    // non-empty. The recorded size is the deterministic size baseline.
    let cases: &[(
        &str,
        fn(&MirProgram, &TypeInterner, &Names<'_>) -> CompiledModule,
    )] = &[
        ("cranelift", compile_cranelift),
        ("llvm", compile_llvm),
        ("wasm", compile_wasm),
    ];
    for (label, compile) in cases {
        let a = compile(&program, &types, &names);
        let b = compile(&program, &types, &names);
        let size = a.object.len();
        assert!(size > 0, "{label}: empty object");
        assert_eq!(
            a.object, b.object,
            "{label}: two compiles differ byte-for-byte"
        );
        assert_eq!(
            a.object.len(),
            b.object.len(),
            "{label}: object size not stable"
        );
        assert_eq!(
            a.exported_symbols, b.exported_symbols,
            "{label}: export censuses differ"
        );
        assert_eq!(
            a.imported_symbols, b.imported_symbols,
            "{label}: import censuses differ"
        );
        eprintln!("5F size baseline: {label} object = {size} bytes");
    }
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "the experimental v0.1 LLVM object tier is gated on Linux"
)]
fn sanitizer_clean() {
    if !llvm_compiler_available() {
        eprintln!("skipping sanitizer gate: no configured clang executable");
        return;
    }
    let (program, types, package) = pipeline(DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);

    // The native backends are re-linked with ASan+UBSan and executed; a clean
    // exit with no sanitizer report is required.
    let cases: &[(
        &str,
        fn(&MirProgram, &TypeInterner, &Names<'_>) -> CompiledModule,
    )] = &[("cranelift", compile_cranelift), ("llvm", compile_llvm)];
    for (label, compile) in cases {
        let module = compile(&program, &types, &names);
        let dir = workdir(&format!("san_{label}"));
        let obj = dir.join("module.o");
        let bin = dir.join("program");
        std::fs::write(&obj, &module.object).unwrap();
        let link = Command::new("cc")
            .arg(&obj)
            .arg(c_shim())
            .arg("-o")
            .arg(&bin)
            .arg("-lm")
            .args(["-fsanitize=address,undefined"])
            .output()
            .unwrap_or_else(|e| panic!("cc spawn: {e}"));
        assert!(
            link.status.success(),
            "{label}: sanitized link failed:\n{}",
            String::from_utf8_lossy(&link.stderr)
        );
        let run = Command::new(&bin).output().unwrap();
        let stderr = String::from_utf8_lossy(&run.stderr);
        let status = run.status.code().unwrap_or(-1);
        assert!(
            !stderr.contains("AddressSanitizer") && !stderr.contains("runtime error"),
            "{label}: sanitizer report:\n{stderr}"
        );
        assert_eq!(
            status, 0,
            "{label}: sanitized run exited non-zero (see stderr)"
        );
    }
}
