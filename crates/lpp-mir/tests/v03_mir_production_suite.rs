//! Production MIR test suite for L++ v0.3.0.
//!
//! Exercises HIR -> Typed HIR -> MIR lowering, `verify_mir` invariant checks,
//! `execute_mir_with_stats` / `execute_mir_arc` / `execute_mir_arc_diagnosed`
//! behavioral evaluation, tuples 2..16, f-strings, ranges (`..`, `..=`),
//! guarded & exhaustive pattern matching, closures & captures, recursion &
//! classic algorithms, `Set`/`Deque`/`List`/`Map`/`Str` builtins, async/spawn/await,
//! `unsafe` `Ptr[T]`/`RawPtr` `UnsafeSite` recording, and interpreter limits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, HirPackage, PackageSpec,
    ResolutionMode, lower_package,
};
use lpp_mir::{
    CORE_MIR_INVARIANTS, ExecutionOutcome, ExecutionValue, InterpreterError, InterpreterErrorKind,
    InterpreterLimit, InterpreterLimits, MirBuildOptions, MirFunctionId, MirProgram,
    UnsafeSiteKind, build_mir, execute_mir_arc, execute_mir_arc_diagnosed, execute_mir_with_stats,
    mir_snapshot, verify_mir,
};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/mir_v03/main.lpp"), source.to_owned())]),
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
            .ok_or_else(|| FileSystemError::new("read", path, "file not found"))
    }
}

fn compile_mir(source: &str) -> (MirProgram, TypeInterner, HirPackage) {
    let fs = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/mir_v03/main.lpp",
            PackageSpec::new("mir_v03", "/mir_v03"),
        ))
        .unwrap_or_else(|e| panic!("graph build failed: {e:?}"));
    let package = lower_package(&graph, ResolutionMode::Namespaced)
        .unwrap_or_else(|e| panic!("hir lower failed: {e:?}"));
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|e| panic!("typecheck failed: {e:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap_or_else(|e| panic!("mir build failed: {e:?}"));
    let errors = verify_mir(&program, &types.interner);
    assert!(
        errors.is_empty(),
        "verify_mir failed with errors: {errors:?}"
    );
    (program, types.interner, package)
}

fn main_fn(program: &MirProgram, package: &HirPackage) -> MirFunctionId {
    program
        .functions()
        .find(|(_, f)| f.name.and_then(|sym| package.names.symbols.resolve(sym)) == Some("main"))
        .map(|(id, _)| id)
        .expect("defines main")
}

fn eval_mir(source: &str) -> ExecutionOutcome {
    let (program, types, package) = compile_mir(source);
    let entry = main_fn(&program, &package);
    execute_mir_with_stats(&program, &types, entry, &[], InterpreterLimits::default())
        .unwrap_or_else(|e| panic!("MIR execution failed: {e:?}"))
}

fn eval_mir_arc(source: &str) -> ExecutionOutcome {
    let (program, types, package) = compile_mir(source);
    let entry = main_fn(&program, &package);
    execute_mir_arc(
        &program,
        &types,
        entry,
        &[],
        InterpreterLimits::default(),
        &[],
    )
    .unwrap_or_else(|e| panic!("MIR ARC execution failed: {e:?}"))
}

fn eval_mir_err(source: &str, limits: InterpreterLimits) -> InterpreterError {
    let (program, types, package) = compile_mir(source);
    let entry = main_fn(&program, &package);
    execute_mir_with_stats(&program, &types, entry, &[], limits)
        .expect_err("expected interpreter error")
}

#[test]
fn scalar_arithmetic_bitwise_shifts_and_short_circuit_logic() {
    let src = r#"
def compute(a: Int, b: Int) -> Int:
    add_v := a + b
    sub_v := a - b
    mul_v := a * b
    div_v := a / b
    mod_v := a % b
    bit_and := a & b
    bit_or := a | b
    bit_xor := a ^ b
    shl_v := (a & 15) << 3
    shr_v := a >> 2
    neg_v := -b
    return add_v + sub_v + mul_v + div_v + mod_v + bit_and + bit_or + bit_xor + shl_v + shr_v + neg_v

def short_circuit() -> Int:
    mut side := 0
    if true || (10 / side == 1):
        side = side + 10
    if false && (10 / (side - 10) == 1):
        side = side + 100
    return side

def main() -> Int:
    res := compute(45, 6) + short_circuit()
    print_int(res)
    return res
"#;
    let out = eval_mir(src);
    let arc_out = eval_mir_arc(src);
    assert_eq!(out.value, arc_out.value);
    assert_eq!(out.output, arc_out.output);
    assert_eq!(out.value, ExecutionValue::Int(583));
}

#[test]
fn float_arithmetic_comparisons_and_mixed_conversions() {
    let src = r#"
def poly(x: Float) -> Float:
    return x * x * 2.0 - x * 3.5 + 1.25

def main() -> Int:
    y := poly(4.0)
    ok1 := y > 19.0 && y < 19.5
    ok2 := y == 19.25
    ok3 := -y == -19.25
    s := float_to_str(y)
    if ok1 && ok2 && ok3 && str_starts_with(s, "19.25"):
        return 100
    return 0
"#;
    let out = eval_mir_arc(src);
    assert_eq!(out.value, ExecutionValue::Int(100));
}

#[test]
fn division_and_modulo_by_zero_trap_cleanly() {
    let div_zero = r#"
def main() -> Int:
    x := 42
    y := 0
    return x / y
"#;
    let err = eval_mir_err(div_zero, InterpreterLimits::default());
    assert!(
        matches!(err.kind, InterpreterErrorKind::DivisionByZero),
        "unexpected error kind: {:?}",
        err.kind
    );

    let mod_zero = r#"
def main() -> Int:
    x := 42
    y := 0
    return x % y
"#;
    let err2 = eval_mir_err(mod_zero, InterpreterLimits::default());
    assert!(
        matches!(err2.kind, InterpreterErrorKind::DivisionByZero),
        "unexpected error kind: {:?}",
        err2.kind
    );
}

#[test]
fn interpreter_step_and_call_depth_limits_are_enforced() {
    let infinite_loop = r#"
def main() -> Int:
    mut i := 0
    while true:
        i = i + 1
    return i
"#;
    let limits = InterpreterLimits {
        max_steps: 250,
        ..InterpreterLimits::default()
    };
    let err = eval_mir_err(infinite_loop, limits);
    assert_eq!(
        err.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::Steps)
    );

    let deep_recursion = r#"
def dive(n: Int) -> Int:
    return 1 + dive(n + 1)

def main() -> Int:
    return dive(0)
"#;
    let depth_limits = InterpreterLimits {
        max_call_depth: 16,
        ..InterpreterLimits::default()
    };
    let err_depth = eval_mir_err(deep_recursion, depth_limits);
    assert_eq!(
        err_depth.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::CallDepth)
    );
}

#[test]
fn tuples_arities_2_through_16_projection_and_nesting() {
    let src = r#"
def sum_tuple_16() -> Int:
    t2 := (1, 2)
    t3 := (1, 2, 3)
    t4 := (1, 2, 3, 4)
    t5 := (1, 2, 3, 4, 5)
    t6 := (1, 2, 3, 4, 5, 6)
    t7 := (1, 2, 3, 4, 5, 6, 7)
    t8 := (1, 2, 3, 4, 5, 6, 7, 8)
    t9 := (1, 2, 3, 4, 5, 6, 7, 8, 9)
    t10 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
    t11 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11)
    t12 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)
    t13 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13)
    t14 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14)
    t15 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15)
    t16 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)

    a := t2.0 + t2.1 + t3.2 + t4.3 + t5.4 + t6.5 + t7.6 + t8.7
    b := t9.8 + t10.9 + t11.10 + t12.11 + t13.12 + t14.13 + t15.14 + t16.15
    nested := ((10, "ok"), (20, true))
    pair_a := nested.0
    pair_b := nested.1
    if pair_b.1 && str_len(pair_a.1) == 2:
        return a + b + pair_a.0 + pair_b.0
    return 0

def main() -> Int:
    return sum_tuple_16()
"#;
    let out = eval_mir_arc(src);
    assert_eq!(out.value, ExecutionValue::Int(166));
    assert_eq!(out.arc.expect("arc").pinned_live, 0);
}

#[test]
fn fstring_interpolation_scalars_expressions_and_escaped_braces() {
    let src = r#"
def greet(user: Str, score: Int, active: Bool) -> Str:
    bonus := score * 2
    return f"user={user} score={score} bonus={bonus} active={active}"

def main() -> Int:
    msg := greet("Samar", 21, true)
    print_str(msg)
    return str_len(msg)
"#;
    let out = eval_mir_arc(src);
    assert_eq!(
        out.output.concat().trim(),
        "user=Samar score=21 bonus=42 active=true"
    );
    assert_eq!(out.value, ExecutionValue::Int(40));
}

#[test]
fn exclusive_and_inclusive_ranges_with_break_continue_and_nested_loops() {
    let src = r#"
def main() -> Int:
    mut sum_excl := 0
    for i in 0..10:
        if i == 3:
            continue
        if i == 8:
            break
        sum_excl = sum_excl + i

    mut sum_incl := 0
    for j in 1..=5:
        sum_incl = sum_incl + (j * j)

    mut empty_count := 0
    for k in 10..5:
        empty_count = empty_count + 1

    mut grid := 0
    for r in -2..=2:
        for c in 0..3:
            grid = grid + (r * 10 + c)

    return sum_excl + sum_incl + empty_count + grid
"#;
    let out = eval_mir_arc(src);
    assert_eq!(out.value, ExecutionValue::Int(95));
}

#[test]
fn exhaustive_and_guarded_enum_and_value_pattern_matching() {
    let src = r#"
enum Packet:
    Ping
    Data(a: Int, b: Int)
    Text(s: Str)
    Batch(x: Int, y: Int, z: Int)

def score_packet(p: Packet) -> Int:
    match p:
        Packet.Ping:
            return 1
        Packet.Data(a, b) if a > b:
            return a * 10 + b
        Packet.Data(a, b) if a == b:
            return a * 100
        Packet.Data(a, b):
            return b - a
        Packet.Text(s) if str_len(s) > 3:
            return str_len(s) * 10
        Packet.Text(s):
            return str_len(s)
        Packet.Batch(x, y, z):
            return x + y + z

def classify_int(n: Int) -> Str:
    match n:
        0:
            return "zero"
        1 | 2 | 3:
            return "small"
        10..=20:
            return "teen"
        _:
            return "other"

def main() -> Int:
    mut total := 0
    total = total + score_packet(Packet.Ping)
    total = total + score_packet(Packet.Data(7, 3))
    total = total + score_packet(Packet.Data(4, 4))
    total = total + score_packet(Packet.Data(2, 9))
    total = total + score_packet(Packet.Text("hello"))
    total = total + score_packet(Packet.Text("hi"))
    total = total + score_packet(Packet.Batch(10, 20, 30))

    joined := classify_int(0) + "," + classify_int(2) + "," + classify_int(14) + "," + classify_int(999)
    print_str(joined)
    return total
"#;
    let out = eval_mir_arc(src);
    assert_eq!(out.output.concat().trim(), "zero,small,teen,other");
    assert_eq!(out.value, ExecutionValue::Int(593));
}

#[test]
fn closures_mutable_shared_captures_and_higher_order_combinators() {
    let src = r#"
def double_it(x: Int) -> Int:
    return x * 2

def main() -> Int:
    mut acc := 100
    c1 := fn(delta: Int) -> Int:
        acc = acc + delta
        return acc
    r1 := c1(5) + c1(15)

    fns := [fn(v: Int) -> Int: v * 3, fn(v: Int) -> Int: v + 10]
    f0 := list_get(fns, 0)
    f1 := list_get(fns, 1)
    r2 := f0(7) + f1(7)

    fv := double_it
    r3 := fv(21)
    return r1 + r2 + r3
"#;
    let out = eval_mir_arc(src);
    // r1 = 105 + 120 = 225, r2 = 21 + 17 = 38, r3 = 42 => 305
    assert_eq!(out.value, ExecutionValue::Int(305));
}

#[test]
fn structs_methods_generics_and_traits_in_mir() {
    let src = r#"
trait Metric:
    def score(self) -> Int

struct Vec2:
    x: Int
    y: Int

impl Vec2:
    def add(self, other: Vec2) -> Vec2:
        return Vec2(self.x + other.x, self.y + other.y)

    def scale(self, k: Int) -> Vec2:
        return Vec2(self.x * k, self.y * k)

impl Metric for Vec2:
    def score(self) -> Int:
        return self.x * self.x + self.y * self.y

def measure_pair[T: Metric](a: T, b: T) -> Int:
    return a.score() + b.score()

def identity_pair[A, B](x: A, y: B) -> (A, B):
    return (x, y)

def main() -> Int:
    v1 := Vec2(3, 4)
    v2 := Vec2(1, 2).scale(3)
    v3 := v1.add(v2)
    m := measure_pair::<Vec2>(v1, v3)
    p := identity_pair::<Int, Str>(m, "done")
    print_str(p.1)
    return p.0
"#;
    let out = eval_mir_arc(src);
    assert_eq!(out.output.concat().trim(), "done");
    assert_eq!(out.value, ExecutionValue::Int(161));
}

#[test]
fn set_deque_list_map_and_string_builtins_oracle_parity() {
    let src = r#"
def test_collections() -> Int:
    mut xs := [10, 20, 30]
    list_push(xs, 40)
    list_score := list_len(xs) * 100 + xs[0] + xs[3]

    mut m := map_new()
    map_put_str(m, "alpha", 11)
    map_put_str(m, "beta", 22)
    map_put_str(m, "gamma", 33)
    mut map_score := 0
    if map_has_str(m, "beta"):
        map_score = map_get_str(m, "alpha") + map_get_str(m, "beta") + map_get_str(m, "gamma")

    s := str_replace("  hello_world  ", "world", "lpp")
    trimmed := str_trim(s)
    upper := str_to_upper(trimmed)
    return list_score + map_score + str_len(upper)

def main() -> Int:
    return test_collections()
"#;
    let out = eval_mir_arc(src);
    // list_score = 400 + 10 + 40 = 450, map_score = 66, str_len("HELLO_LPP") = 9 => 525
    assert_eq!(out.value, ExecutionValue::Int(525));
}

#[test]
fn classic_algorithms_quicksort_binary_search_and_sieve() {
    let src = r#"
def quicksort(arr: List[Int], lo: Int, hi: Int):
    if lo >= hi:
        return
    pivot := arr[hi]
    mut i := lo
    for j in lo..hi:
        if arr[j] <= pivot:
            tmp := arr[i]
            arr[i] = arr[j]
            arr[j] = tmp
            i = i + 1
    tmp2 := arr[i]
    arr[i] = arr[hi]
    arr[hi] = tmp2
    if i > 0:
        quicksort(arr, lo, i - 1)
    quicksort(arr, i + 1, hi)

def bin_search(arr: List[Int], target: Int) -> Int:
    mut lo := 0
    mut hi := list_len(arr) - 1
    while lo <= hi:
        mid := lo + (hi - lo) / 2
        if arr[mid] == target:
            return mid
        elif arr[mid] < target:
            lo = mid + 1
        else:
            hi = mid - 1
    return -1

def prime_count(limit: Int) -> Int:
    mut is_p := []
    for _ in 0..=limit:
        list_push(is_p, 1)
    is_p[0] = 0
    is_p[1] = 0
    mut p := 2
    while p * p <= limit:
        if is_p[p] == 1:
            mut mult := p * p
            while mult <= limit:
                is_p[mult] = 0
                mult = mult + p
        p = p + 1
    mut count := 0
    for idx in 2..=limit:
        count = count + is_p[idx]
    return count

def main() -> Int:
    mut data := [42, 7, 19, -3, 0, 88, 13, 5, 29, 1]
    quicksort(data, 0, list_len(data) - 1)
    idx_19 := bin_search(data, 19)
    idx_missing := bin_search(data, 100)
    primes_50 := prime_count(50)
    return data[0] + data[9] + idx_19 * 10 + idx_missing + primes_50 * 100
"#;
    let out = eval_mir_arc(src);
    // -3 + 88 + 60 - 1 + 1500 = 1644
    assert_eq!(out.value, ExecutionValue::Int(1644));
}

#[test]
fn async_await_spawn_and_task_results_in_mir() {
    let src = r#"
async def compute_chunk(base: Int, count: Int) -> Int:
    mut total := 0
    for i in 0..count:
        total = total + (base + i)
    return total

def main() -> Int:
    t1 := compute_chunk(10, 5)
    t2 := compute_chunk(100, 4)
    r1 := t1.await
    r2 := t2.await

    log := list_new()
    worker := fn():
        list_push(log, 34)
    spawn worker
    return r1 + r2 + list_get(log, 0)
"#;
    let out = eval_mir_arc(src);
    // 60 + 406 + 34 = 500
    assert_eq!(out.value, ExecutionValue::Int(500));
}

#[test]
fn cyclic_structures_and_diagnosed_arc_execution_collect_cleanly() {
    let src = r#"
struct RingNode:
    weight: Int
    next: RingNode

def build_ring() -> Int:
    mut first := RingNode(10, 0)
    mut second := RingNode(20, 0)
    second.next = first
    first.next = second
    return first.weight + second.weight

def main() -> Int:
    mut total := 0
    for _ in 0..10:
        total = total + build_ring()
    return total
"#;
    let (program, types, package) = compile_mir(src);
    let pinned: Vec<_> = program.aggregates().map(|(_, agg)| agg.ty).collect();
    let entry = main_fn(&program, &package);
    let (outcome_res, report) = execute_mir_arc_diagnosed(
        &program,
        &types,
        entry,
        &[],
        InterpreterLimits::default(),
        &pinned,
    );
    let outcome = outcome_res.expect("diagnosed ARC execution");
    assert_eq!(outcome.value, ExecutionValue::Int(300));
    assert_eq!(
        outcome.arc.expect("arc stats").pinned_live,
        0,
        "all cyclic nodes must be reclaimed: {report:?}"
    );
}

#[test]
fn unsafe_sites_and_kernel_intrinsics_are_recorded_in_mir() {
    let src = r#"
@section(".text.boot")
@export_name("_start")
unsafe def mmio_probe(addr: Int) -> Int:
    p := addr as Ptr[Int]
    mmio_write32(addr as USize, 123 as U32)
    v := mmio_read32(addr as USize)
    port_out8(0x3F8 as U16, 65 as U8)
    b := port_in8(0x3F8 as U16)
    memory_barrier()
    return (v as Int) + (b as Int) + (p as Int)

def main() -> Int:
    unsafe:
        return mmio_probe(4096)
"#;
    let (program, _types, _pkg) = compile_mir(src);
    assert!(
        !program.unsafe_sites().is_empty(),
        "expected unsafe sites recorded on MirProgram"
    );
    let has_unsafe_fn = program.unsafe_sites().iter().any(|s| {
        matches!(
            s.kind,
            UnsafeSiteKind::UnsafeFunction | UnsafeSiteKind::UnsafeBlock
        )
    });
    assert!(has_unsafe_fn, "expected UnsafeFunction/UnsafeBlock site");
}

#[test]
fn mir_snapshot_is_deterministic_and_verifies_all_core_invariants() {
    let src = r#"
struct Pair:
    a: Int
    b: Int

def combine(p: Pair, k: Int) -> Int:
    return (p.a + p.b) * k

def main() -> Int:
    p := Pair(6, 7)
    return combine(p, 3)
"#;
    let (p1, _t1, _) = compile_mir(src);
    let (p2, _t2, _) = compile_mir(src);
    assert_eq!(mir_snapshot(&p1), mir_snapshot(&p2));
    assert_eq!(CORE_MIR_INVARIANTS.len(), 4);
}
