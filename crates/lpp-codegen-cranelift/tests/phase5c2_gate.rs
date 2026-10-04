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
//!    destructor, and the used thunks; imports stay inside the closed
//!    runtime-symbol universe.
//! 5. **Table census + rejections.** The registry enumerates to
//!    518 = Family A ∪ B ∪ C ∪ D, disjoint; a Family D builtin fails
//!    with the exact `E5003` at the exact function; tuples stay the
//!    only `E5001` non-builtin rejection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{Backend, CodegenError, CodegenErrorKind, CodegenOptions, NameResolver, Target};
use lpp_codegen_cranelift::CraneliftBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode, Symbol,
    StringInterner, lower_package,
};
use lpp_mir::{
    ExecutionOutcome, InterpreterLimits, MirFunctionId, MirFunctionKind, MirProgram, build_mir,
    execute_mir_arc,
};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

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
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/p5c2/main.lpp",
            PackageSpec::new("p5c2", "/p5c2"),
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

/// The ARC-mode oracle: the 4D plan's pinned set tells the
/// interpreter which types may legitimately survive the run; the
/// end-of-run balance proof validates the interpreter's ARC model.
fn run_arc_oracle(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    let plan = compute_ownership_plan(program, types)
        .unwrap_or_else(|error| panic!("ownership plan failed: {error:?}"));
    let pinned = plan.pinned_types();
    execute_mir_arc(program, types, entry, &[], InterpreterLimits::default(), &pinned)
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
    let dir =
        std::env::temp_dir().join(format!("lpp5c2_{}_{}", std::process::id(), test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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
        .unwrap_or_else(|e| panic!("cc failed to start: {e}"));
    if !link.status.success() {
        panic!(
            "link failed:\n{}",
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

fn expect_success_markers(stdout: &str, test_name: &str) {
    assert_eq!(
        stdout.lines().filter(|line| line.starts_with("fail_")).count(),
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

const CLOSURE_TASK_CORPUS: &str = r#"
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

// ── corpus 2: builtin families (Family A) ─────────────────────────────────

const BUILTIN_CORPUS: &str = r#"
def print_family_check() -> Int:
    print_int(42)
    print_float(8.25)
    print_bool(true)
    print_str("hello_print")
    print(7)
    print(3.5)
    print(false)
    print("direct_print")
    mut s := "consumed"
    write_str(s)
    print_int(1)
    return 0

def string_ops_check() -> Int:
    a := str_concat("hello", " world")
    if str_len(a) != 11:
        print_str("fail_str_len")
    if not str_contains(a, "lo w"):
        print_str("fail_str_contains")
    if not str_starts_with(a, "hello"):
        print_str("fail_str_starts")
    if not str_ends_with(a, "world"):
        print_str("fail_str_ends")
    if str_find(a, "wor") != 6:
        print_str("fail_str_find")
    if str_find(a, "z") != -1:
        print_str("fail_str_find_miss")
    b := str_replace(a, "world", "LPP")
    print_str(b)
    c := str_trim("  pad  ")
    print_str(c)
    print_str(str_lower("MiXeD"))
    print_str(str_upper("mixed"))
    print_str(str_replace("aaa", "a", "bb"))
    print_str(str_replace("abc", "", "x"))
    if str_len(str_trim("\t\n x \r")) != 1:
        print_str("fail_trim_edges")
    return 0

def conversions_check() -> Int:
    if int_to_str(-1234) != "-1234":
        print_str("fail_int_to_str")
    if u64_to_str(-1) != "18446744073709551615":
        print_str("fail_u64_to_str")
    if u64_to_hex(255) != "ff":
        print_str("fail_u64_to_hex")
    if bool_to_str(true) != "true":
        print_str("fail_bool_to_str")
    if float_to_str(0.00001) != "1e-05":
        print_str("fail_float_exp")
    if float_to_str(1234567.0) != "1.23457e+06":
        print_str("fail_float_exp_up")
    if float_to_str(0.0001) != "0.0001":
        print_str("fail_float_decimal")
    if float_to_str(-0.00001) != "-1e-05":
        print_str("fail_float_neg_exp")
    if float_to_str(999999.0) != "999999":
        print_str("fail_float_whole")
    if float_to_str(1.0) != "1":
        print_str("fail_float_one")
    print_str(float_to_str(0.00001))
    print_str(float_to_str(1234567.0))
    if str_to_int("  -42") != -42:
        print_str("fail_str_to_int")
    if str_to_int("99999999999999999999") != 9223372036854775807:
        print_str("fail_str_to_int_saturate")
    if str_to_int("abc") != 0:
        print_str("fail_str_to_int_none")
    if str_to_u64("0x1F") != 31:
        print_str("fail_str_to_u64_hex")
    if str_to_u64("255") != 255:
        print_str("fail_str_to_u64_dec")
    if str_to_u64("ff") != 0:
        print_str("fail_str_to_u64_no_prefix")
    return 0

def integer_helpers_check() -> Int:
    if abs(-5) != 5 or abs(5) != 5:
        print_str("fail_abs")
    if abs(sub_wrap(-9223372036854775807, 1)) != sub_wrap(-9223372036854775807, 1):
        print_str("fail_abs_min")
    if min(-3, 7) != -3 or max(-3, 7) != 7:
        print_str("fail_min_max")
    if min_u(-1, 1) != 1 or max_u(-1, 1) != -1:
        print_str("fail_min_max_u")
    if lt_u(-1, 1) == 1 or ge_u(1, -1) == 1 or gt_u(-1, 1) == 0 or lt_u(1, -1) == 0 or le_u(1, -1) == 0:
        print_str("fail_cmp_u")
    if shr_u(8, 1) != 4 or shl_u(4, 3) != 32:
        print_str("fail_shift")
    if shr_u(1, 64) != 0 or shl_u(1, -1) != 0:
        print_str("fail_shift_bounds")
    if div_u(100, 7) != 14 or rem_u(100, 7) != 2:
        print_str("fail_div_rem")
    if div_u(-7, 2) != 9223372036854775804:
        print_str("fail_div_u_min")
    if popcount64(10) != 2 or clz64(1) != 63 or ctz64(4) != 2:
        print_str("fail_bit_count")
    if clz64(0) != 64 or ctz64(0) != 64:
        print_str("fail_bit_count_zero")
    if bswap16(4660) != 13330:
        print_str("fail_bswap16")
    if bswap32(16909060) != 67305985:
        print_str("fail_bswap32")
    if rotl64(1, 63) != sub_wrap(-9223372036854775807, 1):
        print_str("fail_rotl64")
    if rotl64(3, 1) != 6 or rotr64(6, 1) != 3:
        print_str("fail_rotate64")
    if rotl64(-1, 5) != -1:
        print_str("fail_rotl64_allones")
    if rotl32(1, 31) != 2147483648:
        print_str("fail_rotl32")
    if rotr32(2147483648, 1) != 1073741824:
        print_str("fail_rotr32")
    if trunc_u8(-1) != 255 or trunc_u16(-1) != 65535 or trunc_u32(-1) != 4294967295:
        print_str("fail_trunc_u")
    if trunc_i8(-1) != -1 or trunc_i16(-1) != -1:
        print_str("fail_trunc_i")
    if trunc_i32(2147483648) != -2147483648:
        print_str("fail_trunc_i32")
    if add_checked(10, 20) != 30:
        print_str("fail_add_checked")
    if sub_checked(10, 40) != -30:
        print_str("fail_sub_checked")
    if mul_checked(1000000, 1000000) != 1000000000000:
        print_str("fail_mul_checked")
    if add_wrap(sub_wrap(-9223372036854775807, 1), 1) != -9223372036854775807:
        print_str("fail_add_wrap")
    if sub_wrap(9223372036854775807, 1) != 9223372036854775806:
        print_str("fail_sub_wrap")
    if mul_wrap(3037000500, 3037000500) != -9223372036709301616:
        print_str("fail_mul_wrap")
    return 0

def float_ops_check() -> Int:
    if floor(3.7) != 3.0:
        print_str("fail_floor")
    if ceil(3.1) != 4.0:
        print_str("fail_ceil")
    if pow(2.0, 3.0) != 8.0:
        print_str("fail_pow")
    if sqrt(16.0) != 4.0:
        print_str("fail_sqrt")
    if fmod(7.5, 2.0) != 1.5:
        print_str("fail_fmod")
    if fmod(-7.5, 2.0) != 0.5:
        print_str("fail_fmod_negative")
    print_float(fmod(10.75, 2.0))
    return 0

def list_family_check() -> Int:
    xs := list_new()
    list_push(xs, 10)
    list_push(xs, 20)
    list_set(xs, 0, 15)
    if list_get(xs, 0) != 15 or list_get(xs, 1) != 20 or list_len(xs) != 2:
        print_str("fail_list_int")
    fs := list_new()
    list_push(fs, 1.5)
    list_set(fs, 0, 2.25)
    if list_get(fs, 0) != 2.25:
        print_str("fail_list_float")
    bs := list_new()
    list_push(bs, true)
    list_set(bs, 0, false)
    if list_get(bs, 0):
        print_str("fail_list_bool")
    ms := list_new()
    list_push(ms, "one")
    list_set(ms, 0, "two")
    mut s := "moved_in"
    list_push(ms, s)
    if list_get(ms, 0) != "two" or list_get(ms, 1) != "moved_in" or list_len(ms) != 2:
        print_str("fail_list_str")
    return 0

def main() -> Int:
    print_family_check()
    string_ops_check()
    conversions_check()
    integer_helpers_check()
    float_ops_check()
    list_family_check()
    print_str("all_ok")
    return 0
"#;

// ── corpus 3: slices + SIMD (self-checking) ───────────────────────────────
//
// Hand-computed stdout (the interpreter reports `UnsupportedBuiltin`
// for both families, so this corpus is verified against its exact
// expected output):
//   movemask 516 (byte 2 of lane 0, byte 9 of lane 1 carry 0x6e)
//   add-sum 7237123, extract-1 of sub 28158, mul-sum 14474240,
//   not-sum -7237122, splat-sum 220, shr0 3604480, xor-sum 9,
//   and-sum 8, or-sum 10, shr_var-sum 3618560, checksum(10) 143,
//   slice len 2 / element 30, bool slice true, "world", str-slice len 5.

const SIMD_SLICE_CORPUS: &str = r#"
def simd_self() -> Int:
    v := vec_i64x2(7208960, 28160)
    probe := vec_u8x16_splat(110)
    matches := vec_u8x16_eq(v, probe)
    print_int(vec_u8x16_movemask(matches))
    print_int(vec_i64x2_sum(vec_i64x2_add(v, vec_i64x2(1, 2))))
    print_int(vec_i64x2_extract(vec_i64x2_sub(v, vec_i64x2(1, 2)), 1))
    print_int(vec_i64x2_sum(vec_i64x2_mul(v, vec_i64x2(2, 2))))
    print_int(vec_i64x2_sum(vec_i64x2_not(v)))
    print_int(vec_i64x2_sum(vec_i64x2_splat(110)))
    print_int(vec_i64x2_extract(vec_i64x2_shr(v, 1), 0))
    m := vec_i64x2(3, 5)
    print_int(vec_i64x2_sum(vec_i64x2_xor(m, vec_i64x2(1, 2))))
    print_int(vec_i64x2_sum(vec_i64x2_and(m, vec_i64x2(3, 7))))
    print_int(vec_i64x2_sum(vec_i64x2_or(m, vec_i64x2(1, 2))))
    print_int(vec_i64x2_sum(vec_i64x2_shr_var(v, vec_i64x2(1, 1))))
    print_int(vec_i64_checksum(10))
    return 0

def slice_self() -> Int:
    xs := [10, 20, 30, 40]
    s := slice(xs, 1, 2)
    print_int(slice_len(s))
    print_int(slice_get(s, 1))
    yb := [true, false, true]
    sb := slice(yb, 0, 3)
    if lpp_slice_get_bool(sb, 2):
        print_str("ok_bool_true")
    text := "hello world"
    ss := str_slice(text, 6, 5)
    print_str(str_slice_to_str(ss))
    print_int(slice_len(ss))
    return 0

def main() -> Int:
    simd_self()
    slice_self()
    return 0
"#;

const SIMD_SLICE_EXPECTED: &str = "\
516
7237123
28158
14474240
-7237122
220
3604480
9
8
10
3618560
143
2
30
ok_bool_true
world
5
";

// ── the closed runtime-symbol universe (the import_signature arms) ────────

const IMPORT_UNIVERSE: &[&str] = &[
    "fmod", "lpp_abs", "lpp_add_checked", "lpp_add_wrap",
    "lpp_arc_alloc_with_destructor", "lpp_arc_release", "lpp_arc_retain",
    "lpp_bool_to_str", "lpp_bswap16", "lpp_bswap32", "lpp_bswap64", "lpp_ceil",
    "lpp_closure_destroy", "lpp_clz64", "lpp_ctz64", "lpp_div_u",
    "lpp_eprint_str", "lpp_float_to_str", "lpp_floor", "lpp_ge_u", "lpp_gt_u",
    "lpp_int_to_str", "lpp_le_u", "lpp_list_get", "lpp_list_get_arc",
    "lpp_list_get_bool", "lpp_list_get_float", "lpp_list_len", "lpp_list_new",
    "lpp_list_new_arc", "lpp_list_push", "lpp_list_push_arc",
    "lpp_list_push_bool", "lpp_list_push_float", "lpp_list_set",
    "lpp_list_set_arc", "lpp_list_set_bool", "lpp_list_set_float", "lpp_lt_u",
    "lpp_max", "lpp_max_u", "lpp_min", "lpp_min_u", "lpp_mul_checked",
    "lpp_mul_wrap", "lpp_popcount64", "lpp_pow", "lpp_print_bool",
    "lpp_print_float", "lpp_print_int", "lpp_print_str", "lpp_rem_u",
    "lpp_rotl32", "lpp_rotl64", "lpp_rotr32", "lpp_rotr64", "lpp_shl_u",
    "lpp_shr_u", "lpp_slice_get", "lpp_slice_get_bool", "lpp_slice_init",
    "lpp_slice_len", "lpp_sqrt", "lpp_str_concat", "lpp_str_contains",
    "lpp_str_ends_with", "lpp_str_find", "lpp_str_len", "lpp_str_lower",
    "lpp_str_replace", "lpp_str_slice_to_str", "lpp_str_starts_with",
    "lpp_str_to_int", "lpp_str_to_u64", "lpp_str_trim", "lpp_str_upper",
    "lpp_sub_checked", "lpp_sub_wrap", "lpp_task_await", "lpp_task_destroy",
    "lpp_task_new", "lpp_task_poll", "lpp_trunc_i16", "lpp_trunc_i32",
    "lpp_trunc_i8", "lpp_trunc_u16", "lpp_trunc_u32", "lpp_trunc_u8",
    "lpp_tuple_alloc", "lpp_u64_to_hex", "lpp_u64_to_str",
    "lpp_vec_i64_checksum", "lpp_write_str",
];

// ── the four family name lists (the 5C2 table policy) ─────────────────────

const FAMILY_A_NAMES: &[&str] = &[
    "print", "print_str", "eprint_str", "print_int", "print_float",
    "print_bool", "write_str", "str_concat", "str_len", "str_contains",
    "str_starts_with", "str_ends_with", "str_find", "str_replace",
    "str_trim", "str_to_lower", "str_lower", "str_to_upper", "str_upper",
    "int_to_str", "str_to_int", "float_to_str", "bool_to_str", "u64_to_str",
    "u64_to_hex", "str_to_u64", "abs", "min", "max", "min_u", "max_u",
    "lt_u", "le_u", "gt_u", "ge_u", "shr_u", "shl_u", "div_u", "rem_u",
    "popcount64", "clz64", "ctz64", "bswap16", "bswap32", "bswap64",
    "rotl64", "rotr64", "rotl32", "rotr32", "trunc_u8", "trunc_u16",
    "trunc_u32", "trunc_i8", "trunc_i16", "trunc_i32", "add_checked",
    "sub_checked", "mul_checked", "add_wrap", "sub_wrap", "mul_wrap",
    "floor", "ceil", "pow", "sqrt", "fmod", "list_new", "list_push",
    "list_get", "list_set", "list_len",
];

const FAMILY_C_NAMES: &[&str] = &[
    "slice", "str_slice", "slice_len", "slice_get", "lpp_slice_get_bool",
    "slice_to_str", "str_slice_to_str",
];

fn is_family_a(name: &str) -> bool {
    FAMILY_A_NAMES.contains(&name)
        || (name.starts_with("lpp_") && FAMILY_A_NAMES.contains(&&name[4..]))
}

fn is_family_b(name: &str) -> bool {
    name.starts_with("vec_") || name == "lpp_vec_i64_checksum"
}

fn is_family_c(name: &str) -> bool {
    FAMILY_C_NAMES.contains(&name)
}

fn is_family_map(name: &str) -> bool {
    name.starts_with("map_") || name.starts_with("lpp_map_")
}

// ── gate 1: differential closures + tasks ─────────────────────────────────

#[test]
fn closure_and_task_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(CLOSURE_TASK_CORPUS);
    let names = Names(&package.names.symbols);

    // The cycle closure's captures must pin a type: the 4D plan's
    // pinned set is non-empty for this corpus.
    let plan = compute_ownership_plan(&program, &types)
        .unwrap_or_else(|e| panic!("ownership plan: {e:?}"));
    assert!(
        !plan.pinned_types().is_empty(),
        "the cycle corpus pins no type — the corpus rule is broken"
    );

    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "closure_task");
    assert_eq!(status, 0, "closure/task object exited {status}:\n{stdout}");

    let entry = main_function(&program, &package.names.symbols);
    let oracle = run_arc_oracle(&program, &types, entry);
    let expected: String = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "closure/task corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "closure_task");
}

// ── gate 2: differential builtin families ─────────────────────────────────

#[test]
fn builtin_family_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(BUILTIN_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "builtin");
    assert_eq!(status, 0, "builtin object exited {status}:\n{stdout}");

    let entry = main_function(&program, &package.names.symbols);
    let oracle = run_arc_oracle(&program, &types, entry);
    let expected: String = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "builtin corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "builtin");
}

// ── gate 3: self-check slices + SIMD ──────────────────────────────────────

#[test]
fn simd_and_slice_selfcheck_produces_the_hand_computed_output() {
    let (program, types, package) = pipeline(SIMD_SLICE_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "simd_slice");
    assert_eq!(status, 0, "simd/slice object exited {status}:\n{stdout}");
    assert_eq!(
        stdout, SIMD_SLICE_EXPECTED,
        "simd/slice corpus: object stdout diverges from the hand-computed output"
    );
}

// ── gate 4: determinism + census ──────────────────────────────────────────

#[test]
fn compiles_are_deterministic_and_the_census_matches_the_contract() {
    for (label, corpus) in [
        ("closure_task", CLOSURE_TASK_CORPUS),
        ("builtin", BUILTIN_CORPUS),
        ("simd_slice", SIMD_SLICE_CORPUS),
    ] {
        let (program, types, package) = pipeline(corpus);
        let names = Names(&package.names.symbols);
        let first = compile(&program, &types, &names);
        let second = compile(&program, &types, &names);
        assert_eq!(first.object, second.object, "{label}: objects differ byte-wise");
        assert_eq!(
            first.exported_symbols, second.exported_symbols,
            "{label}: export censuses differ"
        );
        assert_eq!(
            first.imported_symbols, second.imported_symbols,
            "{label}: import censuses differ"
        );
    }

    let (program, types, package) = pipeline(CLOSURE_TASK_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    // Real ELF relocatable with the generated C-ABI entry.
    assert_eq!(&module.object[..4], b"\x7fELF", "object is not ELF");
    assert_eq!(module.entry.as_deref(), Some("main"));

    // Exports: `main` + `lpp_main`, every user function by source
    // name, one `lpp_c{n}` per closure function, one `lpp_drop_s{n}`
    // per struct, one `lpp_drop_c{n}` per capturing closure, and the
    // used thunks (`__lpp_closure_thunk{n}` for zero-parameter
    // closures, `__lpp_task_thunk{n}` for the async functions whose
    // task path is taken).
    let mut expected_exports = BTreeSet::from(["main".to_owned(), "lpp_main".to_owned()]);
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

    let async_thunk_used: BTreeSet<MirFunctionId> =
        used_task_thunks(&program, main_function(&program, &package.names.symbols));

    for (id, function) in program.functions() {
        if !matches!(function.kind, MirFunctionKind::Closure) {
            continue;
        }
        expected_exports.insert(format!("lpp_c{}", id.raw()));
        let captures = program
            .function_parameters(function)
            .iter()
            .filter(|local| {
                matches!(
                    program.local(**local).expect("local").kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .count();
        let user_params = program
            .function_parameters(function)
            .iter()
            .filter(|local| {
                !matches!(
                    program.local(**local).expect("local").kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .count();
        if captures > 0 {
            expected_exports.insert(format!("lpp_drop_c{}", id.raw()));
        }
        if user_params == 0 {
            expected_exports.insert(format!("__lpp_closure_thunk{}", id.raw()));
        }
    }
    for &id in &async_thunk_used {
        expected_exports.insert(format!("__lpp_task_thunk{}", id.raw()));
    }

    assert_eq!(
        module.exported_symbols, expected_exports,
        "export census diverges from the contract"
    );

    // Imports: inside the closed runtime-symbol universe, and the
    // closure/task surface is actually present in this corpus.
    for symbol in module.imported_symbols.iter() {
        assert!(
            IMPORT_UNIVERSE.contains(&symbol.as_str()),
            "import {symbol} escapes the closed universe"
        );
    }
    for required in [
        "lpp_arc_alloc_with_destructor", "lpp_arc_release", "lpp_arc_retain",
        "lpp_closure_destroy", "lpp_tuple_alloc", "lpp_task_new",
        "lpp_task_poll", "lpp_task_await", "lpp_task_destroy", "lpp_print_str",
        "lpp_list_push", "lpp_list_push_arc", "lpp_list_get_arc", "lpp_list_len",
        "lpp_str_len",
    ] {
        assert!(
            module.imported_symbols.contains(required),
            "corpus uses {required}; the import is missing"
        );
    }
}

/// The async functions whose task path is taken: called as async
/// calls, used as function values, or the (async) entry — their
/// `__lpp_task_thunk{n}` is declared.
fn used_task_thunks(program: &MirProgram, entry: MirFunctionId) -> BTreeSet<MirFunctionId> {
    let mut used = BTreeSet::new();
    let entry_fn = program.function(entry).expect("entry exists");
    if matches!(entry_fn.kind, MirFunctionKind::Async) {
        used.insert(entry);
    }
    for (_, function) in program.functions() {
        for block_id in program.function_blocks(function) {
            let block = program.block(*block_id).expect("block retained");
            for &instruction_id in program.block_instructions(block) {
                let instruction = program.instruction(instruction_id).expect("instruction retained");
                if let lpp_mir::InstructionKind::Assign { value, .. } = &instruction.kind {
                    match value {
                        lpp_mir::Rvalue::Call { callee, .. } => {
                            if let lpp_mir::Operand::Function(callee_id) = callee
                                && matches!(
                                    program
                                        .function(*callee_id)
                                        .expect("callee retained")
                                        .kind,
                                    MirFunctionKind::Async
                                )
                            {
                                used.insert(*callee_id);
                            }
                        }
                        lpp_mir::Rvalue::Use(operand) => {
                            if let lpp_mir::Operand::Function(function_id) = operand
                                && matches!(
                                    program
                                        .function(*function_id)
                                        .expect("function retained")
                                        .kind,
                                    MirFunctionKind::Async
                                )
                            {
                                used.insert(*function_id);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    used
}

// ── gate 5: table census + rejections ─────────────────────────────────────

fn expect_code(error: &CodegenError, code: &str, function: MirFunctionId) {
    assert_eq!(error.code(), code);
    assert_eq!(error.function, Some(function), "typed rejection must name the exact function");
}

#[test]
fn builtin_table_census_and_family_d_rejection_are_exact() {
    // The registry enumerates to exactly 518 = Family A ∪ B ∪ C ∪ MAP ∪ D,
    // disjoint (by `BuiltinId`; a few names carry two ids).
    let builtins = lpp_runtime_abi::generated::BUILTINS;
    let mut counts = [0usize; 5];
    for builtin in builtins.iter() {
        let name = builtin.name;
        let family = if is_family_a(name) {
            0
        } else if is_family_b(name) {
            1
        } else if is_family_c(name) {
            2
        } else if is_family_map(name) {
            4
        } else {
            3
        };
        counts[family] += 1;
    }
    assert_eq!(builtins.len(), 518, "the registry must enumerate 518 builtins");
    assert_eq!(counts[0], 110, "family A ids (71 names, dual spellings)");
    assert_eq!(counts[1], 19, "family B ids (18 names + the checksum dual)");
    assert_eq!(counts[2], 7, "family C ids");
    // Family MAP: the hash map runtime, now lowered natively (was Family D).
    assert_eq!(counts[4], 30, "family MAP ids (hash map runtime)");
    assert_eq!(counts[3], 352, "family D ids (382 minus the 30 map ids)");
    assert_eq!(
        counts.iter().sum::<usize>(),
        builtins.len(),
        "the families must partition the registry"
    );

    // A Family D builtin fails with the exact E5003 at the exact
    // function; tuples stay the only E5001 non-builtin rejection.
    let (program, types, package) =
        pipeline("def main() -> Int:\n    w := webview_window_create(\"t\", 0, 0, 0)\n    return w\n");
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    let error = CraneliftBackend
        .compile_module(&program, &types, &options)
        .err()
        .unwrap_or_else(|| panic!("webview: expected E5003, got Ok"));
    expect_code(&error, "E5003", main_function(&program, &package.names.symbols));
    match &error.kind {
        CodegenErrorKind::UnrepresentableBuiltin { builtin, .. } => {
            assert_eq!(builtin.descriptor().name, "webview_window_create");
        }
        kind => panic!("webview: wrong kind {kind:?}"),
    }

    // Tuples are now lowered as flat heap records, so a tuple program compiles.
    let (program, types, package) =
        pipeline("def main() -> Int:\n    t := (1, 2)\n    return t.0\n");
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    CraneliftBackend
        .compile_module(&program, &types, &options)
        .unwrap_or_else(|e| panic!("tuple: expected Ok, got {e}"));
}
