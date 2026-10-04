//! Phase 5D gate — the hand-written `wasm32-wasi` backend is correct and
//! deterministic.
//!
//! Slice 1 (data surface):
//! 1. **Differential execution (data-surface corpus).** Integers
//!    (wrapping arithmetic, signed div/rem, bitwise, shifts including
//!    large/negative amounts, all comparisons), booleans, chars, floats
//!    (arithmetic, modulo, NaN, comparisons), control flow, direct calls
//!    (multi-parameter, recursion), and the string surface — the object's
//!    stdout, run under Node's `node:wasi`, equals the Phase 4 reference
//!    oracle (`execute_mir_with_stats`).
//! 2. **Determinism.** Two compiles produce byte-identical objects and
//!    identical symbol censuses.
//! 3. **Compile-fail (5D2b surface).** Async/await/spawn and bare
//!    function values fail with the exact `E5001`; slices, SIMD, and
//!    tuples likewise. (Sync closures compile as of 5D2b slice 1.)
//!
//! Slice 2 (5D2a, managed data surface):
//! 4. **Differential execution (managed corpus).** Structs (construction,
//!    field read/write, nesting, reassignment, mixed-width layout,
//!    struct-in-struct, managed fields), enums (dense match dispatch,
//!    payload binding, explicit arms), lists (literals of every element
//!    class, element read/write, `for`-in, managed elements, nested
//!    lists, list-in-struct), float printing (`{:.6}` byte-exact), and
//!    cross-function passing: object stdout equals the Phase 4 reference
//!    oracle.
//! 5. **Differential execution (ARC stress corpus).** Aliasing,
//!    self-assignment, field/element swaps, move-then-reassign, cross-
//!    container stores, deep nesting: object stdout equals the ARC-mode
//!    oracle (`execute_mir_arc` with the 4D `pinned_types()`), whose
//!    end-of-run balance proof validates the interpreter's ARC model for
//!    the same programs the object executes.
//! 6. **Determinism (5D2a).** Both corpora compile byte-identically.
//! 7. **Structure census.** Exports exactly `_start` + `memory` + the
//!    user functions; imports exactly `fd_write` + `proc_exit`; the name
//!    section carries exactly one generated destructor per nominal
//!    (`lpp_drop_s{n}`/`lpp_drop_e{n}`, in `MirAggregateId` order) plus
//!    `lpp_drop_list` and `lpp_drop_none`; the funcref table is
//!    `[no-op, drops…, list…]` with the element section matching.
//! 8. **Compile-fail (5D2b surface, 5D2a corpus context).** Async fails
//!    with the exact `E5001` even in a struct program.
//!
//! Slice 3 (5D2b slice 1, function values):
//! 9. **Differential execution (closure corpus).** Value captures (a
//!    `mut` capture is a cell written through the env), capsule copies
//!    called through the dispatch table, closures stored in lists, and
//!    two-parameter closure calls: object stdout equals the ARC-mode
//!    oracle (`execute_mir_arc` with the 4D `pinned_types()`).
//! 10. **Compile-fail (remaining 5D2b surface).** Async and tuples
//!     stay `E5001`; the closure smoke program compiles and runs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{Backend, CodegenOptions, CompiledModule, NameResolver, Target};
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{ExecutionOutcome, InterpreterLimits, MirAggregateId, MirFunctionId, MirProgram, build_mir};
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
    lpp_mir::execute_mir_arc(program, types, entry, &[], InterpreterLimits::default(), &pinned)
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
    (
        String::from_utf8_lossy(&run.stdout).into_owned(),
        status,
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

// ── slice 1: data-surface corpus ───────────────────────────────────────────
//
// Follows the 5B style: a condition that a *correct* compiler must satisfy
// prints a `fail_*` marker on violation. The object and the oracle therefore
// print the same markers; any miscompile changes which markers appear.

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

// ── 5D2a: managed data surface corpus ──────────────────────────────────────
//
// The 5C aggregate corpus (structs, enums, lists, cross-function passing)
// plus the byte-exact `{:.6}` float-print surface, differential-tested
// against the Phase 4 oracle.

const MANAGED_DATA_SURFACE_CORPUS: &str = r#"
struct Point:
    x: Int
    y: Int
struct Box:
    origin: Point
    size: Int
    tag: Char
    flag: Bool
struct Outer:
    inner: Point
    label: Str
    items: List[Point]
struct L1:
    next: L2
struct L2:
    deep: L3
    pad: Int
struct L3:
    value: Int
enum Shape:
    Square(size: Int)
    Circle(radius: Int)
    Blank
enum Wrap:
    Plain(n: Int)
    Fancy(point: Point)
    Empty
def area(kind: Shape) -> Int:
    mut answer := 0
    match kind:
        Square(size):
            answer = size * size
        Circle(radius):
            answer = radius * 3
        Blank:
            answer = 0
    return answer
def wrap_value(w: Wrap) -> Int:
    mut answer := 0
    match w:
        Plain(n):
            answer = n
        Fancy(point):
            answer = point.x + point.y
        Empty:
            answer = 0
    return answer
def check_shapes() -> Int:
    mut total := area(Shape.Square(3))
    total = total + area(Shape.Circle(2))
    total = total + area(Shape.Blank)
    probe := Shape.Square(7)
    total = total + area(probe)
    return total
def check_structs() -> Int:
    p := Point(3, 4)
    mut total := p.x + p.y
    q := p
    total = total + q.x
    mut r := Box(Point(1, 2), 5, 'k', true)
    total = total + r.origin.x
    total = total + r.origin.y
    total = total + r.size
    r.origin = Point(10, 20)
    total = total + r.origin.x
    total = total + r.origin.y
    r.size = 6
    r.tag = 'm'
    if r.tag == 'm':
        total = total + 1
    if r.flag:
        total = total + 1
    if not r.flag:
        total = total + 1000
    return total
def check_managed_struct() -> Int:
    mut s := Outer(Point(1, 2), "hello", [Point(3, 4), Point(5, 6)])
    mut total := s.inner.x
    total = total + s.items[0].y
    total = total + s.items[1].x
    s.inner = Point(9, 9)
    total = total + s.inner.x
    s.items[0] = Point(7, 8)
    total = total + s.items[0].x
    mut t := s
    total = total + t.inner.y
    t.label = "world"
    total = total + s.inner.x
    return total
def check_deep() -> Int:
    mut d := L1(L2(L3(41), 2))
    mut total := d.next.deep.value
    total = total + d.next.pad
    d.next.pad = 5
    total = total + d.next.pad
    return total
def check_lists() -> Int:
    mut xs := [10, 20, 30]
    mut total := xs[0] + xs[1]
    total = total + xs[2]
    xs[1] = 25
    total = total + xs[1]
    mut sum := 0
    for v in xs:
        sum = sum + v
    total = total + sum
    ys := [4, 5]
    total = total + ys[0] + ys[1]
    return total
def check_list_float() -> Float:
    mut xs := [1.5, 2.25]
    mut total := xs[0]
    total = total + xs[1]
    xs[0] = 4.5
    total = total + xs[0]
    return total
def check_list_char() -> Int:
    xs := ['a', 'b']
    mut total := 0
    if xs[0] == 'a':
        total = total + 1
    if xs[1] == 'b':
        total = total + 1
    return total
def check_list_bool() -> Int:
    mut xs := [true, false]
    mut total := 0
    if xs[0]:
        total = total + 1
    if not xs[1]:
        total = total + 1
    xs[1] = true
    if xs[1]:
        total = total + 1
    return total
def check_nested() -> Int:
    xs := [[1, 2], [3, 4]]
    mut total := xs[0][0]
    total = total + xs[0][1]
    total = total + xs[1][0]
    total = total + xs[1][1]
    return total
def check_list_struct() -> Int:
    mut xs := [Point(1, 1), Point(2, 2)]
    mut total := xs[0].x
    total = total + xs[0].y
    total = total + xs[1].x
    xs[1] = Point(5, 6)
    total = total + xs[1].y
    return total
def describe(p: Point, xs: List[Int], kind: Shape) -> Int:
    mut total := p.x + p.y
    total = total + xs[0]
    total = total + area(kind)
    return total
def check_calls() -> Int:
    mut total := describe(Point(3, 4), [7, 8], Shape.Square(2))
    total = total + describe(Point(1, 1), [100], Shape.Blank)
    return total
def check_wrap() -> Int:
    mut total := wrap_value(Wrap.Plain(7))
    total = total + wrap_value(Wrap.Fancy(Point(2, 3)))
    total = total + wrap_value(Wrap.Empty)
    held := Wrap.Fancy(Point(4, 5))
    total = total + wrap_value(held)
    return total
def check_float_print():
    print(1.5)
    print(-8.25)
    print(0.1)
    print(8.0 / 3.0)
    print(1000000000000000.0)
    print(-0.0)
    print_float(2.05)
    print_float(-0.0)
    print_float(8.0 / 3.0)

def main() -> Int:
    if check_shapes() == 64:
        print_str("ok_shapes")
    else:
        print_str("fail_shapes")
    if check_structs() == 50:
        print_str("ok_structs")
    else:
        print_str("fail_structs")
    if check_managed_struct() == 44:
        print_str("ok_managed_struct")
    else:
        print_str("fail_managed_struct")
    if check_deep() == 48:
        print_str("ok_deep")
    else:
        print_str("fail_deep")
    if check_lists() == 159:
        print_str("ok_lists")
    else:
        print_str("fail_lists")
    if check_list_float() == 8.25:
        print_str("ok_list_float")
    else:
        print_str("fail_list_float")
    if check_list_char() == 2:
        print_str("ok_list_char")
    else:
        print_str("fail_list_char")
    if check_list_bool() == 3:
        print_str("ok_list_bool")
    else:
        print_str("fail_list_bool")
    if check_nested() == 10:
        print_str("ok_nested")
    else:
        print_str("fail_nested")
    if check_list_struct() == 10:
        print_str("ok_list_struct")
    else:
        print_str("fail_list_struct")
    if check_calls() == 120:
        print_str("ok_calls")
    else:
        print_str("fail_calls")
    if check_wrap() == 21:
        print_str("ok_wrap")
    else:
        print_str("fail_wrap")
    check_float_print()
    print_str("all_ok")
    return 0
"#;

// ── 5D2a: ARC stress corpus ────────────────────────────────────────────────
//
// The 5C ARC stress programs: aliasing, self-assignment, swaps,
// move-then-reassign, cross-container stores, deep nesting.

const ARC_STRESS_CORPUS: &str = r#"
struct Point:
    x: Int
    y: Int
struct Box:
    origin: Point
    size: Int
    tag: Char
    flag: Bool
struct Holder:
    value: Int
    items: List[Holder]
struct L1:
    next: L2
struct L2:
    deep: L3
    pad: Int
struct L3:
    value: Int
def check_aliasing() -> Int:
    p := Point(5, 5)
    mut total := 0
    q := p
    r := q
    total = total + p.x
    total = total + q.x
    total = total + r.y
    s := Box(p, 1, 'a', false)
    total = total + s.origin.x
    return total
def check_self_assign() -> Int:
    mut p := Point(3, 7)
    mut s := Box(Point(1, 1), 1, 'a', false)
    mut total := 0
    total = total + s.origin.x
    s.origin = s.origin
    total = total + s.origin.y
    q := p
    p = q
    total = total + p.x
    return total
def check_swap() -> Int:
    mut a := Point(1, 10)
    mut b := Point(2, 20)
    mut tmp := a
    a = b
    b = tmp
    mut total := a.x
    total = total + b.x
    total = total + a.y
    total = total + b.y
    return total
def check_move_reassign() -> Int:
    mut p := Point(1, 1)
    mut holder := Box(p, 9, 'z', true)
    mut total := 0
    total = total + holder.origin.x
    holder.origin = Point(4, 5)
    total = total + holder.origin.x
    total = total + holder.origin.y
    p = Point(8, 8)
    total = total + p.x
    return total
def check_xstore() -> Int:
    mut a := Box(Point(1, 10), 1, 'a', false)
    mut b := Box(Point(2, 20), 2, 'b', true)
    mut total := 0
    a.origin = b.origin
    total = total + a.origin.x
    total = total + b.origin.y
    total = total + a.origin.y
    total = total + b.origin.x
    return total
def check_deep_nest() -> Int:
    d := L1(L2(L3(41), 2))
    mut e := L1(L2(L3(100), 3))
    mut total := d.next.deep.value
    total = total + e.next.deep.value
    total = total + d.next.pad + e.next.pad
    return total
def main() -> Int:
    if check_aliasing() == 20:
        print_str("ok_aliasing")
    else:
        print_str("fail_aliasing")
    if check_self_assign() == 5:
        print_str("ok_self_assign")
    else:
        print_str("fail_self_assign")
    if check_swap() == 33:
        print_str("ok_swap")
    else:
        print_str("fail_swap")
    if check_move_reassign() == 18:
        print_str("ok_move_reassign")
    else:
        print_str("fail_move_reassign")
    if check_xstore() == 44:
        print_str("ok_xstore")
    else:
        print_str("fail_xstore")
    if check_deep_nest() == 146:
        print_str("ok_deep_nest")
    else:
        print_str("fail_deep_nest")
    print_str("all_ok")
    return 0
"#;

// ── wasm binary inspection (census) ────────────────────────────────────────

/// Unsigned LEB128 decode.
fn uleb(bytes: &[u8], at: &mut usize) -> u64 {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = bytes[*at];
        *at += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    result
}

/// Parse the custom "name" section (function names) and the funcref table
/// size from the raw wasm object.
fn parse_names_and_table(object: &[u8]) -> (BTreeMap<String, u32>, Option<u32>, Option<Vec<u32>>) {
    assert_eq!(&object[0..4], b"\0asm", "not a wasm module");
    let mut at = 8usize; // magic + version
    let mut names: BTreeMap<String, u32> = BTreeMap::new();
    let mut table_min: Option<u32> = None;
    let mut table_elems: Option<Vec<u32>> = None;
    while at < object.len() {
        let id = object[at];
        at += 1;
        let len = uleb(object, &mut at) as usize;
        let payload = &object[at..at + len];
        at += len;
        if id == 0 {
            // Custom section: "name" + payload.
            let mut p = 0usize;
            let name_len = uleb(payload, &mut p) as usize;
            let section_name = &payload[p..p + name_len];
            p += name_len;
            if section_name == b"name" {
                while p < payload.len() {
                    let sub = payload[p];
                    p += 1;
                    let sub_len = uleb(payload, &mut p) as usize;
                    let sub_end = p + sub_len;
                    if sub == 1 {
                        // Function names.
                        let mut q = p;
                        let count = uleb(payload, &mut q) as usize;
                        for _ in 0..count {
                            let nlen = uleb(payload, &mut q) as usize;
                            let n = String::from_utf8(payload[q..q + nlen].to_vec()).unwrap();
                            q += nlen;
                            let func_idx = uleb(payload, &mut q) as u32;
                            names.insert(n, func_idx);
                        }
                    }
                    p = sub_end;
                }
            }
        } else if id == 4 {
            // Table section: one funcref table, min size.
            let mut p = 0usize;
            let count = uleb(payload, &mut p) as usize;
            assert_eq!(count, 1, "exactly one table");
            let elem_type = payload[p];
            p += 1;
            assert_eq!(elem_type, 0x70, "funcref table");
            let flags = payload[p];
            p += 1;
            assert_eq!(flags, 0x00, "min-only table limits");
            table_min = Some(uleb(payload, &mut p) as u32);
        } else if id == 9 {
            // Element section: the table initialization.
            let mut p = 0usize;
            let count = uleb(payload, &mut p) as usize;
            assert_eq!(count, 1, "exactly one element segment");
            let mode = payload[p];
            p += 1;
            assert_eq!(mode, 0x00, "active segment, table 0");
            // Init expr: i32.const 0; end
            let mut q = p;
            assert_eq!(payload[q], 0x41);
            q += 1;
            let mut _s = 0i64;
            let mut byte = payload[q] as i64;
            q += 1;
            _s = (byte & 0x7f) as i64;
            while payload[q - 1] & 0x80 != 0 {
                byte = payload[q] as i64;
                _s |= (byte & 0x7f) << 7;
                q += 1;
            }
            assert_eq!(payload[q], 0x0b, "init expr end");
            q += 1;
            let n = uleb(payload, &mut q) as usize;
            let mut elems = Vec::with_capacity(n);
            for _ in 0..n {
                elems.push(uleb(payload, &mut q) as u32);
            }
            table_elems = Some(elems);
        }
    }
    (names, table_min, table_elems)
}

// ── tests ──────────────────────────────────────────────────────────────────

#[test]
fn data_surface_matches_oracle() {
    let (program, types, package) = pipeline(DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let oracle = run_oracle(&program, &types, entry);
    let expected = oracle.output.concat();

    let module = compile(&program, &types, &names);
    assert_eq!(module.target, Target::Wasm32Wasi);
    let (stdout, status) = run_wasm(&module.object, "data_surface");
    assert_eq!(status, 0, "wasm exited non-zero:\n{stdout}");
    expect_success_markers(&stdout, "data_surface");
    assert_eq!(
        stdout, expected,
        "object stdout diverged from the reference oracle"
    );
}

#[test]
fn deterministic_object() {
    for (label, corpus) in [
        ("data_surface", DATA_SURFACE_CORPUS),
        ("managed", MANAGED_DATA_SURFACE_CORPUS),
        ("arc_stress", ARC_STRESS_CORPUS),
    ] {
        let (program, types, package) = pipeline(corpus);
        let names = Names(&package.names.symbols);
        let a = compile(&program, &types, &names);
        let b = compile(&program, &types, &names);
        assert_eq!(a.object, b.object, "{label}: two compiles differ byte-for-byte");
        assert_eq!(
            a.exported_symbols, b.exported_symbols,
            "{label}: export censuses differ"
        );
        assert_eq!(
            a.imported_symbols, b.imported_symbols,
            "{label}: import censuses differ"
        );
    }
}

// ── 5D2a gate 1: differential managed corpus ───────────────────────────────

#[test]
fn managed_corpus_matches_the_reference_oracle() {
    let (program, types, package) = pipeline(MANAGED_DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "managed");
    assert_eq!(status, 0, "managed corpus object exited {status}:\n{stdout}");

    let oracle = run_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "managed corpus: object stdout diverges from the oracle"
    );
    expect_success_markers(&stdout, "managed");
}

// ── 5D2a gate 2: differential ARC stress corpus ────────────────────────────

#[test]
fn arc_stress_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(ARC_STRESS_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "arc_stress");
    assert_eq!(status, 0, "arc stress object exited {status}:\n{stdout}");

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "arc stress corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "arc stress");
}

// ── 5D2a gate 3: structure census ──────────────────────────────────────────

#[test]
fn object_census_matches_the_contract() {
    let (program, types, package) = pipeline(MANAGED_DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    // Exports: _start, memory, and the user functions — nothing else.
    let mut expected_exports = BTreeSet::from(["_start".to_string(), "memory".to_string()]);
    for (_, function) in program.functions() {
        if let Some(symbol) = function.name
            && let Some(name) = package.names.symbols.resolve(symbol)
        {
            expected_exports.insert(name.to_string());
        }
    }
    assert_eq!(
        module.exported_symbols, expected_exports,
        "export census diverged"
    );
    assert_eq!(
        module.imported_symbols,
        BTreeSet::from([
            "wasi_snapshot_preview1.fd_write".to_string(),
            "wasi_snapshot_preview1.proc_exit".to_string(),
        ]),
        "import census diverged"
    );

    // The name section: one destructor per nominal, in MirAggregateId
    // order, plus the list destroyer and the no-op.
    let (name_section, table_min, table_elems) = parse_names_and_table(&module.object);
    let aggregates: Vec<MirAggregateId> = program.aggregates().map(|(id, _)| id).collect();
    let kinds: Vec<_> = aggregates
        .iter()
        .map(|&id| program.aggregate(id).unwrap().kind)
        .collect();
    let expected_drops: BTreeSet<String> = aggregates
        .iter()
        .enumerate()
        .map(|(pos, &id)| {
            let tag = match kinds[pos] {
                lpp_mir::MirAggregateKind::Struct => 's',
                lpp_mir::MirAggregateKind::Enum => 'e',
            };
            format!("lpp_drop_{tag}{}", id.raw())
        })
        .chain(std::iter::once("lpp_drop_list".to_string()))
        .chain(std::iter::once("lpp_drop_none".to_string()))
        .collect();
    let drop_names_in_section: BTreeSet<String> = name_section
        .keys()
        .filter(|name| name.starts_with("lpp_drop_"))
        .cloned()
        .collect();
    assert_eq!(
        drop_names_in_section, expected_drops,
        "drop function names diverged"
    );

    // The table: [no-op, drops in MirAggregateId order, list].
    let expected_table_len = 1 + aggregates.len() as u32 + 1;
    assert_eq!(table_min, Some(expected_table_len), "funcref table size");
    let elems = table_elems.expect("element section initializes the table");
    assert_eq!(elems.len() as u32, expected_table_len, "table element count");
    assert_eq!(
        name_section.get("lpp_drop_none"),
        Some(&elems[0]),
        "table slot 0 is the no-op"
    );
    for (pos, &id) in aggregates.iter().enumerate() {
        let tag = match kinds[pos] {
            lpp_mir::MirAggregateKind::Struct => 's',
            lpp_mir::MirAggregateKind::Enum => 'e',
        };
        let name = format!("lpp_drop_{tag}{}", id.raw());
        assert_eq!(
            name_section.get(&name),
            Some(&elems[pos + 1]),
            "drop {name} is not tabled in MirAggregateId order"
        );
    }
    // Table = [no-op, drop0..drop_{n-1}, list]: the list destroyer
    // occupies slot 1 + n.
    assert_eq!(
        name_section.get("lpp_drop_list"),
        Some(&elems[aggregates.len() + 1]),
        "list destroyer is the last table entry"
    );
}

// ── 5D2b slice 1: closure corpus (closures + capsule calls) ────────────────
//
// The synchronous subset of the 5C2 closure corpus: value captures
// (cells for mutables), capsule copies called through the dispatch
// table, closures stored in lists, and two-parameter closure calls.
// Expected marker values (hand-computed, identical to the 5C2 gate):
//   counter 5, list_closure 42, nested 35, managed_capture 19,
//   sync_value 42, cycle 108, managed_read_only 13, minimal 3,
//   call_twice 6, no_closure 3.
//
// Corpus-domain rule (from the 5C2 contract): the cycle closure's
// function type (two parameters) is shared by no other closure in
// the program, so the 4D-pinned type cannot pin an unrelated capsule.

const CLOSURE_CORPUS: &str = r#"
struct Point:
    x: Int
    y: Int

def counter_check() -> Int:
    mut count := 0
    counter := fn() -> Int:
        count = count + 1
        return count
    first_call := counter()
    second_call := counter()
    return count + first_call + second_call

def list_closure_check() -> Int:
    add_one := fn(value: Int) -> Int: value + 1
    callbacks := [add_one]
    callback := list_get(callbacks, 0)
    return callback(41)

def nested_capture_check() -> Int:
    base := 10
    add := fn(x: Int) -> Int:
        return base + x
    first_call := add(5)
    return first_call + add(10)

def managed_capture_check() -> Int:
    mut p := Point(3, 7)
    bump := fn() -> Int:
        p = Point(p.x + 1, p.y)
        return p.y
    a := bump()
    b := bump()
    return a + b + p.x

def sync_value_check() -> Int:
    double := fn(x: Int) -> Int: x * 2
    f := double
    return f(21)

def cycle_check() -> Int:
    mut total := 0
    xs := list_new()
    f := fn(seed: Int, extra: Int) -> Int:
        total = total + list_len(xs)
        return total + seed + extra
    g := f
    list_push(xs, f)
    return g(100, 7)

def managed_read_only() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x + p.y
    return read() + p.x

def minimal_capture() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x
    return read()

def capture_call_twice() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x
    a := read()
    b := read()
    return a + b

def no_closure() -> Int:
    p := Point(3, 7)
    return p.x

def main() -> Int:
    if counter_check() == 5:
        print_str("ok_counter")
    else:
        print_str("fail_counter")
    if list_closure_check() == 42:
        print_str("ok_list_closure")
    else:
        print_str("fail_list_closure")
    if nested_capture_check() == 35:
        print_str("ok_nested")
    else:
        print_str("fail_nested")
    if managed_capture_check() == 19:
        print_str("ok_managed_capture")
    else:
        print_str("fail_managed_capture")
    if sync_value_check() == 42:
        print_str("ok_sync_value")
    else:
        print_str("fail_sync_value")
    if cycle_check() == 108:
        print_str("ok_cycle")
    else:
        print_str("fail_cycle")
    if managed_read_only() == 13:
        print_str("ok_managed_read_only")
    else:
        print_str("fail_managed_read_only")
    if minimal_capture() == 3:
        print_str("ok_minimal")
    else:
        print_str("fail_minimal")
    if capture_call_twice() == 6:
        print_str("ok_call_twice")
    else:
        print_str("fail_call_twice")
    if no_closure() == 3:
        print_str("ok_no_closure")
    else:
        print_str("fail_no_closure")
    print_str("all_ok")
    return 0
"#;

// ── 5D2b slice 2: bare function values ─────────────────────────────────────
//
// A bare function value (`f := add`) is a 16-byte capsule
// `[code, env=NULL]` on the ARC heap; it dispatches through the same
// table and call types as a closure. The oracle (ARC-mode interpreter)
// carries `RuntimeValue::Function`, so the differential gate covers the
// materialization, aliasing, reassignment, and list-element paths.

const FUNCTION_VALUE_CORPUS: &str = r#"
def add(a: Int, b: Int) -> Int:
    return a + b

def mul(a: Int, b: Int) -> Int:
    return a * b

def sub(a: Int, b: Int) -> Int:
    return a - b

def value_alias_check() -> Int:
    f := add
    g := mul
    a := f(1, 2)
    b := g(3, 4)
    h := g
    c := h(5, 6)
    return a + b + c

def value_swap_check() -> Int:
    f := add
    g := mul
    f := g
    g := add
    a := f(2, 3)
    b := g(9, 1)
    return a + b

def value_reassign_check() -> Int:
    f := add
    a := f(1, 1)
    f := sub
    b := f(10, 4)
    return a + b

def value_in_list_check() -> Int:
    ops := [add, mul]
    op0 := list_get(ops, 0)
    op1 := list_get(ops, 1)
    a := op0(7, 1)
    b := op1(2, 5)
    return a + b

def value_call_twice_check() -> Int:
    f := mul
    a := f(2, 3)
    b := f(4, 5)
    return a + b

def main() -> Int:
    if value_alias_check() == 45:
        print_str("ok_value_alias")
    else:
        print_str("fail_value_alias")
    if value_swap_check() == 16:
        print_str("ok_value_swap")
    else:
        print_str("fail_value_swap")
    if value_reassign_check() == 8:
        print_str("ok_value_reassign")
    else:
        print_str("fail_value_reassign")
    if value_in_list_check() == 18:
        print_str("ok_value_list")
    else:
        print_str("fail_value_list")
    if value_call_twice_check() == 26:
        print_str("ok_value_twice")
    else:
        print_str("fail_value_twice")
    print_str("all_ok")
    return 0
"#;

#[test]
fn function_value_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(FUNCTION_VALUE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "function_value");
    assert_eq!(
        status, 0,
        "function value corpus object exited {status}:\n{stdout}"
    );

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "function value corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "function_value");
}

// ── 5D2b gate 1: differential closure corpus ───────────────────────────────

#[test]
fn closure_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(CLOSURE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "closure");
    assert_eq!(status, 0, "closure corpus object exited {status}:\n{stdout}");

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "closure corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "closure");
}

const TASK_CORPUS: &str = r#"
struct Point:
    x: Int
    y: Int

async def first() -> Str:
    return "ready"

async def second() -> Str:
    value := first().await
    return value

def counter_check() -> Int:
    mut count := 0
    counter := fn() -> Int:
        count = count + 1
        return count
    first_call := counter()
    second_call := counter()
    return count + first_call + second_call

def list_closure_check() -> Int:
    add_one := fn(value: Int) -> Int: value + 1
    callbacks := [add_one]
    callback := list_get(callbacks, 0)
    return callback(41)

def nested_capture_check() -> Int:
    base := 10
    add := fn(x: Int) -> Int:
        return base + x
    first_call := add(5)
    return first_call + add(10)

def managed_capture_check() -> Int:
    mut p := Point(3, 7)
    bump := fn() -> Int:
        p = Point(p.x + 1, p.y)
        return p.y
    a := bump()
    b := bump()
    return a + b + p.x

def sync_value_check() -> Int:
    double := fn(x: Int) -> Int: x * 2
    f := double
    return f(21)

def async_value_check() -> Int:
    av := first
    t := av()
    return str_len(t.await)

def cycle_check() -> Int:
    mut total := 0
    xs := list_new()
    f := fn(seed: Int, extra: Int) -> Int:
        total = total + list_len(xs)
        return total + seed + extra
    g := f
    list_push(xs, f)
    return g(100, 7)

def task_chain_check() -> Int:
    t := first()
    a := t.await
    b := t.await
    return str_len(a) + str_len(b)

def spawn_check() -> Int:
    xs := list_new()
    bump := fn():
        list_push(xs, 1)
    spawn bump
    return list_len(xs)

def managed_read_only() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x + p.y
    return read() + p.x

def minimal_capture() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x
    return read()

def capture_call_twice() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x
    a := read()
    b := read()
    return a + b

def no_closure() -> Int:
    p := Point(3, 7)
    return p.x

async def main():
    if counter_check() == 5:
        print_str("ok_counter")
    else:
        print_str("fail_counter")
    if list_closure_check() == 42:
        print_str("ok_list_closure")
    else:
        print_str("fail_list_closure")
    if nested_capture_check() == 35:
        print_str("ok_nested")
    else:
        print_str("fail_nested")
    if managed_capture_check() == 19:
        print_str("ok_managed_capture")
    else:
        print_str("fail_managed_capture")
    if sync_value_check() == 42:
        print_str("ok_sync_value")
    else:
        print_str("fail_sync_value")
    if async_value_check() == 5:
        print_str("ok_async_value")
    else:
        print_str("fail_async_value")
    if cycle_check() == 108:
        print_str("ok_cycle")
    else:
        print_str("fail_cycle")
    if task_chain_check() == 10:
        print_str("ok_task_chain")
    else:
        print_str("fail_task_chain")
    if spawn_check() == 1:
        print_str("ok_spawn")
    else:
        print_str("fail_spawn")
    if managed_read_only() == 13:
        print_str("ok_managed_read_only")
    else:
        print_str("fail_managed_read_only")
    if minimal_capture() == 3:
        print_str("ok_minimal")
    else:
        print_str("fail_minimal")
    if capture_call_twice() == 6:
        print_str("ok_call_twice")
    else:
        print_str("fail_call_twice")
    if no_closure() == 3:
        print_str("ok_no_closure")
    else:
        print_str("fail_no_closure")
    chain := second().await
    if str_len(chain) == 5:
        print_str("ok_async_chain")
    else:
        print_str("fail_async_chain")
    print_str("all_ok")
"#;
// ── 5D2b slice 3 gate: differential closure + task corpus ──────────────────

#[test]
fn task_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(TASK_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    std::fs::write("/tmp/corpus.wasm", &module.object).unwrap();
    let (stdout, status) = run_wasm(&module.object, "task");
    assert_eq!(status, 0, "task corpus object exited {status}:\n{stdout}");

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "task corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "task");
}

// ── 5D2b slice 4 gate: differential builtin corpus (batch 1) ──────────────

const BUILTIN_CORPUS: &str = r#"
def bits_check() -> Int:
    if bswap64(258) != 144396663052566528:
        return 101
    if bswap32(65794) != 33620224:
        return 102
    if bswap16(65794) != 513:
        return 103
    if bswap64(65794) != 144397762564194304:
        return 104
    if clz64(1) != 63:
        return 105
    if clz64(4611686018427387904) != 1:
        return 106
    if ctz64(8) != 3:
        return 107
    if ctz64(1) != 0:
        return 108
    if popcount64(255) != 8:
        return 109
    if popcount64(4294967295) != 32:
        return 110
    return 0

def rot_check() -> Int:
    if rotl64(1, 60) != 1152921504606846976:
        return 111
    if rotr64(1152921504606846976, 60) != 1:
        return 112
    if rotl64(5, 64) != 5:
        return 113
    if rotl64(5, 65) != 10:
        return 114
    if rotl64(-1, 32) != -1:
        return 115
    if rotl32(1, 30) != 1073741824:
        return 116
    if rotr32(1073741824, 30) != 1:
        return 117
    if rotl32(2147483648, 1) != 1:
        return 118
    if rotr32(-1, 1) != 4294967295:
        return 119
    return 0

def shift_check() -> Int:
    if shl_u(1, 62) != 4611686018427387904:
        return 121
    if shr_u(4611686018427387904, 62) != 1:
        return 122
    if shl_u(1, 64) != 0:
        return 123
    if shl_u(1, 100) != 0:
        return 124
    if shl_u(1, -1) != 0:
        return 125
    if shr_u(65535, 100) != 0:
        return 126
    if shr_u(-8, 1) != 9223372036854775804:
        return 127
    return 0

def div_check() -> Int:
    if div_u(10, 3) != 3:
        return 131
    if rem_u(10, 3) != 1:
        return 132
    if div_u(7, 2) != 3:
        return 133
    if rem_u(7, 2) != 1:
        return 134
    if div_u(-10, 3) != 6148914691236517202:
        return 135
    if rem_u(-10, 3) != 0:
        return 136
    return 0

def order_check() -> Int:
    if min(3, 7) != 3:
        return 141
    if max(3, 7) != 7:
        return 142
    if min(-5, 7) != -5:
        return 143
    if max(-5, 7) != 7:
        return 144
    if min_u(-1, 1) != 1:
        return 145
    if max_u(-1, 1) != -1:
        return 146
    if lt_u(-1, 1):
        return 147
    if not le_u(1, 1):
        return 148
    if gt_u(0, -1):
        return 149
    if not ge_u(-1, -1):
        return 150
    return 0

def trunc_check() -> Int:
    if trunc_u8(300) != 44:
        return 151
    if trunc_u16(100000) != 34464:
        return 152
    if trunc_u32(5000000000) != 705032704:
        return 153
    if trunc_i8(200) != -56:
        return 154
    if trunc_i16(49152) != -16384:
        return 155
    if trunc_i32(5000000000) != 705032704:
        return 156
    return 0

def wrap_check() -> Int:
    if add_wrap(9223372036854775807, 1) != (-9223372036854775807 - 1):
        return 161
    if sub_wrap(-9223372036854775807 - 1, 1) != 9223372036854775807:
        return 162
    if mul_wrap(4611686018427387904, 2) != (-9223372036854775807 - 1):
        return 163
    if add_checked(1, 2) != 3:
        return 164
    if sub_checked(1, 2) != -1:
        return 165
    if mul_checked(1000000, 1000000) != 1000000000000:
        return 166
    if add_checked(-9223372036854775807 - 1, 0) != (-9223372036854775807 - 1):
        return 167
    if abs(-5) != 5:
        return 168
    if abs(5) != 5:
        return 169
    return 0

def float_check() -> Int:
    print_float(floor(-2.5))
    print_float(ceil(-2.5))
    print_float(sqrt(144.0))
    print_float(fmod(7.5, 2.0))
    print_float(fmod(-7.5, 2.0))
    return 0

async def main():
    if bits_check() == 0:
        print_str("ok_bits")
    else:
        print_str("fail_bits")
    if rot_check() == 0:
        print_str("ok_rot")
    else:
        print_str("fail_rot")
    if shift_check() == 0:
        print_str("ok_shift")
    else:
        print_str("fail_shift")
    if div_check() == 0:
        print_str("ok_div")
    else:
        print_str("fail_div")
    if order_check() == 0:
        print_str("ok_order")
    else:
        print_str("fail_order")
    if trunc_check() == 0:
        print_str("ok_trunc")
    else:
        print_str("fail_trunc")
    if wrap_check() == 0:
        print_str("ok_wrap")
    else:
        print_str("fail_wrap")
    if float_check() == 0:
        print_str("ok_float")
    else:
        print_str("fail_float")
    print_str("all_ok")
"#;

#[test]
fn builtin_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(BUILTIN_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "builtin");
    assert_eq!(
        status, 0,
        "builtin corpus object exited {status}:\n{stdout}"
    );

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "builtin corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "builtin");
}

// The 5D2b slice 4, batch 3 corpus: every string builtin, asserted
// against the ARC oracle (stdout diff, like the other corpora).
const STR_BUILTIN_CORPUS: &str = r#"
def str1_check() -> Int:
    if str_concat("foo", "bar") != "foobar":
        return 201
    if str_concat("", "x") != "x":
        return 202
    if str_concat("x", "") != "x":
        return 203
    if not str_contains("hello world", "world"):
        return 204
    if str_contains("hello world", "mars"):
        return 205
    if not str_contains("abc", ""):
        return 206
    if not str_starts_with("foobar", "foo"):
        return 207
    if str_starts_with("foobar", "bar"):
        return 208
    if not str_ends_with("foobar", "bar"):
        return 209
    if str_ends_with("foobar", "foo"):
        return 210
    if str_find("hello world", "world") != 6:
        return 211
    if str_find("hello", "x") != -1:
        return 212
    if str_find("abc", "") != 0:
        return 213
    if str_find("aaaa", "aa") != 0:
        return 214
    if str_replace("a-b-c", "-", "+") != "a+b+c":
        return 215
    if str_replace("aaa", "a", "bb") != "bbbbbb":
        return 216
    if str_replace("abc", "x", "y") != "abc":
        return 217
    if str_replace("abc", "", "z") != "abc":
        return 218
    if str_replace("aaa", "aa", "b") != "ba":
        return 219
    if str_trim("  hi  \n\t") != "hi":
        return 220
    if str_trim("   ") != "":
        return 221
    if str_to_lower("AbC") != "abc":
        return 222
    if str_to_upper("AbC") != "ABC":
        return 223
    if str_to_lower("a1b!C") != "a1b!c":
        return 224
    if str_len("hello") != 5:
        return 225
    return 0

def str2_check() -> Int:
    if int_to_str(123) != "123":
        return 231
    if int_to_str(-456) != "-456":
        return 232
    if int_to_str(0) != "0":
        return 233
    if int_to_str(-9223372036854775807 - 1) != "-9223372036854775808":
        return 234
    if int_to_str(9223372036854775807) != "9223372036854775807":
        return 235
    if str_to_int("42") != 42:
        return 241
    if str_to_int("-42") != -42:
        return 242
    if str_to_int("  -42  ") != -42:
        return 243
    if str_to_int("+42") != 42:
        return 244
    if str_to_int("42xyz") != 42:
        return 245
    if str_to_int("") != 0:
        return 246
    if str_to_int("  ") != 0:
        return 247
    if str_to_int("99999999999999999999") != 9223372036854775807:
        return 248
    if str_to_int("-99999999999999999999") != (-9223372036854775807 - 1):
        return 249
    if str_to_int("12 34") != 12:
        return 250
    return 0

def str3_check() -> Int:
    if u64_to_str(42) != "42":
        return 261
    if u64_to_str(-1) != "18446744073709551615":
        return 262
    if u64_to_str(0) != "0":
        return 263
    if u64_to_hex(255) != "ff":
        return 264
    if u64_to_hex(4096) != "1000":
        return 265
    if u64_to_hex(0) != "0":
        return 266
    if u64_to_hex(-1) != "ffffffffffffffff":
        return 267
    if u64_to_hex(3735928559) != "deadbeef":
        return 268
    if str_to_u64("255") != 255:
        return 271
    if str_to_u64("0x10") != 16:
        return 272
    if str_to_u64("0XDEAD") != 57005:
        return 273
    if str_to_u64("  0xff") != 255:
        return 274
    if str_to_u64("abc") != 0:
        return 275
    if str_to_u64("12abc") != 12:
        return 276
    if str_to_u64("0x") != 0:
        return 277
    if str_to_u64("-5") != 0:
        return 278
    if bool_to_str(true) != "true":
        return 281
    if bool_to_str(false) != "false":
        return 282
    return 0

def str4_check() -> Int:
    if float_to_str(1.5) != "1.5":
        return 291
    if float_to_str(-2.25) != "-2.25":
        return 292
    if float_to_str(0.0) != "0":
        return 293
    if float_to_str(123.0) != "123":
        return 294
    if float_to_str(123456.0) != "123456":
        return 295
    if float_to_str(1234567.0) != "1.23457e+06":
        return 296
    if float_to_str(0.0001) != "0.0001":
        return 297
    if float_to_str(0.00001) != "1e-05":
        return 298
    if float_to_str(100000.0) != "100000":
        return 299
    if float_to_str(1000000.0) != "1e+06":
        return 300
    if float_to_str(0.30000000000000004) != "0.3":
        return 301
    if float_to_str(123.456) != "123.456":
        return 302
    if float_to_str(-0.00005) != "-5e-05":
        return 303
    if float_to_str(99999.9) != "99999.9":
        return 304
    return 0

def main() -> Int:
    if str1_check() == 0:
        print_str("ok_str1")
    else:
        print_str("fail_str1")
    if str2_check() == 0:
        print_str("ok_str2")
    else:
        print_str("fail_str2")
    if str3_check() == 0:
        print_str("ok_str3")
    else:
        print_str("fail_str3")
    if str4_check() == 0:
        print_str("ok_str4")
    else:
        print_str("fail_str4")
    print_str("all_ok")
    return 0
"#;


#[test]
fn string_builtin_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(STR_BUILTIN_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "string-builtin");
    assert_eq!(
        status, 0,
        "string builtin corpus object exited {status}:\n{stdout}"
    );

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "string builtin corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "string-builtin");
}
