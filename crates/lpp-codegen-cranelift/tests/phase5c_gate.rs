//! Phase 5C gate — the Cranelift aggregate data surface is correct,
//! ARC-balanced, and deterministic.
//!
//! 1. **Differential execution (aggregate corpus).** Structs
//!    (construction, field read/write, nesting, reassignment,
//!    mixed-width layout, struct-in-struct, managed fields), enums
//!    (dense match dispatch, payload binding, explicit arms, match on
//!    parameter and local), lists (literals of every element class,
//!    element read/write, `ListLen` via `for`-in, reassignment,
//!    managed-element lists, nested lists, list-in-struct), and
//!    cross-function passing of all three: object stdout equals the
//!    Phase 4 oracle (`execute_mir_with_stats`), no `fail_*` marker,
//!    `all_ok` present.
//! 2. **Differential execution (ARC stress corpus).** Aliasing,
//!    self-assignment, field/element swaps, move-then-reassign,
//!    cross-container stores, deep nesting: object stdout equals the
//!    ARC-mode oracle
//!    (`execute_mir_arc` with the 4D `pinned_types()`), whose
//!    end-of-run balance proof validates the interpreter's own ARC
//!    model for the same programs the object executes.
//! 3. **Determinism.** Two compiles of each corpus produce
//!    byte-identical objects and identical symbol censuses.
//! 4. **Symbol census.** Real host-format object, entry `main`; exports
//!    exactly the user functions plus `lpp_main`/`main` and one generated
//!    destructor per nominal (structs `lpp_drop_s{n}`, enums
//!    `lpp_drop_e{n}`, in `MirAggregateId` order); imports exactly
//!    the runtime symbols the corpus uses.
//! 5. **Compile-fail.** Closure and async programs fail with the
//!    exact `E5001` at the exact function; `print_int` fails with the
//!    exact `E5003`. The 5B compile-fails stay green in the 5B gate.

use std::collections::BTreeMap;
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
    ExecutionOutcome, InterpreterLimits, MirFunctionId, MirProgram, build_mir, execute_mir_arc,
    execute_mir_with_stats,
};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

// ── scaffolding (mirrors the 5B gate) ──────────────────────────────────────

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

/// The name-resolution boundary: the HIR name index stays with the
/// caller; the backend asks for names through `NameResolver`.
struct Names<'a>(&'a StringInterner);

impl NameResolver for Names<'_> {
    fn resolve(&self, symbol_raw: u32) -> Option<&str> {
        self.0.resolve(Symbol::from_raw(symbol_raw))
    }
}

fn pipeline(source: &str) -> (MirProgram, TypeInterner, lpp_hir::HirPackage) {
    let filesystem = MemoryFileSystem::new(source, "/p5c");
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/p5c/main.lpp",
            PackageSpec::new("p5c", "/p5c"),
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
    let options = CodegenOptions::new(Target::X86_64, names);
    backend
        .compile_module(program, types, &options)
        .unwrap_or_else(|e| panic!("compile_module failed: {e}"))
}

fn workdir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp5c_{}_{}", std::process::id(), test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Link the object with the C shim (the 5B/5C ARC + list runtime) and
/// run it; returns (stdout, exit status).
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

// ── 6B.3e: drop-in link against the Rust runtime cdylib ─────────────────────
//
// The capstone of Phase 6B: the SAME generated object that links against the C
// `c_shim.c` also links against the Rust `lpp-runtime` cdylib and produces
// byte-identical stdout. This proves the Rust runtime is a true drop-in
// replacement at the link level — interchangeable in a real compiled program,
// not merely ABI-compatible in isolation. The cdylib is self-contained (it
// embeds the Rust std), so the C object needs nothing but `-llpp_runtime`.

/// Locate (building if necessary) the Rust runtime cdylib. `cargo test` does
/// not build the `cdylib` crate-type unless something depends on it, so build
/// it on demand; cargo has released the package lock by the time test binaries
/// run, which makes this nested build safe.
fn runtime_cdylib() -> std::path::PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let filename = if cfg!(target_os = "windows") {
        "lpp_runtime.dll"
    } else if cfg!(target_os = "macos") {
        "liblpp_runtime.dylib"
    } else {
        "liblpp_runtime.so"
    };
    let library = workspace.join("target/debug").join(filename);
    if !library.exists() {
        let out = Command::new("cargo")
            .args(["build", "-p", "lpp-runtime"])
            .current_dir(workspace)
            .output()
            .unwrap_or_else(|e| panic!("cargo failed to start: {e}"));
        assert!(
            out.status.success() && library.exists(),
            "failed to build {filename}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    library
}

/// Link `object` against the Rust runtime cdylib (instead of `c_shim.c`) and
/// run it; returns (stdout, exit status). Mirrors `link_and_run`.
fn link_and_run_runtime(object: &[u8], test_name: &str) -> (String, i32) {
    let so = runtime_cdylib();
    let libdir = so.parent().expect("cdylib directory").to_path_buf();
    let dir = workdir(test_name);
    let module = dir.join("module.o");
    let bin = dir.join("program_runtime");
    std::fs::write(&module, object).unwrap();

    let link = Command::new("cc")
        .arg(&module)
        .arg("-o")
        .arg(&bin)
        .arg("-L")
        .arg(&libdir)
        .arg("-llpp_runtime")
        .arg("-Wl,-rpath")
        .arg(&libdir)
        .arg("-lm")
        .output()
        .unwrap_or_else(|e| panic!("cc failed to start: {e}"));
    if !link.status.success() {
        panic!(
            "drop-in link against liblpp_runtime.so failed:\n{}",
            String::from_utf8_lossy(&link.stderr)
        );
    }

    let run = Command::new(&bin)
        .output()
        .unwrap_or_else(|e| panic!("run failed to start: {e}"));
    (
        String::from_utf8_lossy(&run.stdout).into_owned(),
        run.status.code().unwrap_or(-1),
    )
}

/// Compile `source`, link the one object BOTH ways (C shim and Rust cdylib),
/// and require identical stdout plus a clean exit from each.
fn assert_drop_in_equivalent(source: &str, label: &str) {
    let (program, types, package) = pipeline(source);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    let (shim_out, shim_status) = link_and_run(&module.object, &format!("{label}_shim"));
    let (rt_out, rt_status) = link_and_run_runtime(&module.object, &format!("{label}_runtime"));

    assert_eq!(
        shim_status, 0,
        "{label}: C-shim link exited {shim_status}:\n{shim_out}"
    );
    assert_eq!(
        rt_status, 0,
        "{label}: Rust-runtime link exited {rt_status}:\n{rt_out}"
    );
    assert_eq!(
        shim_out, rt_out,
        "{label}: the Rust runtime cdylib must produce byte-identical stdout to c_shim.c"
    );
    expect_success_markers(&rt_out, label);
}

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "MSVC runtime setup is covered by the Windows driver smoke gate"
)]
fn rust_runtime_is_a_drop_in_for_the_c_shim() {
    // Two proven corpora (aggregate structs/ARC/arithmetic/print, and the ARC
    // stress corpus) each link against the C shim and the Rust cdylib and must
    // run byte-identically. This is the end-to-end Phase 6B proof.
    assert_drop_in_equivalent(AGGREGATE_CORPUS, "dropin_aggregate");
    assert_drop_in_equivalent(ARC_STRESS_CORPUS, "dropin_arc_stress");
}

// ── corpus 1: aggregate surface ────────────────────────────────────────────
//
// Expected marker values (hand-computed):
//   shapes 64, structs 50, managed_struct 44, deep 48, lists 159,
//   list_float 8.25, list_char 2, list_bool 3, nested 10,
//   list_struct 10, calls 120, wrap 21.

const AGGREGATE_CORPUS: &str = r#"
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
    print_str("all_ok")
    return 0
"#;

// ── corpus 2: ARC stress ───────────────────────────────────────────────────
//
// Expected marker values (hand-computed):
//   aliasing 20, self_assign 5, swap 33, move_reassign 18,
//   xstore 44, deep_nest 146.
// ARC-oracle domain notes: construction fields, list elements, and
// call arguments MOVE their local sources (use-after-move is E4303),
// so no check reads a local after transferring it; and no 5C cycle
// type is 4D-pinnable (no struct-field edges, no closures yet), so no
// self-cyclic aggregate appears in the corpus.

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

// ── gate 1: differential aggregate corpus ─────────────────────────────────

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "MSVC runtime setup is covered by the Windows driver smoke gate"
)]
fn aggregate_corpus_matches_the_phase4_oracle() {
    let (program, types, package) = pipeline(AGGREGATE_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "aggregate");
    assert_eq!(status, 0, "aggregate object exited {status}:\n{stdout}");

    let entry = main_function(&program, &package.names.symbols);
    let oracle = run_oracle(&program, &types, entry);
    let expected: String = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "aggregate corpus: object stdout diverges from the oracle"
    );
    expect_success_markers(&stdout, "aggregate");
}

// ── gate 2: differential ARC stress corpus ────────────────────────────────

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "MSVC runtime setup is covered by the Windows driver smoke gate"
)]
fn arc_stress_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(ARC_STRESS_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "arc_stress");
    assert_eq!(status, 0, "arc stress object exited {status}:\n{stdout}");

    let entry = main_function(&program, &package.names.symbols);
    let oracle = run_arc_oracle(&program, &types, entry);
    let expected: String = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "arc stress corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "arc stress");
}

// ── gate 3: determinism ───────────────────────────────────────────────────

#[test]
fn two_compiles_produce_byte_identical_objects() {
    for (label, corpus) in [
        ("aggregate", AGGREGATE_CORPUS),
        ("arc_stress", ARC_STRESS_CORPUS),
    ] {
        let (program, types, package) = pipeline(corpus);
        let names = Names(&package.names.symbols);
        let first = compile(&program, &types, &names);
        let second = compile(&program, &types, &names);
        assert_eq!(
            first.object, second.object,
            "{label}: objects differ byte-wise"
        );
        assert_eq!(
            first.exported_symbols, second.exported_symbols,
            "{label}: export censuses differ"
        );
        assert_eq!(
            first.imported_symbols, second.imported_symbols,
            "{label}: import censuses differ"
        );
    }
}

// ── gate 4: symbol census ─────────────────────────────────────────────────

#[test]
fn object_census_exports_and_imports_match_the_contract() {
    let (program, types, package) = pipeline(AGGREGATE_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    // Real host-format relocatable with the generated C-ABI entry.
    #[cfg(target_os = "windows")]
    assert_eq!(
        &module.object[..2],
        b"\x64\x86",
        "object is not x86-64 COFF"
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        &module.object[..4],
        b"\xcf\xfa\xed\xfe",
        "object is not 64-bit Mach-O"
    );
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    assert_eq!(&module.object[..4], b"\x7fELF", "object is not ELF");
    assert_eq!(module.entry.as_deref(), Some("main"));

    // Exports: `lpp_main` + the generated `main` wrapper, every user
    // function by source name, and exactly one generated destructor
    // per nominal aggregate (structs `lpp_drop_s{n}`, enums
    // `lpp_drop_e{n}`) — nothing else.
    let mut expected_exports =
        std::collections::BTreeSet::from(["main".to_owned(), "lpp_main".to_owned()]);
    for (_, function) in program.functions() {
        if let Some(symbol) = function.name.as_ref() {
            if let Some(name) = package.names.symbols.resolve(*symbol) {
                if name != "main" {
                    expected_exports.insert(name.to_owned());
                }
            }
        }
    }
    for (id, aggregate) in program.aggregates() {
        let tag = match aggregate.kind {
            lpp_mir::MirAggregateKind::Struct => 's',
            lpp_mir::MirAggregateKind::Enum => 'e',
        };
        expected_exports.insert(format!("lpp_drop_{tag}{}", id.raw()));
    }
    assert_eq!(
        module.exported_symbols, expected_exports,
        "export census diverges from the contract"
    );

    // Imports: exactly the runtime symbols this corpus uses — the ARC
    // allocator/retainer/releaser, the list family across every
    // element class the corpus touches, and print_str. No fmod (the
    // corpus never divides floats by a non-constant), no allocator
    // for destructors that manage nothing.
    assert_eq!(
        module.imported_symbols,
        std::collections::BTreeSet::from([
            "lpp_arc_alloc_with_destructor".to_owned(),
            "lpp_arc_release".to_owned(),
            "lpp_arc_retain".to_owned(),
            "lpp_list_get".to_owned(),
            "lpp_list_get_arc".to_owned(),
            "lpp_list_get_bool".to_owned(),
            "lpp_list_get_float".to_owned(),
            "lpp_list_len".to_owned(),
            "lpp_list_new".to_owned(),
            "lpp_list_new_arc".to_owned(),
            "lpp_list_push".to_owned(),
            "lpp_list_push_arc".to_owned(),
            "lpp_list_push_bool".to_owned(),
            "lpp_list_push_float".to_owned(),
            "lpp_list_set".to_owned(),
            "lpp_list_set_arc".to_owned(),
            "lpp_list_set_bool".to_owned(),
            "lpp_list_set_float".to_owned(),
            "lpp_print_str".to_owned(),
        ])
    );
}

// ── gate 5: compile-fail (superseded by the 5C2 contract) ────────────────
//
// 5C2 lifts the closure and async rejections: those programs now
// compile. The `E5001` set shrinks to tuples, and the builtin
// rejection is the Family D surface (the 5C `print_int` example is
// Family A in 5C2 and lowers normally).

fn expect_code(error: &CodegenError, code: &str, function: MirFunctionId) {
    assert_eq!(error.code(), code);
    assert_eq!(
        error.function,
        Some(function),
        "typed rejection must name the exact function"
    );
}

#[test]
fn tuple_rejection_and_family_d_builtins_are_exact() {
    // Closure and async programs compile in 5C2 (the 5C rejections are
    // lifted by the function-value surface).
    let (program, types, package) = pipeline(
        "def main() -> Int:\n    base := 5\n    cb := fn(x):\n        return x + base\n    return cb(1)\n",
    );
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    CraneliftBackend
        .compile_module(&program, &types, &options)
        .unwrap_or_else(|e| panic!("closure: expected Ok in 5C2, got {e}"));

    let (program, types, package) = pipeline(
        "async def value() -> Int:\n    return 1\n\ndef main():\n    t := value()\n    print_int(t.await)\n",
    );
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    CraneliftBackend
        .compile_module(&program, &types, &options)
        .unwrap_or_else(|e| panic!("async: expected Ok in 5C2, got {e}"));

    // Tuples are now lowered as flat heap records (construction +
    // tuple-field projection), so a tuple program compiles.
    let (program, types, package) =
        pipeline("def main() -> Int:\n    t := (1, 2)\n    return t.0\n");
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    CraneliftBackend
        .compile_module(&program, &types, &options)
        .unwrap_or_else(|e| panic!("tuple: expected Ok, got {e}"));

    // A Family D builtin: E5003 at the exact function.
    let (program, types, package) = pipeline(
        "def main() -> Int:\n    w := webview_window_create(\"t\", 0, 0, 0)\n    return w\n",
    );
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    let error = CraneliftBackend
        .compile_module(&program, &types, &options)
        .err()
        .unwrap_or_else(|| panic!("webview: expected E5003, got Ok"));
    expect_code(
        &error,
        "E5003",
        main_function(&program, &package.names.symbols),
    );
    match &error.kind {
        CodegenErrorKind::UnrepresentableBuiltin { builtin, .. } => {
            assert_eq!(
                builtin.descriptor().name,
                "webview_window_create",
                "the rejected builtin is webview_window_create"
            );
        }
        kind => panic!("webview: wrong kind {kind:?}"),
    }
}
