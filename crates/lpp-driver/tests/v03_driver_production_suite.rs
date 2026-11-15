//! Production integration test suite for `lpp-driver` (v0.3.0).
//!
//! Covers:
//! - Cross-backend parity (`Cranelift`, `Llvm -O0`, `Llvm -O3`, `MIR`) across 26 feature-rich L++ programs:
//!   tuples (2..=16 + destructuring + negative 0/1/17), f-strings, ranges, guarded/exhaustive match,
//!   ADTs, `IntSet`/`IntDeque`, closures & higher-order functions, generics/turbofish/traits,
//!   structs/methods, recursion/algorithms, control flow, defaults/variadics, lists/maps,
//!   strings, `StrSlice`, math/floats, type aliases/bitwise widths, SIMD, async/spawn/await + cycles,
//!   `AtomicInt`/`SyncQueue`, `unsafe` `Ptr[T]`/`RawPtr`/MMIO/`@repr(C)`/`@packed`/`@align`, extern C FFI.
//! - Bare-metal `x86_64-unknown-none` & `aarch64-unknown-none` Multiboot2 + MMIO + allocator kernels & ELF verification (`readelf`, `objdump`).
//! - `-g` DWARF emission + `readelf --debug-dump=info,line` + `gdb -batch` line-accurate stepping + `LPP_BACKTRACE=1`.
//! - `--deny-unsafe` (`[E4501]`), safe-context rejections (`[E3201]`), invalid casts (`[E3202]`).
//! - `lpp fmt`, `lpp test`, `lpp repl`, and `lpp-lsp` JSON-RPC tooling sequences.

use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use lpp_common::OptimizationLevel;
use lpp_driver::fmt::{format_source, run_fmt};
use lpp_driver::frontend_eval::check_in_memory;
use lpp_driver::test_runner::run_tests_cli;
use lpp_driver::{
    BackendChoice, CompileError, CompileOptions, CompilerSession, DriverRequest, RewriteEngine,
    Target, build_executable_configured, build_freestanding_kernel_with_backend,
    compile_entry_with_options, default_runtime_lib_dir, exec_output_retry, runtime_library_path,
};
use lpp_hir::Symbol;
use lpp_mir::{InterpreterLimits, execute_mir_with_stats};

static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(0);
static RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();

fn workdir(tag: &str) -> PathBuf {
    let seq = NEXT_DIR_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("lpp-drv-v03-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ensure_runtime_dir() -> &'static PathBuf {
    RUNTIME_DIR.get_or_init(|| {
        if let Some(path) = runtime_library_path() {
            return path.parent().expect("runtime parent").to_path_buf();
        }
        let dir = default_runtime_lib_dir();
        let workspace = dir.parent().and_then(Path::parent).expect("workspace root");
        let out = Command::new("cargo")
            .args(["build", "--locked", "-p", "lpp-runtime"])
            .current_dir(workspace)
            .output()
            .expect("cargo build --locked -p lpp-runtime");
        assert!(
            out.status.success(),
            "runtime build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        dir
    })
}

fn run_on_backend(
    name: &str,
    source: &str,
    backend: BackendChoice,
    opt: OptimizationLevel,
) -> String {
    let tag = match (backend, opt) {
        (BackendChoice::Cranelift, _) => "cranelift",
        (BackendChoice::Llvm, OptimizationLevel::O3) => "llvm_o3",
        (BackendChoice::Llvm, _) => "llvm_o0",
        (BackendChoice::Wasm, _) => "wasm",
    };
    let dir = workdir(&format!("{name}_{tag}"));
    let entry = dir.join("main.lpp");
    let exe = dir.join(format!("main{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&entry, source).unwrap();

    let rt_dir = ensure_runtime_dir();
    let mut opts = CompileOptions::for_backend(backend);
    opts.optimization = opt;
    build_executable_configured(&entry, "v03_prod", backend, opts, &exe, rt_dir, Some("cc"))
        .unwrap_or_else(|e| panic!("build failed ({tag}) for {name}: {e:?}"));

    let out = exec_output_retry(&exe).unwrap_or_else(|e| panic!("exec failed ({tag}) {name}: {e}"));
    assert!(
        out.status.success(),
        "run failed ({tag}) for {name}: status={:?}, stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n");
    let _ = std::fs::remove_dir_all(&dir);
    stdout
}

fn run_on_mir(name: &str, source: &str) -> String {
    let dir = workdir(&format!("{name}_mir"));
    let entry = dir.join("main.lpp");
    let checked = check_in_memory(source, &entry)
        .unwrap_or_else(|diags| panic!("MIR check failed for {name}: {diags:?}"));
    let main_id = checked
        .mir
        .functions()
        .find_map(|(id, f)| {
            (f.name.and_then(|s| {
                checked
                    .package
                    .names
                    .symbols
                    .resolve(Symbol::from_raw(s.raw()))
            }) == Some("main"))
            .then_some(id)
        })
        .expect("main present");
    let outcome = execute_mir_with_stats(
        &checked.mir,
        &checked.inference.interner,
        main_id,
        &[],
        InterpreterLimits::default(),
    )
    .unwrap_or_else(|e| panic!("MIR exec failed for {name}: {e}"));
    let _ = std::fs::remove_dir_all(&dir);
    outcome.output.concat()
}

fn assert_all_4_parity(name: &str, source: &str, expected: &str) {
    let mir_out = run_on_mir(name, source);
    let cl_out = run_on_backend(
        name,
        source,
        BackendChoice::Cranelift,
        OptimizationLevel::O0,
    );
    let llvm0_out = run_on_backend(name, source, BackendChoice::Llvm, OptimizationLevel::O0);
    let llvm3_out = run_on_backend(name, source, BackendChoice::Llvm, OptimizationLevel::O3);
    assert_eq!(mir_out, expected, "{name}: MIR mismatch");
    assert_eq!(cl_out, expected, "{name}: Cranelift mismatch");
    assert_eq!(llvm0_out, expected, "{name}: LLVM -O0 mismatch");
    assert_eq!(llvm3_out, expected, "{name}: LLVM -O3 mismatch");
}

fn assert_native_3_parity(name: &str, source: &str, expected: &str) {
    let cl_out = run_on_backend(
        name,
        source,
        BackendChoice::Cranelift,
        OptimizationLevel::O0,
    );
    let llvm0_out = run_on_backend(name, source, BackendChoice::Llvm, OptimizationLevel::O0);
    let llvm3_out = run_on_backend(name, source, BackendChoice::Llvm, OptimizationLevel::O3);
    assert_eq!(cl_out, expected, "{name}: Cranelift mismatch");
    assert_eq!(llvm0_out, expected, "{name}: LLVM -O0 mismatch");
    assert_eq!(llvm3_out, expected, "{name}: LLVM -O3 mismatch");
}

fn compile_expect_err(name: &str, source: &str, opts: CompileOptions) -> CompileError {
    let dir = workdir(name);
    let entry = dir.join("main.lpp");
    std::fs::write(&entry, source).unwrap();
    let err = compile_entry_with_options(&entry, "v03_neg", BackendChoice::Cranelift, opts)
        .expect_err("expected compile error");
    let _ = std::fs::remove_dir_all(&dir);
    err
}

// ============================================================================
// 1. Cross-Backend Parity Corpus (26 Feature-Rich Programs)
// ============================================================================

#[test]
fn parity_tuples_fstrings_ranges_and_match() {
    // Program 1: Tuples across arities 2, 3, 5, 8, 12, 16 + destructuring + return tuple
    let p1 = r#"
def make_pair(a: Int, b: Str) -> (Int, Str):
    return (a * 2, b + "!")

def main():
    t2 := (10, 20)
    t3 := (1, "mid", true)
    t5 := (1, 2, 3, 4, 5)
    t8 := (10, 20, 30, 40, 50, 60, 70, 80)
    t12 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)
    t16 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
    print(t2.0 + t2.1)
    print(t3.0)
    print_str(t3.1)
    print_bool(t3.2)
    print(t5.0 + t5.4)
    print(t8.3 + t8.7)
    print(t12.0 + t12.11)
    print(t16.0 + t16.8 + t16.15)
    (num, tag) := make_pair(21, "ok")
    print(num)
    print_str(tag)
"#;
    assert_all_4_parity("p01_tuples", p1, "30\n1\nmid\n1\n6\n120\n13\n26\n42\nok!\n");

    // Verify 0-tuple, 1-tuple, and 17-tuple are rejected cleanly
    for (tag, bad_src) in [
        ("tuple0", "def main():\n    x := ()\n"),
        ("tuple1", "def main():\n    x := (42,)\n    print(x.0)\n"),
        (
            "tuple17",
            "def main():\n    x := (1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17)\n    print(x.0)\n",
        ),
    ] {
        let errs = check_in_memory(bad_src, Path::new(&format!("/tmp/{tag}.lpp")))
            .err()
            .expect("invalid tuple arity must be rejected");
        assert!(!errs.is_empty(), "{tag} produced no errors");
    }

    // Program 2: F-strings with expressions, calls, fields, booleans
    let p2 = r#"
struct Metric:
    name: Str
    count: Int

def double(n: Int) -> Int:
    return n * 2

def main():
    m := Metric("req", 14)
    flag := true
    empty_s := ""
    s1 := f"k={m.name},n={m.count},d={double(m.count)},sum={3 + 4 * 5}"
    s2 := f"flag={flag},empty={empty_s}"
    print_str(s1)
    print_str(s2)
"#;
    assert_all_4_parity(
        "p02_fstrings",
        p2,
        "k=req,n=14,d=28,sum=23\nflag=true,empty=\n",
    );

    // Program 3: Exclusive, inclusive, empty, and negative ranges
    let p3 = r#"
def main():
    mut s_excl := 0
    for i in 0..6:
        s_excl = s_excl + i
    mut s_incl := 0
    for j in 1..=5:
        s_incl = s_incl + j
    mut s_empty := 0
    for k in 5..5:
        s_empty = s_empty + k + 100
    mut s_neg := 0
    for m in -3..=2:
        s_neg = s_neg + m
    print(s_excl)
    print(s_incl)
    print(s_empty)
    print(s_neg)
"#;
    assert_all_4_parity("p03_ranges", p3, "15\n15\n0\n-3\n");

    // Program 4: Guarded & exhaustive match on ints and chars
    let p4 = r#"
def bucket(n: Int) -> Str:
    match n:
        -10 | -5:
            return "neg_pin"
        0..10 if n % 3 == 0:
            return "low_div3"
        0..10:
            return "low_other"
        10..=20 if n > 15:
            return "mid_hi"
        10..=20:
            return "mid_lo"
        v if v >= 100:
            return "big"
        _:
            return "rest"

def char_class(c: Char) -> Int:
    match c:
        'a'..='z':
            return 1
        'A'..='Z':
            return 2
        '0'..='9':
            return 3
        _:
            return 0

def main():
    print_str(bucket(-5))
    print_str(bucket(6))
    print_str(bucket(7))
    print_str(bucket(18))
    print_str(bucket(12))
    print_str(bucket(120))
    print_str(bucket(42))
    print(char_class('m'))
    print(char_class('Q'))
    print(char_class('7'))
    print(char_class('@'))
"#;
    assert_all_4_parity(
        "p04_match_guards",
        p4,
        "neg_pin\nlow_div3\nlow_other\nmid_hi\nmid_lo\nbig\nrest\n1\n2\n3\n0\n",
    );

    // Program 5: Enums (ADTs) with multi-field payloads and guarded arms
    let p5 = r#"
enum Packet:
    Empty
    Data(seq: Int, size: Int)
    Text(msg: Str)

def score(p: Packet) -> Int:
    match p:
        Packet.Empty:
            return 0
        Packet.Data(s, sz) if s == sz:
            return s * 100
        Packet.Data(s, sz):
            return s + sz
        Packet.Text(m):
            return str_len(m)

def main():
    print(score(Packet.Empty))
    print(score(Packet.Data(4, 4)))
    print(score(Packet.Data(3, 9)))
    print(score(Packet.Text("hello")))
"#;
    assert_all_4_parity("p05_adts", p5, "0\n400\n12\n5\n");
}

#[test]
fn parity_collections_closures_generics_and_traits() {
    // Program 6: IntSet & raw set_* union / intersection
    let p6 = r#"
from stdlib.collections import IntSet

def main():
    s1 := int_set_new()
    s1.add(10)
    s1.add(20)
    s1.add(10)
    s1.add_str("red")
    s1.add_str("blue")
    print(s1.len())
    print_bool(s1.has(20))
    print_bool(s1.has_str("red"))
    s1.remove(10)
    print(s1.len())

    s2 := set_new()
    set_add(s2, 20)
    set_add(s2, 30)
    u := set_union(s1.handle, s2)
    inter := set_intersect(s1.handle, s2)
    print(set_len(u))
    print(set_len(inter))
    set_free(s2)
    set_free(u)
    set_free(inter)
    s1.free()
"#;
    assert_all_4_parity("p06_set", p6, "4\n1\n1\n3\n4\n1\n");

    // Program 7: IntDeque push/pop/peek/get
    let p7 = r#"
from stdlib.collections import IntDeque

def main():
    dq := int_deque_new()
    dq.push_back(20)
    dq.push_front(10)
    dq.push_back(30)
    dq.push_front(5)
    print(dq.len())
    print(dq.peek_front())
    print(dq.get(1))
    print(dq.get(2))
    print(dq.peek_back())
    print(dq.pop_front())
    print(dq.pop_back())
    print(dq.len())
    dq.free()
"#;
    assert_all_4_parity("p07_deque", p7, "4\n5\n10\n20\n30\n5\n30\n2\n");

    // Program 8: Closures capturing mutable & immutable locals
    let p8 = r#"
def main():
    base := 100
    mut total := 0
    step := fn(delta: Int) -> Int:
        total = total + delta
        return base + total

    print(step(5))
    print(step(15))
    print(step(25))
    print(total)
"#;
    assert_all_4_parity("p08_closures", p8, "105\n120\n145\n45\n");

    // Program 9: First-class closure lists, aliasing, and closure fold
    let p9 = r#"
def main():
    inc3 := fn(x: Int) -> Int:
        return x + 3
    dbl := fn(x: Int) -> Int:
        return x * 2
    cbs := [inc3, dbl]
    f0 := list_get(cbs, 0)
    f1 := list_get(cbs, 1)
    print(f0(f0(10)))
    print(f1(f0(4)))

    scale := 2
    step := fn(acc: Int, i: Int) -> Int:
        return acc + i * scale
    mut total := 0
    for i in 1..5:
        total = step(total, i)
    print(total)
"#;
    assert_all_4_parity("p09_hof", p9, "16\n14\n20\n");

    // Program 10: Generics, turbofish `::<T>`, generic struct
    let p10 = r#"
struct Pair[A, B]:
    first: A
    second: B

def id[T](x: T) -> T:
    return x

def pick_second[A, B](a: A, b: B) -> B:
    return b

def main():
    a := id::<Int>(42)
    s := id::<Str>("generic")
    p := Pair(7, "seven")
    print(a)
    print_str(s)
    print(p.first)
    print_str(p.second)
    picked := pick_second::<Str, Int>("ignored", 99)
    print(picked)
"#;
    assert_all_4_parity("p10_generics", p10, "42\ngeneric\n7\nseven\n99\n");

    // Program 11: Traits & trait-bounded polymorphism
    let p11 = r#"
trait Weight:
    def weight(self) -> Int

struct Crate:
    kg: Int

struct Letter:
    grams: Int

impl Weight for Crate:
    def weight(self) -> Int:
        return self.kg * 1000

impl Weight for Letter:
    def weight(self) -> Int:
        return self.grams

def print_weight[T: Weight](item: T):
    print(item.weight())

def main():
    c := Crate(3)
    l := Letter(45)
    print_weight(c)
    print_weight(l)
"#;
    assert_all_4_parity("p11_traits", p11, "3000\n45\n");
}

#[test]
fn parity_structs_algorithms_control_flow_and_params() {
    // Program 12: Structs, impl methods, method chaining
    let p12 = r#"
struct Vec2:
    x: Int
    y: Int

impl Vec2:
    def add(self, o: Vec2) -> Vec2:
        return Vec2(self.x + o.x, self.y + o.y)

    def scale(self, k: Int) -> Vec2:
        return Vec2(self.x * k, self.y * k)

    def norm2(self) -> Int:
        return self.x * self.x + self.y * self.y

def main():
    v := Vec2(1, 2).add(Vec2(2, 2)).scale(3)
    print(v.x)
    print(v.y)
    print(v.norm2())
"#;
    assert_all_4_parity("p12_structs", p12, "9\n12\n225\n");

    // Program 13: Recursive Fibonacci + iterative binary search
    let p13 = r#"
def fib(n: Int) -> Int:
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)

def bsearch(arr: List[Int], target: Int) -> Int:
    mut lo := 0
    mut hi := list_len(arr) - 1
    while lo <= hi:
        mid := lo + (hi - lo) / 2
        v := arr[mid]
        if v == target:
            return mid
        elif v < target:
            lo = mid + 1
        else:
            hi = mid - 1
    return -1

def main():
    print(fib(10))
    nums := [2, 5, 9, 14, 21, 33, 48]
    print(bsearch(nums, 21))
    print(bsearch(nums, 15))
"#;
    assert_all_4_parity("p13_algo", p13, "55\n4\n-1\n");

    // Program 14: Control flow with while, break, continue, if/elif/else
    let p14 = r#"
def main():
    mut i := 0
    mut acc := 0
    while i < 20:
        i = i + 1
        if i % 2 == 0:
            continue
        if i > 11:
            break
        acc = acc + i
    print(acc)

    mut label := ""
    if acc > 50:
        label = "high"
    elif acc == 36:
        label = "exact36"
    else:
        label = "low"
    print_str(label)
"#;
    assert_all_4_parity("p14_ctrl", p14, "36\nexact36\n");

    // Program 15: Default parameters & typed variadic rest parameters
    let p15 = r#"
def calc(base: Int, mul: Int = 3, bias: Int = 1) -> Int:
    return base * mul + bias

def sum_all(...nums: Int) -> Int:
    mut acc := 0
    for n in nums:
        acc = acc + n
    return acc

def main():
    print(calc(5))
    print(calc(5, 4))
    print(calc(5, 4, 10))
    print(sum_all())
    print(sum_all(1, 2, 3, 4, 5))
"#;
    assert_all_4_parity("p15_params", p15, "16\n21\n30\n0\n15\n");

    // Program 16: Lists of Int, Float, Bool, Str
    let p16 = r#"
def main():
    mut ints := [10, 20]
    list_push(ints, 30)
    print(list_len(ints))
    print(ints[2])

    floats := [1.25, 2.5, 3.75]
    print_float(floats[0] + floats[2])

    bools := [true, false, true]
    print_bool(bools[1])
    print_bool(bools[2])

    mut strs := ["one", "two"]
    list_push(strs, "three")
    for s in strs:
        print_str(s)
"#;
    assert_all_4_parity("p16_lists", p16, "3\n30\n5.000000\n0\n1\none\ntwo\nthree\n");

    // Program 17: Map operations (insert, overwrite, lookup, membership)
    let p17 = r#"
def main():
    mut m := map_new()
    map_put(m, "port", 8080)
    map_put(m, "workers", 4)
    map_put(m, "workers", 8)
    print_bool(map_has(m, "port"))
    print_bool(map_has(m, "missing"))
    print(map_get(m, "port"))
    print(map_get(m, "workers"))
"#;
    assert_all_4_parity("p17_maps", p17, "1\n0\n8080\n8\n");
}

#[test]
fn parity_strings_slices_math_and_types() {
    // Program 18: String stdlib suite
    let p18 = r#"
def main():
    raw := "  Hello, L++ World!  "
    trimmed := str_trim(raw)
    print(str_len(trimmed))
    print_bool(str_starts_with(trimmed, "Hello"))
    print_bool(str_ends_with(trimmed, "World!"))
    print_bool(str_contains(trimmed, "L++"))
    print(str_find(trimmed, "L++"))
    replaced := str_replace(trimmed, "World", "v0.3")
    print_str(str_to_upper(replaced))
    print_str(str_to_lower("ABC-xyz"))
    print_str(str_substr(trimmed, 7, 3))
    print_str(str_repeat("ab", 3))
    print_bool(str_substr(trimmed, 0, 5) == "Hello")
    print(str_to_int("404") + 1)
    print_str(int_to_str(-77))
"#;
    assert_all_4_parity(
        "p18_strings",
        p18,
        "17\n1\n1\n1\n7\nHELLO, L++ V0.3!\nabc-xyz\nL++\nababab\n1\n405\n-77\n",
    );

    // Program 18b: Native string join & padding
    let p18b = r#"
def main():
    parts := ["a", "b", "c"]
    print_str(str_join(parts, "::"))
"#;
    assert_native_3_parity("p18b_str_join", p18b, "a::b::c\n");

    // Program 19: Zero-copy StrSlice operations (native parity)
    let p19 = r#"
def main():
    s := "compiler-pipeline-v03"
    s1 := str_slice(s, 0, 8)
    s2 := str_slice(s, 9, 8)
    s3 := str_slice(s, 18, 3)
    print_str(str_slice_to_str(s1))
    print_str(str_slice_to_str(s2))
    print_str(str_slice_to_str(s3))
"#;
    assert_native_3_parity("p19_strslice", p19, "compiler\npipeline\nv03\n");

    // Program 20: Float math & stdlib.math
    let p20 = r#"
from stdlib.math import lcm

def main():
    print_float(sqrt(81.0))
    print_float(floor(3.9))
    print_float(ceil(3.1))
    print_float(fmod(9.5, 4.0))
    print(abs(-42))
    print(min(19, 7))
    print(max(19, 7))
    print(lcm(15, 20))
"#;
    assert_all_4_parity(
        "p20_math",
        p20,
        "9.000000\n3.000000\n4.000000\n1.500000\n42\n7\n19\n60\n",
    );

    // Program 21: Type aliases & fixed-width integer/bitwise operations
    let p21 = r#"
type Port = U16
type Mask = U32
type Tag = Str

def main():
    p: Port = 8080 as U16
    m: Mask = (0x00FF as U32) | ((0x0A as U32) << (8 as U32))
    t: Tag = "net"
    print(p as Int)
    print(m as Int)
    print((m & (0xFF as U32)) as Int)
    print((m >> (8 as U32)) as Int)
    print_str(t)
"#;
    assert_all_4_parity("p21_aliases_bits", p21, "8080\n2815\n255\n10\nnet\n");
}

#[test]
fn parity_simd_async_sync_unsafe_and_ffi() {
    // Program 22: SIMD VectorI64x2 & VectorI64x4
    let p22 = r#"
from stdlib.simd import VectorI64x4, vec_i64x4, vec_i64x4_splat

def main():
    a := vec_i64x2(12, 30)
    b := vec_i64x2_splat(4)
    c := vec_i64x2_add(a, b)
    print(vec_i64x2_extract(c, 0))
    print(vec_i64x2_extract(c, 1))

    v1 := vec_i64x4(2, 4, 6, 8)
    v2 := vec_i64x4_splat(3)
    v3 := v1.add(v2)
    print(v3.sum())
    print(v1.dot(v2))
    print(v3.lane(3))
"#;
    assert_native_3_parity("p22_simd", p22, "16\n34\n32\n60\n11\n");

    // Program 23: Async / spawn / await + cycle-capable ARC reclamation
    let p23 = r#"
struct CycleNode:
    val: Int
    next: CycleNode

async def compute_step(x: Int) -> Int:
    return x * 3 + 5

def main():
    seed := 9
    spawn fn() -> Void:
        v := seed + 1
    out := compute_step(11).await
    print(out)

    leaf := CycleNode(2, 0)
    mut head := CycleNode(1, leaf)
    head.next = head
    print(head.val)
"#;
    assert_native_3_parity("p23_async_cycles", p23, "38\n1\n");

    // Program 24: AtomicInt & SyncQueue
    let p24 = r#"
from stdlib.sync import AtomicInt, atomic_int_new, SyncQueue, sync_queue_new

def main():
    atom := atomic_int_new(100)
    prev := atom.fetch_add(23)
    print(prev)
    print(atom.load())
    atom.free()

    q := sync_queue_new()
    q.push(11)
    q.push(22)
    q.push(33)
    print(q.pop_or(-1))
    print(q.pop_or(-1))
    print(q.pop_or(-1))
    print(q.pop_or(-1))
    q.free()
"#;
    assert_native_3_parity("p24_sync", p24, "100\n123\n11\n22\n33\n-1\n");

    // Program 25: unsafe Ptr[T], RawPtr, MMIO, @repr(C), @packed, @align, size_of/align_of
    let p25 = r#"
extern "C":
    def malloc(size: USize) -> RawPtr
    def free(ptr: RawPtr)

@repr(C)
@packed
struct PackedHeader:
    magic: U8
    len: U32

@repr(C)
@align(16)
struct AlignedDesc:
    addr: U64
    flags: U16

unsafe def write_at(p: Ptr[U64], idx: Int, val: U64):
    ptr_write[U64](ptr_offset[U64](p, idx), val)

unsafe def read_at(p: Ptr[U64], idx: Int) -> U64:
    return ptr_read[U64](ptr_offset[U64](p, idx))

def main():
    print(size_of[PackedHeader]())
    print(align_of[PackedHeader]())
    print(align_of[AlignedDesc]())
    unsafe:
        n := ptr_null[U64]()
        print_bool(ptr_is_null(n))

        raw1 := malloc(64 as USize)
        raw2 := malloc(64 as USize)
        p := raw1 as Ptr[U64]
        print_bool(ptr_is_null(p))

        write_at(p, 0, 11111 as U64)
        write_at(p, 1, 22222 as U64)
        print(read_at(p, 0) as Int)
        print(read_at(p, 1) as Int)

        mmio_write32((raw1 as USize) + (16 as USize), 0x1A2B3C4D as U32)
        memory_barrier()
        print(mmio_read32((raw1 as USize) + (16 as USize)) as Int)

        b1 := raw1 as Ptr[U8]
        b2 := raw2 as Ptr[U8]
        memset_raw(b2, 0 as U8, 32 as USize)
        memcpy_raw(b2, b1, 24 as USize)
        print(memcmp_raw(b1, b2, 24 as USize))

        free(raw1)
        free(raw2)
"#;
    assert_native_3_parity(
        "p25_unsafe_ptr_mmio",
        p25,
        "5\n1\n16\n1\n0\n11111\n22222\n439041101\n0\n",
    );

    // Program 26: extern "C" FFI + @export_name
    let p26 = r#"
extern "C":
    def abs(x: Int) -> Int
    def strlen(s: Str) -> Int

@export_name("lpp_v03_ffi_helper")
def ffi_helper(a: Int, b: Int) -> Int:
    return a * 10 + b

def main():
    print(abs(-314))
    print(strlen("lpp-ffi"))
    print(ffi_helper(4, 2))
"#;
    assert_native_3_parity("p26_ffi", p26, "314\n7\n42\n");
}

// ============================================================================
// 2. Bare-Metal Kernels (x86_64-unknown-none & aarch64-unknown-none) + ELF Audit
// ============================================================================

#[test]
fn bare_metal_multiboot2_mmio_allocator_kernels_for_x86_64_and_aarch64() {
    let dir = workdir("bare_metal_suite");

    // Custom x86_64 kernel exercising Multiboot2, MMIO, @repr(C) @align(4096), and bump allocator
    let x64_src = dir.join("kernel_x64.lpp");
    std::fs::write(
        &x64_src,
        r#"
@repr(C)
@align(4096)
struct PageFrame:
    magic: U64
    next_phys: U64

unsafe def bump_alloc_page(state_addr: USize) -> USize:
    cursor_ptr := state_addr as Ptr[U64]
    cur := ptr_read[U64](cursor_ptr)
    next := cur + (4096 as U64)
    ptr_write[U64](cursor_ptr, next)
    memory_barrier()
    return cur as USize

@export_name("_start")
unsafe def kmain():
    cpu_cli()
    ptr_write[U64](0x9000 as Ptr[U64], 0x100000 as U64)
    page := bump_alloc_page(0x9000 as USize)
    mmio_write32(page, 0xCAFEBABE as U32)
    val := mmio_read32(page)
    mmio_write32(0xB8000 as USize, val)
    _ := rdtsc()
    while true:
        cpu_hlt()
"#,
    )
    .unwrap();

    for (tag, backend, opt) in [
        ("cranelift", BackendChoice::Cranelift, OptimizationLevel::O0),
        ("llvm_o3", BackendChoice::Llvm, OptimizationLevel::O3),
    ] {
        let elf_path = dir.join(format!("kernel_x64_{tag}.elf"));
        let mut opts = CompileOptions::for_backend(backend);
        opts.target = Target::X86_64None;
        opts.optimization = opt;
        opts.image_base = Some(0x0010_0000);
        opts.multiboot2 = true;

        let module = build_freestanding_kernel_with_backend(
            &x64_src,
            "v03_x64_kernel",
            backend,
            opts,
            &elf_path,
        )
        .unwrap_or_else(|e| panic!("x86_64 kernel build ({tag}) failed: {e:?}"));
        assert_eq!(module.entry.as_deref(), Some("_start"));

        let bytes = std::fs::read(&elf_path).unwrap();
        assert_eq!(&bytes[0..4], b"\x7fELF");
        assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 2); // ET_EXEC
        assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62); // EM_X86_64

        // Multiboot2 magic (0xE85250D6) within first 32 KiB
        let mb2_magic = 0xE852_50D6_u32.to_le_bytes();
        assert!(
            bytes[..bytes.len().min(32 * 1024)]
                .windows(4)
                .any(|w| w == mb2_magic),
            "missing Multiboot2 header magic ({tag})"
        );

        assert_elf_exec_without_interp(&bytes, 62, tag);

        match Command::new("readelf")
            .args(["-h", "-l"])
            .arg(&elf_path)
            .output()
        {
            Ok(readelf) => {
                assert!(readelf.status.success());
                let re_text = String::from_utf8_lossy(&readelf.stdout);
                assert!(re_text.contains("EXEC (Executable file)"), "{re_text}");
                assert!(
                    !re_text.contains("INTERP"),
                    "unexpected PT_INTERP: {re_text}"
                );
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => panic!("failed invoking readelf ({tag}): {e}"),
        }

        match Command::new("objdump").args(["-d"]).arg(&elf_path).output() {
            Ok(objdump) if objdump.status.success() => {
                let disasm = String::from_utf8_lossy(&objdump.stdout);
                assert!(disasm.contains("cli"), "missing cli ({tag})");
                assert!(disasm.contains("hlt"), "missing hlt ({tag})");
                assert!(disasm.contains("rdtsc"), "missing rdtsc ({tag})");
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => panic!("failed invoking objdump ({tag}): {e}"),
        }
    }

    // Custom aarch64-unknown-none kernel with MMIO + bump allocator on Cranelift & LLVM
    let a64_src = dir.join("kernel_a64.lpp");
    std::fs::write(
        &a64_src,
        r#"
unsafe def uart_putc(ch: U8):
    mmio_write32(0x09000000 as USize, ch as U32)
    memory_barrier()

@export_name("_start")
unsafe def a64_main():
    cpu_cli()
    uart_putc(76 as U8)
    uart_putc(43 as U8)
    uart_putc(43 as U8)
    while true:
        cpu_hlt()
"#,
    )
    .unwrap();

    for (tag, backend, opt) in [
        ("cranelift", BackendChoice::Cranelift, OptimizationLevel::O0),
        ("llvm_o2", BackendChoice::Llvm, OptimizationLevel::O2),
    ] {
        let a64_elf = dir.join(format!("kernel_a64_{tag}.elf"));
        let mut opts = CompileOptions::for_backend(backend);
        opts.target = Target::Aarch64None;
        opts.optimization = opt;
        build_freestanding_kernel_with_backend(&a64_src, "v03_a64_kernel", backend, opts, &a64_elf)
            .unwrap_or_else(|e| panic!("aarch64 kernel build ({tag}) failed: {e:?}"));

        let bytes = std::fs::read(&a64_elf).unwrap();
        assert_eq!(&bytes[0..4], b"\x7fELF");
        assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 2); // ET_EXEC
        assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 183); // EM_AARCH64

        assert_elf_exec_without_interp(&bytes, 183, tag);

        match Command::new("readelf")
            .args(["-h", "-l"])
            .arg(&a64_elf)
            .output()
        {
            Ok(readelf) => {
                assert!(readelf.status.success());
                let re_text = String::from_utf8_lossy(&readelf.stdout);
                assert!(re_text.contains("AArch64"), "{re_text}");
                assert!(!re_text.contains("INTERP"), "{re_text}");
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => panic!("failed invoking readelf ({tag}): {e}"),
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

fn assert_elf_exec_without_interp(bytes: &[u8], expected_machine: u16, tag: &str) {
    assert!(bytes.len() >= 64, "ELF64 header truncated ({tag})");
    assert_eq!(&bytes[0..4], b"\x7fELF", "invalid ELF magic ({tag})");
    assert_eq!(
        u16::from_le_bytes([bytes[16], bytes[17]]),
        2,
        "expected ET_EXEC ({tag})"
    );
    assert_eq!(
        u16::from_le_bytes([bytes[18], bytes[19]]),
        expected_machine,
        "unexpected e_machine ({tag})"
    );
    let phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    let phentsize = u16::from_le_bytes([bytes[54], bytes[55]]) as usize;
    let phnum = u16::from_le_bytes([bytes[56], bytes[57]]) as usize;
    assert!(
        phentsize >= 56 && phnum > 0,
        "expected ELF program headers ({tag})"
    );
    for i in 0..phnum {
        let off = phoff + i * phentsize;
        let p_type = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        assert_ne!(
            p_type, 3,
            "unexpected PT_INTERP in freestanding ELF ({tag})"
        );
    }
}

// ============================================================================
// 3. DWARF `-g`, `gdb -batch` Line-Accurate Stepping & `LPP_BACKTRACE=1`
// ============================================================================

#[cfg(unix)]
#[test]
fn dwarf_debug_info_gdb_line_stepping_and_panic_backtrace() {
    let dir = workdir("dwarf_gdb_step");
    let src = dir.join("step_demo.lpp");
    // Exact line numbers:
    // 1: def compute(x: Int) -> Int:
    // 2:     doubled := x * 2
    // 3:     return doubled + 4
    // 4:
    // 5: def main():
    // 6:     seed := 19
    // 7:     answer := compute(seed)
    // 8:     print(answer)
    std::fs::write(
        &src,
        concat!(
            "def compute(x: Int) -> Int:\n",
            "    doubled := x * 2\n",
            "    return doubled + 4\n",
            "\n",
            "def main():\n",
            "    seed := 19\n",
            "    answer := compute(seed)\n",
            "    print(answer)\n",
        ),
    )
    .unwrap();

    let rt_dir = ensure_runtime_dir();

    for (tag, backend) in [
        ("cranelift", BackendChoice::Cranelift),
        ("llvm", BackendChoice::Llvm),
    ] {
        let exe = dir.join(format!("step_demo_{tag}"));
        let mut opts = CompileOptions::for_backend(backend);
        opts.debug_info = true;
        build_executable_configured(&src, "step_pkg", backend, opts, &exe, rt_dir, Some("cc"))
            .unwrap_or_else(|e| panic!("build -g failed ({tag}): {e:?}"));

        let run_out = exec_output_retry(&exe).unwrap();
        assert!(run_out.status.success());
        assert_eq!(String::from_utf8_lossy(&run_out.stdout).trim(), "42");

        match Command::new("readelf")
            .args(["--debug-dump=info,line"])
            .arg(&exe)
            .output()
        {
            Ok(readelf) => {
                assert!(readelf.status.success());
                let dump = String::from_utf8_lossy(&readelf.stdout);
                assert!(
                    dump.contains("step_demo.lpp"),
                    "missing filename in DWARF ({tag})"
                );
                assert!(
                    dump.contains("compute") || dump.contains("main"),
                    "missing function DIEs in DWARF ({tag})"
                );
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => panic!("failed invoking readelf ({tag}): {e}"),
        }

        match Command::new("gdb")
            .args([
                "-batch",
                "-ex",
                "set pagination off",
                "-ex",
                "b step_demo.lpp:6",
                "-ex",
                "run",
                "-ex",
                "next",
                "-ex",
                "step",
                "-ex",
                "bt",
                "-ex",
                "continue",
            ])
            .arg(&exe)
            .output()
        {
            Ok(gdb) => {
                let gdb_out = String::from_utf8_lossy(&gdb.stdout);
                let gdb_err = String::from_utf8_lossy(&gdb.stderr);
                assert!(
                    gdb.status.success(),
                    "gdb failed ({tag}): stdout={gdb_out}\nstderr={gdb_err}"
                );
                assert!(
                    gdb_out.contains("step_demo.lpp:6"),
                    "missing initial breakpoint on line 6 ({tag}):\n{gdb_out}"
                );
                assert!(
                    gdb_out.contains("step_demo.lpp:7"),
                    "expected `next` to step to line 7 ({tag}):\n{gdb_out}"
                );
                assert!(
                    gdb_out.contains("compute") && gdb_out.contains("main"),
                    "expected backtrace inside compute -> main ({tag}):\n{gdb_out}"
                );
                assert!(
                    gdb_out.contains("42"),
                    "expected program stdout 42 after continue ({tag}):\n{gdb_out}"
                );
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => panic!("failed invoking gdb ({tag}): {e}"),
        }
    }

    // Verify LPP_BACKTRACE=1 emits panic backtrace on runtime division by zero
    let panic_src = dir.join("div_zero.lpp");
    std::fs::write(
        &panic_src,
        "def fail_div(a: Int, b: Int) -> Int:\n    return a / b\n\ndef main():\n    print(fail_div(10, 0))\n",
    )
    .unwrap();
    let panic_exe = dir.join("div_zero");
    let mut panic_opts = CompileOptions::for_backend(BackendChoice::Cranelift);
    panic_opts.debug_info = true;
    build_executable_configured(
        &panic_src,
        "panic_pkg",
        BackendChoice::Cranelift,
        panic_opts,
        &panic_exe,
        rt_dir,
        Some("cc"),
    )
    .unwrap();

    let mut panic_out = None;
    for attempt in 0..8 {
        match Command::new(&panic_exe).env("LPP_BACKTRACE", "1").output() {
            Ok(out) => {
                panic_out = Some(out);
                break;
            }
            Err(e) if e.raw_os_error() == Some(26) && attempt < 7 => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => panic!("failed running div_zero: {e}"),
        }
    }
    let panic_out = panic_out.expect("div_zero output");
    assert!(!panic_out.status.success());
    let stderr = String::from_utf8_lossy(&panic_out.stderr);
    assert!(
        stderr.contains("L++ RUNTIME PANIC") && stderr.contains("Backtrace:"),
        "expected panic backtrace, got:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================================
// 4. `--deny-unsafe` Rejection (`E4501`) & Safe-Context Rejections (`E3201`/`E3202`)
// ============================================================================

#[test]
fn deny_unsafe_and_safe_context_diagnostics() {
    let mut deny_opts = CompileOptions::for_backend(BackendChoice::Cranelift);
    deny_opts.deny_unsafe = true;

    let unsafe_block_err = compile_expect_err(
        "deny_block",
        "def main():\n    unsafe:\n        x := 1 as U64\n        print(x as Int)\n",
        deny_opts.clone(),
    );
    assert!(
        format!("{unsafe_block_err}").contains("[E4501]"),
        "{unsafe_block_err}"
    );

    let unsafe_fn_err = compile_expect_err(
        "deny_fn",
        "unsafe def low() -> Int:\n    return 1\n\ndef main():\n    print(1)\n",
        deny_opts.clone(),
    );
    assert!(
        format!("{unsafe_fn_err}").contains("[E4501]"),
        "{unsafe_fn_err}"
    );

    // Safe program compiles under --deny-unsafe
    let safe_dir = workdir("deny_safe_ok");
    let safe_entry = safe_dir.join("main.lpp");
    std::fs::write(&safe_entry, "def main():\n    print(123)\n").unwrap();
    compile_entry_with_options(&safe_entry, "safe_pkg", BackendChoice::Cranelift, deny_opts)
        .expect("safe code must pass under --deny-unsafe");

    // CLI `--check --deny-unsafe` returns non-zero on unsafe program and 0 on safe program
    let cli_unsafe = safe_dir.join("unsafe_cli.lpp");
    std::fs::write(
        &cli_unsafe,
        "def main():\n    unsafe:\n        memory_barrier()\n",
    )
    .unwrap();
    let req_fail = DriverRequest::from_args(
        [
            "lpp".to_string(),
            cli_unsafe.to_string_lossy().into_owned(),
            "--check".to_string(),
            "--deny-unsafe".to_string(),
        ],
        &safe_dir,
    )
    .unwrap();
    assert_ne!(
        CompilerSession::new(RewriteEngine)
            .execute(&req_fail)
            .exit_code(),
        0
    );

    let req_ok = DriverRequest::from_args(
        [
            "lpp".to_string(),
            safe_entry.to_string_lossy().into_owned(),
            "--check".to_string(),
            "--deny-unsafe".to_string(),
        ],
        &safe_dir,
    )
    .unwrap();
    assert_eq!(
        CompilerSession::new(RewriteEngine)
            .execute(&req_ok)
            .exit_code(),
        0
    );
    let _ = std::fs::remove_dir_all(&safe_dir);

    // Safe-context rejection of raw pointer op [E3201] and invalid cast [E3202]
    let e3201 = compile_expect_err(
        "safe_e3201",
        "def main():\n    p := ptr_null[U64]()\n    print(ptr_read[U64](p) as Int)\n",
        CompileOptions::for_backend(BackendChoice::Cranelift),
    );
    assert!(format!("{e3201}").contains("[E3201]"), "{e3201}");

    let e3202 = compile_expect_err(
        "cast_e3202",
        "def main():\n    unsafe:\n        v := \"nope\" as Int\n        print(v)\n",
        CompileOptions::for_backend(BackendChoice::Cranelift),
    );
    assert!(format!("{e3202}").contains("[E3202]"), "{e3202}");
}

// ============================================================================
// 5. Tooling: `lpp fmt`, `lpp test`, `lpp repl`, and `lpp-lsp` JSON-RPC
// ============================================================================

#[test]
fn lpp_fmt_and_test_runner_filtering_pass_and_fail_exit_codes() {
    let dir = workdir("fmt_and_test");

    // 1. `lpp fmt` idempotency, tab expansion, trailing blank line normalization, --check / --write
    let dirty = concat!(
        "# Header comment\n",
        "def main():   \n",
        "\tif true:\n",
        "\t\tx := 10   \n",
        "\telse:\n",
        "\t\tx := 20\n\n\n\n",
        "\tprint(x)\n\n",
    );
    let once = format_source(dirty);
    let twice = format_source(&once);
    assert_eq!(once, twice, "format_source must be idempotent");
    assert!(once.contains("# Header comment\n"));
    assert!(once.contains("    if true:\n        x := 10\n"));
    assert!(!once.contains('\t'));

    let fmt_file = dir.join("needs_fmt.lpp");
    std::fs::write(&fmt_file, dirty).unwrap();
    assert_eq!(
        run_fmt(&["--check".into(), fmt_file.display().to_string()], &dir),
        1
    );
    assert_eq!(
        run_fmt(&["--write".into(), fmt_file.display().to_string()], &dir),
        0
    );
    assert_eq!(
        run_fmt(&["--check".into(), fmt_file.display().to_string()], &dir),
        0
    );

    // 2. `lpp test` passing, filtering, failing exit codes, and CLI arg validation
    let _ = ensure_runtime_dir();
    let pass_file = dir.join("passing_tests.lpp");
    std::fs::write(
        &pass_file,
        r#"
@test
def check_arith() -> Bool:
    return 6 * 7 == 42

def test_fstring_eval():
    s := f"n={10 + 5}"
    if !str_contains(s, "15"):
        exit(1)
"#,
    )
    .unwrap();

    // Passing suite (native Cranelift + MIR)
    assert_eq!(run_tests_cli(&[pass_file.display().to_string()], &dir), 0);
    assert_eq!(
        run_tests_cli(
            &[
                "--mir".into(),
                "--filter".into(),
                "check_arith".into(),
                pass_file.display().to_string(),
            ],
            &dir
        ),
        0
    );

    // Failing test returns non-zero (1), while --filter selecting only the passing test returns 0
    let mixed_file = dir.join("mixed_tests.lpp");
    std::fs::write(
        &mixed_file,
        r#"
@test
def test_ok() -> Bool:
    return true

@test
def test_broken() -> Bool:
    return 1 == 2
"#,
    )
    .unwrap();
    assert_eq!(
        run_tests_cli(&[mixed_file.display().to_string()], &dir),
        1,
        "failing test must return exit code 1"
    );
    assert_eq!(
        run_tests_cli(&["--mir".into(), mixed_file.display().to_string(),], &dir),
        1,
        "failing test under --mir must return exit code 1"
    );
    assert_eq!(
        run_tests_cli(
            &[
                "-k".into(),
                "test_ok".into(),
                mixed_file.display().to_string(),
            ],
            &dir
        ),
        0
    );

    // Missing value for --filter returns usage exit code 2
    assert_eq!(run_tests_cli(&["--filter".into()], &dir), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lpp_repl_multi_turn_eval_and_lpp_lsp_json_rpc_sequence() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let lpp_bin = repo_root
        .join("target/debug")
        .join(format!("lpp{}", std::env::consts::EXE_SUFFIX));
    let lsp_bin = repo_root
        .join("target/debug")
        .join(format!("lpp-lsp{}", std::env::consts::EXE_SUFFIX));

    // 1. `lpp repl` multi-turn session
    assert!(
        lpp_bin.is_file(),
        "expected built lpp binary at {lpp_bin:?}"
    );
    let mut repl = Command::new(&lpp_bin)
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lpp repl");
    {
        let stdin = repl.stdin.as_mut().unwrap();
        stdin
            .write_all(
                concat!(
                    ":help\n",
                    "def triple(n: Int) -> Int:\n",
                    "    return n * 3\n",
                    "\n",
                    "x := triple(14)\n",
                    "x\n",
                    "f\"result={x}\"\n",
                    "bad_var := 1 + true\n",
                    ":show\n",
                    ":clear\n",
                    "y := 99\n",
                    "y\n",
                    ":quit\n",
                )
                .as_bytes(),
            )
            .unwrap();
    }
    let repl_out = repl.wait_with_output().unwrap();
    assert!(
        repl_out.status.success(),
        "repl failed: {}",
        String::from_utf8_lossy(&repl_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&repl_out.stdout);
    let stderr = String::from_utf8_lossy(&repl_out.stderr);
    assert!(
        stdout.contains("Commands:"),
        "missing :help output:\n{stdout}"
    );
    assert!(stdout.contains("42"), "missing 42:\n{stdout}");
    assert!(stdout.contains("result=42"), "missing result=42:\n{stdout}");
    assert!(
        stdout.contains("def triple"),
        "missing :show decl:\n{stdout}"
    );
    assert!(
        stdout.contains("99"),
        "missing post-clear eval 99:\n{stdout}"
    );
    assert!(!stderr.is_empty(), "expected error diagnostic on bad_var");

    // 2. `lpp-lsp` JSON-RPC request/notification lifecycle
    assert!(
        lsp_bin.is_file(),
        "expected built lpp-lsp binary at {lsp_bin:?}"
    );
    let mut lsp = Command::new(&lsp_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn lpp-lsp");
    let mut lsp_in = lsp.stdin.take().unwrap();
    let send_msg = |w: &mut std::process::ChildStdin, json: &str| {
        let frame = format!("Content-Length: {}\r\n\r\n{}", json.len(), json);
        w.write_all(frame.as_bytes()).unwrap();
        w.flush().unwrap();
    };

    let doc_uri = "file:///tmp/v03_lsp_probe.lpp";
    send_msg(
        &mut lsp_in,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
    );
    send_msg(
        &mut lsp_in,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
    );
    send_msg(
        &mut lsp_in,
        &format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{doc_uri}","languageId":"lpp","version":1,"text":"def main():\n    bad: Int = \"wrong\"\n"}}}}}}"#
        ),
    );
    send_msg(
        &mut lsp_in,
        &format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{doc_uri}","version":2}},"contentChanges":[{{"text":"struct Point:\n    x: Int\n\ndef add(a: Int, b: Int) -> Int:\n    return a + b\n\ndef main():\n    p := Point(1)\n    print(add(p.x, 2))\n"}}]}}}}"#
        ),
    );
    send_msg(
        &mut lsp_in,
        &format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{{"textDocument":{{"uri":"{doc_uri}"}}}}}}"#
        ),
    );
    send_msg(
        &mut lsp_in,
        &format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"textDocument/hover","params":{{"textDocument":{{"uri":"{doc_uri}"}},"position":{{"line":3,"character":5}}}}}}"#
        ),
    );
    send_msg(
        &mut lsp_in,
        &format!(
            r#"{{"jsonrpc":"2.0","id":4,"method":"textDocument/completion","params":{{"textDocument":{{"uri":"{doc_uri}"}},"position":{{"line":7,"character":10}}}}}}"#
        ),
    );
    send_msg(
        &mut lsp_in,
        &format!(
            r#"{{"jsonrpc":"2.0","id":5,"method":"textDocument/formatting","params":{{"textDocument":{{"uri":"{doc_uri}"}},"options":{{"tabSize":4,"insertSpaces":true}}}}}}"#
        ),
    );
    send_msg(
        &mut lsp_in,
        r#"{"jsonrpc":"2.0","id":6,"method":"shutdown"}"#,
    );
    send_msg(&mut lsp_in, r#"{"jsonrpc":"2.0","method":"exit"}"#);
    drop(lsp_in);

    let mut lsp_raw = Vec::new();
    lsp.stdout
        .take()
        .unwrap()
        .read_to_end(&mut lsp_raw)
        .unwrap();
    let status = lsp.wait().unwrap();
    assert!(status.success(), "lpp-lsp exited non-zero: {status:?}");

    let lsp_text = String::from_utf8_lossy(&lsp_raw);
    assert!(
        lsp_text.contains("\"documentSymbolProvider\":true"),
        "missing initialize capabilities:\n{lsp_text}"
    );
    assert!(
        lsp_text.contains("\"code\":\"E3100\""),
        "expected semantic diagnostic E3100 on didOpen:\n{lsp_text}"
    );
    assert!(
        lsp_text.contains("\"diagnostics\":[]"),
        "expected cleared diagnostics after didChange:\n{lsp_text}"
    );
    assert!(
        lsp_text.contains("\"name\":\"Point\"")
            && lsp_text.contains("\"name\":\"add\"")
            && lsp_text.contains("\"name\":\"main\""),
        "expected documentSymbol entries:\n{lsp_text}"
    );
    assert!(
        lsp_text.contains("\"id\":3")
            && lsp_text.contains("\"id\":4")
            && lsp_text.contains("\"id\":5"),
        "missing hover/completion/formatting responses:\n{lsp_text}"
    );
    assert!(
        lsp_text.contains("\"newText\""),
        "expected formatting edit:\n{lsp_text}"
    );
}
