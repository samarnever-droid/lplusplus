//! Gate: `lpp-runtime` (Phase 6B.1 + 6B.2 — `docs/rewrite/PHASE_6.md`).
//!
//! The v1 C runtime (`lpp_runtime.c`) is the behavioral reference. This
//! gate proves the Rust re-implementation three ways:
//!
//! 1. **Behavioral** — ARC semantics (retain/release accounting,
//!    destructors exactly once, the weak-generation protocol, immortal
//!    no-ops, the local fast path) through the extern "C" ABI.
//! 2. **Differential** — the same scenario program compiled against the C
//!    runtime and against the Rust runtime prints identical fingerprints
//!    (guarded: skips cleanly when no C compiler is present).
//! 3. **Symbol census** — the Rust staticlib exports exactly the v1 ABI
//!    surface for the 6B.1 slice, and every one of those symbols is
//!    defined by the C reference object too.

use std::path::PathBuf;
use std::process::Command;

// ── 2. Differential against the C reference ───────────────────────────────

/// The scenario program: identical C and Rust implementations run the same
/// ARC sequences and print a fingerprint. Equal fingerprints = equal
/// behavior for this slice.
const C_SCENARIO: &str = r#"
#include <stdio.h>
#include <string.h>
#include <stdint.h>
extern void *lpp_arc_alloc(int64_t);
extern void *lpp_arc_alloc_with_destructor(int64_t, void (*)(void *));
extern void lpp_arc_retain(void *);
extern void lpp_arc_release(void *);
extern void lpp_arc_retain_local(void *);
extern void lpp_arc_release_local(void *);
extern int64_t lpp_weak_generation(void *);
extern int64_t lpp_weak_get(int64_t, int64_t);
extern char *lpp_empty_str(void);
static int drops;
static void dtor(void *p) { (void)p; drops++; }
int main(void) {
    char out[256] = "";
    char tmp[32];
    void *a = lpp_arc_alloc_with_destructor(32, dtor);
    int64_t ga = lpp_weak_generation(a);
    int gpos = ga > 0;
    int live = lpp_weak_get((int64_t)a, ga) == (int64_t)a;
    snprintf(tmp, sizeof tmp, "A genpos=%d live=%d ", gpos, live);
    strcat(out, tmp);
    lpp_arc_retain(a); lpp_arc_release(a); lpp_arc_release(a);
    int dead = lpp_weak_get((int64_t)a, ga) == 0;
    snprintf(tmp, sizeof tmp, "drops=%d dead=%d ", drops, dead);
    strcat(out, tmp);
    void *b = lpp_arc_alloc(16);
    lpp_arc_retain_local(b); lpp_arc_release_local(b); lpp_arc_release_local(b);
    char *e = lpp_empty_str();
    lpp_arc_retain(e); lpp_arc_release(e);
    int epos = lpp_weak_generation(e) == 0x41524331LL;
    snprintf(tmp, sizeof tmp, "B drops=%d emptypos=%d", drops, epos);
    strcat(out, tmp);
    printf("%s\n", out);
    return 0;
}
"#;

/// The golden fingerprint of the ARC scenario, produced by the v1 C
/// reference runtime. The Rust runtime asserts the same constant in
/// `arc::tests::scenario_fingerprint_is_golden`, so both implementations
/// are pinned to the same observable behavior without cross-linking.
const GOLDEN: &str = "A genpos=1 live=1 drops=1 dead=1 B drops=1 emptypos=1";

#[test]
fn c_reference_matches_the_golden() {
    let cc = match Command::new("cc").arg("--version").output() {
        Ok(o) if o.status.success() => "cc",
        _ => {
            eprintln!("skipping: no C compiler available");
            return;
        }
    };
    let dir = std::env::temp_dir().join(format!("lpp-runtime-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let scenario = dir.join("scenario.c");
    std::fs::write(&scenario, C_SCENARIO).unwrap();
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = dir.join("scenario_c");
    let status = Command::new(cc)
        .args(["-O1", "-fno-stack-protector"])
        .arg(crate_root.join("lpp_runtime.c"))
        .arg(&scenario)
        .arg("-o")
        .arg(&bin)
        .arg("-lm")
        .status()
        .unwrap();
    if !status.success() {
        eprintln!("skipping: cc failed to build the C reference");
        return;
    }
    let c_out = Command::new(&bin).output().unwrap();
    let c_fingerprint = String::from_utf8_lossy(&c_out.stdout).trim().to_string();
    assert_eq!(
        c_fingerprint, GOLDEN,
        "the v1 C reference moved — update GOLDEN (and check the Rust side!)"
    );
}

// ── 2b. Differential: list / slice / ARC-list against the C reference ─────

/// The golden fingerprint for the list/slice scenario. The Rust runtime
/// asserts the same constant in `list::tests::list_slice_scenario_is_golden`.
const LS_GOLDEN: &str = "L len=3 g0=10 g2=30 set1=99 f3=1 b4=1 pop=1 len=4 cap=1 clear=0 S slen=3 sg0=2 sg2=4 A alive=0 dropped=1";

const LS_SCENARIO: &str = r#"
#include <stdio.h>
#include <string.h>
#include <stdint.h>
extern void *lpp_arc_alloc_with_destructor(int64_t, void (*)(void *));
extern void lpp_arc_release(void *);
extern void *lpp_list_new(void);
extern void *lpp_list_new_arc(void);
extern void lpp_list_push(void *, int64_t);
extern void lpp_list_push_arc(void *, void *);
extern void lpp_list_push_float(void *, double);
extern void lpp_list_push_bool(void *, int8_t);
extern int64_t lpp_list_get(void *, int64_t);
extern double lpp_list_get_float(void *, int64_t);
extern int8_t lpp_list_get_bool(void *, int64_t);
extern void lpp_list_set(void *, int64_t, int64_t);
extern int64_t lpp_list_len(void *);
extern int64_t lpp_list_pop(void *);
extern void lpp_list_reserve(void *, int64_t);
extern int64_t lpp_list_capacity(void *);
extern void lpp_list_clear(void *);
extern void lpp_list_free(void *);
extern void *lpp_slice_init(void *, void *, int64_t, int64_t, int64_t);
extern int64_t lpp_slice_len(void *);
extern int64_t lpp_slice_get(void *, int64_t);
static int drops;
static void dtor(void *p) { (void)p; drops++; }
int main(void) {
    char out[512] = ""; char tmp[64];
    void *xs = lpp_list_new();
    lpp_list_push(xs, 10); lpp_list_push(xs, 20); lpp_list_push(xs, 30);
    snprintf(tmp, sizeof tmp, "L len=%lld g0=%lld g2=%lld ",
        (long long)lpp_list_len(xs), (long long)lpp_list_get(xs,0), (long long)lpp_list_get(xs,2));
    strcat(out, tmp);
    lpp_list_set(xs, 1, 99);
    lpp_list_push_float(xs, 3.5);
    lpp_list_push_bool(xs, 1);
    int fok = lpp_list_get_float(xs,3) == 3.5;
    int bok = lpp_list_get_bool(xs,4) != 0;
    snprintf(tmp, sizeof tmp, "set1=%lld f3=%d b4=%d ",
        (long long)lpp_list_get(xs,1), fok, bok);
    strcat(out, tmp);
    int64_t popped = lpp_list_pop(xs);
    snprintf(tmp, sizeof tmp, "pop=%lld len=%lld ", (long long)popped, (long long)lpp_list_len(xs));
    strcat(out, tmp);
    lpp_list_reserve(xs, 100);
    int cok = lpp_list_capacity(xs) >= 100;
    lpp_list_clear(xs);
    snprintf(tmp, sizeof tmp, "cap=%d clear=%lld ", cok, (long long)lpp_list_len(xs));
    strcat(out, tmp);
    lpp_list_free(xs);
    void *ys = lpp_list_new();
    for (int64_t i = 1; i <= 5; i++) lpp_list_push(ys, i);
    _Alignas(8) char storage[64];
    void *sl = lpp_slice_init(storage, ys, 1, 3, 1);
    snprintf(tmp, sizeof tmp, "S slen=%lld sg0=%lld sg2=%lld ",
        (long long)lpp_slice_len(sl), (long long)lpp_slice_get(sl,0), (long long)lpp_slice_get(sl,2));
    strcat(out, tmp);
    lpp_list_free(ys);
    void *al = lpp_list_new_arc();
    void *obj = lpp_arc_alloc_with_destructor(8, dtor);
    lpp_list_push_arc(al, obj);
    lpp_arc_release(obj);
    int alive = drops;
    lpp_list_free(al);
    snprintf(tmp, sizeof tmp, "A alive=%d dropped=%d", alive, drops);
    strcat(out, tmp);
    printf("%s\n", out);
    return 0;
}
"#;

#[test]
fn c_reference_matches_the_list_slice_golden() {
    let cc = match Command::new("cc").arg("--version").output() {
        Ok(o) if o.status.success() => "cc",
        _ => {
            eprintln!("skipping: no C compiler available");
            return;
        }
    };
    let dir = std::env::temp_dir().join(format!("lpp-runtime-ls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let scenario = dir.join("ls_scenario.c");
    std::fs::write(&scenario, LS_SCENARIO).unwrap();
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = dir.join("ls_c");
    let status = Command::new(cc)
        .args(["-O1", "-fno-stack-protector"])
        .arg(crate_root.join("lpp_runtime.c"))
        .arg(&scenario)
        .arg("-o")
        .arg(&bin)
        .arg("-lm")
        .status()
        .unwrap();
    if !status.success() {
        eprintln!("skipping: cc failed to build the C reference");
        return;
    }
    let c_out = Command::new(&bin).output().unwrap();
    let c_fingerprint = String::from_utf8_lossy(&c_out.stdout).trim().to_string();
    assert_eq!(
        c_fingerprint, LS_GOLDEN,
        "the v1 C reference list/slice behavior moved — update LS_GOLDEN (and the Rust side!)"
    );
}

// ── 2c. Differential: numeric builtins against the C reference ────────────

/// The golden FNV fingerprint of the numeric scenario. The Rust runtime
/// asserts the same constant in `numeric::tests::numeric_scenario_is_golden`.
const NUM_GOLDEN: &str = "8794162424430263925";

const NUM_SCENARIO: &str = r#"
#include <stdio.h>
#include <stdint.h>
extern int64_t lpp_abs(int64_t);
extern int64_t lpp_min(int64_t,int64_t); extern int64_t lpp_max(int64_t,int64_t);
extern int64_t lpp_shr_u(int64_t,int64_t); extern int64_t lpp_shl_u(int64_t,int64_t);
extern int64_t lpp_div_u(int64_t,int64_t); extern int64_t lpp_rem_u(int64_t,int64_t);
extern int64_t lpp_lt_u(int64_t,int64_t); extern int64_t lpp_le_u(int64_t,int64_t);
extern int64_t lpp_gt_u(int64_t,int64_t); extern int64_t lpp_ge_u(int64_t,int64_t);
extern int64_t lpp_min_u(int64_t,int64_t); extern int64_t lpp_max_u(int64_t,int64_t);
extern int64_t lpp_rotl64(int64_t,int64_t); extern int64_t lpp_rotr64(int64_t,int64_t);
extern int64_t lpp_rotl32(int64_t,int64_t); extern int64_t lpp_rotr32(int64_t,int64_t);
extern int64_t lpp_clz64(int64_t); extern int64_t lpp_ctz64(int64_t); extern int64_t lpp_popcount64(int64_t);
extern int64_t lpp_bswap16(int64_t); extern int64_t lpp_bswap32(int64_t); extern int64_t lpp_bswap64(int64_t);
extern int64_t lpp_trunc_u8(int64_t); extern int64_t lpp_trunc_u16(int64_t); extern int64_t lpp_trunc_u32(int64_t);
extern int64_t lpp_trunc_i8(int64_t); extern int64_t lpp_trunc_i16(int64_t); extern int64_t lpp_trunc_i32(int64_t);
extern int64_t lpp_add_checked(int64_t,int64_t); extern int64_t lpp_sub_checked(int64_t,int64_t); extern int64_t lpp_mul_checked(int64_t,int64_t);
extern int64_t lpp_add_wrap(int64_t,int64_t); extern int64_t lpp_sub_wrap(int64_t,int64_t); extern int64_t lpp_mul_wrap(int64_t,int64_t);
extern double lpp_sqrt(double); extern double lpp_floor(double); extern double lpp_ceil(double); extern double lpp_pow(double,double);
static long long H = 1469598103934665603LL;
static void mix(long long v){ H = (H ^ v) * 1099511628211LL; }
int main(void){
    mix(lpp_abs(-5)); mix(lpp_abs(123456789012LL));
    mix(lpp_min(3,7)); mix(lpp_max(3,7)); mix(lpp_min(-1,-2)); mix(lpp_max(-1,-2));
    mix(lpp_shr_u(-1,4)); mix(lpp_shr_u(-1,64)); mix(lpp_shr_u(255,-3));
    mix(lpp_shl_u(1,63)); mix(lpp_shl_u(1,64)); mix(lpp_shl_u(3,10));
    mix(lpp_div_u(-1,2)); mix(lpp_div_u(100,7)); mix(lpp_rem_u(7,3)); mix(lpp_rem_u(-1,16));
    mix(lpp_lt_u(-1,1)); mix(lpp_le_u(5,5)); mix(lpp_gt_u(-1,1)); mix(lpp_ge_u(2,3));
    mix(lpp_min_u(-1,1)); mix(lpp_max_u(-1,1));
    mix(lpp_rotl64(1,63)); mix(lpp_rotr64(1,1)); mix(lpp_rotl64(5,0));
    mix(lpp_rotl32(1,31)); mix(lpp_rotr32(0x80000000LL,31)); mix(lpp_rotl32(0x12345678LL,8));
    mix(lpp_clz64(0)); mix(lpp_clz64(1)); mix(lpp_ctz64(0)); mix(lpp_ctz64(8));
    mix(lpp_popcount64(-1)); mix(lpp_popcount64(11));
    mix(lpp_bswap16(0x1234)); mix(lpp_bswap32(0x12345678LL)); mix(lpp_bswap64(0x0123456789abcdefLL));
    mix(lpp_trunc_u8(0x1ff)); mix(lpp_trunc_u16(0x1ffff)); mix(lpp_trunc_u32(-1));
    mix(lpp_trunc_i8(0xff)); mix(lpp_trunc_i16(0xffff)); mix(lpp_trunc_i32(0xffffffffLL));
    mix(lpp_add_checked(2,3)); mix(lpp_sub_checked(2,3)); mix(lpp_mul_checked(-2,3));
    mix(lpp_add_wrap(9223372036854775807LL,1)); mix(lpp_sub_wrap(-9223372036854775807LL-1,1)); mix(lpp_mul_wrap(9223372036854775807LL,2));
    mix((int64_t)lpp_sqrt(9.0)); mix((int64_t)lpp_floor(-1.5)); mix((int64_t)lpp_ceil(1.2)); mix((int64_t)lpp_pow(2.0,10.0));
    printf("%lld\n", H);
    return 0;
}
"#;

#[test]
fn c_reference_matches_the_numeric_golden() {
    let cc = match Command::new("cc").arg("--version").output() {
        Ok(o) if o.status.success() => "cc",
        _ => {
            eprintln!("skipping: no C compiler available");
            return;
        }
    };
    let dir = std::env::temp_dir().join(format!("lpp-runtime-num-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let scenario = dir.join("num_scenario.c");
    std::fs::write(&scenario, NUM_SCENARIO).unwrap();
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = dir.join("num_c");
    let status = Command::new(cc)
        .args(["-O1", "-fno-stack-protector"])
        .arg(crate_root.join("lpp_runtime.c"))
        .arg(&scenario)
        .arg("-o")
        .arg(&bin)
        .arg("-lm")
        .status()
        .unwrap();
    if !status.success() {
        eprintln!("skipping: cc failed to build the C reference");
        return;
    }
    let c_out = Command::new(&bin).output().unwrap();
    let c_fingerprint = String::from_utf8_lossy(&c_out.stdout).trim().to_string();
    assert_eq!(
        c_fingerprint, NUM_GOLDEN,
        "the v1 C reference numeric behavior moved — update NUM_GOLDEN (and the Rust side!)"
    );
}

// ── 2d. Differential: string builtins against the C reference ─────────────

/// The golden FNV fingerprint of the string scenario. The Rust runtime
/// asserts the same constant in `string::tests::string_scenario_is_golden`.
const STR_GOLDEN: &str = "5679634501925218854";

const STR_SCENARIO: &str = r#"
#include <stdio.h>
#include <string.h>
#include <stdint.h>
extern char *lpp_str_concat(const char*,const char*);
extern int64_t lpp_str_find(const char*,const char*);
extern char *lpp_str_replace(const char*,const char*,const char*);
extern char *lpp_str_trim(const char*);
extern int64_t lpp_str_contains(const char*,const char*);
extern int64_t lpp_str_starts_with(const char*,const char*);
extern int64_t lpp_str_ends_with(const char*,const char*);
extern char *lpp_str_upper(const char*); extern char *lpp_str_lower(const char*);
extern char *lpp_int_to_str(int64_t); extern char *lpp_float_to_str(double); extern char *lpp_bool_to_str(int8_t);
extern int64_t lpp_str_to_int(const char*); extern int64_t lpp_str_to_u64(const char*);
extern int64_t lpp_str_eq(const char*,const char*); extern int64_t lpp_str_len(const char*);
extern char *lpp_u64_to_str(int64_t); extern char *lpp_u64_to_hex(int64_t);
extern void *lpp_arc_alloc(int64_t);
extern void *lpp_slice_init(void*,void*,int64_t,int64_t,int64_t);
extern char *lpp_str_slice_get(void*,int64_t); extern char *lpp_str_slice_to_str(void*);
static long long H = 1469598103934665603LL;
static void mix(long long v){ H = (H ^ v) * 1099511628211LL; }
static void mixs(const char *s){ if(!s){ mix(-1); return;} long long n=strlen(s); mix(n); for(long long i=0;i<n;i++) mix((long long)(unsigned char)s[i]); }
int main(void){
    mixs(lpp_str_concat("Hello, ","world"));
    mix(lpp_str_find("abcdef","cd")); mix(lpp_str_find("abcdef","zz"));
    mixs(lpp_str_replace("a-b-c","-","+"));
    mixs(lpp_str_replace("aaa","a","bb"));
    mixs(lpp_str_trim("  hi\t"));
    mix(lpp_str_contains("hello","ell")); mix(lpp_str_contains("hello","xyz"));
    mix(lpp_str_starts_with("hello","he")); mix(lpp_str_ends_with("hello","lo"));
    mixs(lpp_str_upper("aBc")); mixs(lpp_str_lower("aBc"));
    mixs(lpp_int_to_str(-42)); mixs(lpp_int_to_str(9223372036854775807LL)); mixs(lpp_int_to_str(0));
    mixs(lpp_float_to_str(3.5)); mixs(lpp_float_to_str(0.1)); mixs(lpp_float_to_str(1e20)); mixs(lpp_float_to_str(-2.25));
    mixs(lpp_bool_to_str(1)); mixs(lpp_bool_to_str(0));
    mix(lpp_str_to_int("  -123abc")); mix(lpp_str_to_int("9223372036854775807")); mix(lpp_str_to_int("xyz"));
    mix(lpp_str_to_u64("0xff")); mix(lpp_str_to_u64("1234")); mix(lpp_str_to_u64("0X1A"));
    mix(lpp_str_eq("abc","abc")); mix(lpp_str_eq("abc","abd")); mix(lpp_str_eq("",""));
    mix(lpp_str_len("hello")); mix(lpp_str_len(""));
    mixs(lpp_u64_to_str(-1)); mixs(lpp_u64_to_str(0));
    mixs(lpp_u64_to_hex(255)); mixs(lpp_u64_to_hex(0)); mixs(lpp_u64_to_hex(-1));
    char *src = (char*)lpp_arc_alloc(16); strcpy(src,"abcdefg");
    _Alignas(8) char sv[64];
    void *ss = lpp_slice_init(sv, src, 2, 3, 0);
    mixs(lpp_str_slice_to_str(ss)); mixs(lpp_str_slice_get(ss,1));
    printf("%lld\n", H);
    return 0;
}
"#;

#[test]
fn c_reference_matches_the_string_golden() {
    let cc = match Command::new("cc").arg("--version").output() {
        Ok(o) if o.status.success() => "cc",
        _ => {
            eprintln!("skipping: no C compiler available");
            return;
        }
    };
    let dir = std::env::temp_dir().join(format!("lpp-runtime-str-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let scenario = dir.join("str_scenario.c");
    std::fs::write(&scenario, STR_SCENARIO).unwrap();
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = dir.join("str_c");
    let status = Command::new(cc)
        .args(["-O1", "-fno-stack-protector"])
        .arg(crate_root.join("lpp_runtime.c"))
        .arg(&scenario)
        .arg("-o")
        .arg(&bin)
        .arg("-lm")
        .status()
        .unwrap();
    if !status.success() {
        eprintln!("skipping: cc failed to build the C reference");
        return;
    }
    let c_out = Command::new(&bin).output().unwrap();
    let c_fingerprint = String::from_utf8_lossy(&c_out.stdout).trim().to_string();
    assert_eq!(
        c_fingerprint, STR_GOLDEN,
        "the v1 C reference string behavior moved — update STR_GOLDEN (and the Rust side!)"
    );
}

// ── 2e. Differential: tasks + tuple + vec checksum against the C reference ─

/// The golden FNV fingerprint of the managed scenario. The Rust runtime
/// asserts the same constant in `task::tests::managed_scenario_is_golden`.
const MANAGED_GOLDEN: &str = "-1109191003795678529";

const MANAGED_SCENARIO: &str = r#"
#include <stdio.h>
#include <stdint.h>
extern void *lpp_arc_alloc_with_destructor(int64_t, void(*)(void*));
extern void lpp_arc_release(void*);
extern void *lpp_task_new(void*, void*, int64_t);
extern int64_t lpp_task_poll(void*);
extern int64_t lpp_task_await(void*);
extern void lpp_task_destroy(void*);
extern void *lpp_tuple_alloc(int64_t, int64_t, int64_t);
extern int64_t lpp_vec_i64_checksum(int64_t);
static long long H = 1469598103934665603LL;
static void mix(long long v){ H = (H ^ v) * 1099511628211LL; }
static int drops;
static void dtor(void *p){ (void)p; drops++; }
static int64_t code_int(void *env){ (void)env; return 42; }
static int64_t code_managed(void *env){ (void)env; void *o = lpp_arc_alloc_with_destructor(8, dtor); return (int64_t)(intptr_t)o; }
int main(void){
    void *envA = lpp_arc_alloc_with_destructor(8, dtor);
    void *tA = lpp_task_new((void*)(intptr_t)code_int, envA, 0);
    mix(lpp_task_poll(tA));
    mix(lpp_task_await(tA));
    mix(lpp_task_poll(tA));
    lpp_task_destroy(tA);
    mix(drops);
    void *envB = lpp_arc_alloc_with_destructor(8, dtor);
    void *tB = lpp_task_new((void*)(intptr_t)code_managed, envB, 1);
    int64_t r = lpp_task_await(tB);
    mix(r != 0);
    lpp_arc_release((void*)(intptr_t)r);
    lpp_task_destroy(tB);
    mix(drops);
    void *child = lpp_arc_alloc_with_destructor(8, dtor);
    void *tup = lpp_tuple_alloc(32, 1, 16);
    *(void**)((char*)tup + 16) = child;
    lpp_arc_release(tup);
    mix(drops);
    mix(lpp_vec_i64_checksum(0));
    mix(lpp_vec_i64_checksum(1));
    mix(lpp_vec_i64_checksum(10));
    mix(lpp_vec_i64_checksum(100));
    mix(lpp_vec_i64_checksum(-5));
    mix(lpp_vec_i64_checksum(1000));
    printf("%lld\n", H);
    return 0;
}
"#;

#[test]
fn c_reference_matches_the_managed_golden() {
    let cc = match Command::new("cc").arg("--version").output() {
        Ok(o) if o.status.success() => "cc",
        _ => {
            eprintln!("skipping: no C compiler available");
            return;
        }
    };
    let dir = std::env::temp_dir().join(format!("lpp-runtime-managed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let scenario = dir.join("managed_scenario.c");
    std::fs::write(&scenario, MANAGED_SCENARIO).unwrap();
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = dir.join("managed_c");
    let status = Command::new(cc)
        .args(["-O1", "-fno-stack-protector"])
        .arg(crate_root.join("lpp_runtime.c"))
        .arg(&scenario)
        .arg("-o")
        .arg(&bin)
        .arg("-lm")
        .status()
        .unwrap();
    if !status.success() {
        eprintln!("skipping: cc failed to build the C reference");
        return;
    }
    let c_out = Command::new(&bin).output().unwrap();
    let c_fingerprint = String::from_utf8_lossy(&c_out.stdout).trim().to_string();
    assert_eq!(
        c_fingerprint, MANAGED_GOLDEN,
        "the v1 C reference task/tuple/vec behavior moved — update MANAGED_GOLDEN (and the Rust side!)"
    );
}

// ── 3. Symbol census ──────────────────────────────────────────────────────

/// The exported ABI surface for 6B.1 (ARC) + 6B.2 (list/slice): every
/// symbol the slices claim to implement. Each must be exported by the Rust
/// staticlib AND defined by the v1 C reference object.
const ABI_SYMBOLS: &[&str] = &[
    // 6B.1 — ARC core
    "lpp_arc_alloc",
    "lpp_arc_alloc_with_destructor",
    "lpp_arc_release",
    "lpp_arc_release_local",
    "lpp_arc_retain",
    "lpp_arc_retain_local",
    "lpp_closure_destroy",
    "lpp_empty_str",
    "lpp_weak_generation",
    "lpp_weak_get",
    // 6B.2 — List[T]
    "lpp_list_new",
    "lpp_list_new_arc",
    "lpp_list_push",
    "lpp_list_push_arc",
    "lpp_list_push_float",
    "lpp_list_push_bool",
    "lpp_list_get",
    "lpp_list_get_float",
    "lpp_list_get_bool",
    "lpp_list_get_arc",
    "lpp_list_set",
    "lpp_list_set_bool",
    "lpp_list_set_float",
    "lpp_list_set_arc",
    "lpp_list_len",
    "lpp_list_pop",
    "lpp_list_free",
    "lpp_list_reserve",
    "lpp_list_capacity",
    "lpp_list_clear",
    // 6B.2 — Slice[T]
    "lpp_slice_init",
    "lpp_slice_len",
    "lpp_slice_get",
    "lpp_slice_get_float",
    "lpp_slice_get_bool",
    // 6B.3a — numeric builtins
    "lpp_abs",
    "lpp_min",
    "lpp_max",
    "lpp_sqrt",
    "lpp_floor",
    "lpp_ceil",
    "lpp_pow",
    "lpp_shr_u",
    "lpp_shl_u",
    "lpp_div_u",
    "lpp_rem_u",
    "lpp_lt_u",
    "lpp_le_u",
    "lpp_gt_u",
    "lpp_ge_u",
    "lpp_min_u",
    "lpp_max_u",
    "lpp_rotl64",
    "lpp_rotr64",
    "lpp_rotl32",
    "lpp_rotr32",
    "lpp_clz64",
    "lpp_ctz64",
    "lpp_popcount64",
    "lpp_bswap16",
    "lpp_bswap32",
    "lpp_bswap64",
    "lpp_trunc_u8",
    "lpp_trunc_u16",
    "lpp_trunc_u32",
    "lpp_trunc_i8",
    "lpp_trunc_i16",
    "lpp_trunc_i32",
    "lpp_add_checked",
    "lpp_sub_checked",
    "lpp_mul_checked",
    "lpp_add_wrap",
    "lpp_sub_wrap",
    "lpp_mul_wrap",
    // 6B.3b — string builtins
    "lpp_str_concat",
    "lpp_str_find",
    "lpp_str_replace",
    "lpp_str_trim",
    "lpp_str_contains",
    "lpp_str_starts_with",
    "lpp_str_ends_with",
    "lpp_str_upper",
    "lpp_str_lower",
    "lpp_str_eq",
    "lpp_str_len",
    "lpp_int_to_str",
    "lpp_float_to_str",
    "lpp_bool_to_str",
    "lpp_u64_to_str",
    "lpp_u64_to_hex",
    "lpp_str_to_int",
    "lpp_str_to_u64",
    "lpp_str_slice_get",
    "lpp_str_slice_to_str",
    // 6B.3c — IO builtins
    "lpp_print_int",
    "lpp_print_float",
    "lpp_print_bool",
    "lpp_print_str",
    "lpp_write_str",
    "lpp_eprint_str",
    // 6B.3d — tasks, tuple, vec checksum
    "lpp_task_new",
    "lpp_task_poll",
    "lpp_task_await",
    "lpp_task_destroy",
    "lpp_tuple_alloc",
    "lpp_vec_i64_checksum",
];

/// Symbols defined by the `c_shim` reference but NOT by `lpp_runtime.c`.
/// The census checks these against `c_shim.c` instead of the v1 runtime.
const SHIM_ONLY_SYMBOLS: &[&str] = &["lpp_eprint_str"];

#[test]
fn symbol_census_matches_the_c_reference() {
    let target_a =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/liblpp_runtime.a");
    if !target_a.exists() {
        eprintln!("skipping: build the crate first (cargo build -p lpp-runtime)");
        return;
    }
    let nm = Command::new("nm")
        .args(["-g", target_a.to_str().unwrap()])
        .output()
        .expect("nm must exist");
    let mut rust_syms = std::collections::BTreeSet::new();
    for line in String::from_utf8_lossy(&nm.stdout).lines() {
        let mut parts = line.split_whitespace();
        let _ = parts.next(); // value
        let kind = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("");
        if ["T", "t", "B", "D", "R", "r"].contains(&kind) && name.starts_with("lpp_") {
            rust_syms.insert(name.to_string());
        }
    }
    for sym in ABI_SYMBOLS {
        assert!(
            rust_syms.contains(*sym),
            "Rust staticlib is missing v1 symbol {sym} (has: {rust_syms:?})"
        );
    }

    // Every 6B.1 symbol must also be defined by the C reference object.
    let cc = match Command::new("cc").arg("--version").output() {
        Ok(o) if o.status.success() => "cc",
        _ => {
            eprintln!("skipping C-reference half of the census: no C compiler");
            return;
        }
    };
    let dir = std::env::temp_dir().join(format!("lpp-runtime-census-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let c_obj = dir.join("lpp_runtime_ref.o");
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = Command::new(cc)
        .args(["-O2", "-c"])
        .arg(crate_root.join("lpp_runtime.c"))
        .arg("-o")
        .arg(&c_obj)
        .status()
        .unwrap();
    assert!(status.success(), "the v1 C runtime must still compile");
    let nm = Command::new("nm")
        .args(["-g", "--defined-only", c_obj.to_str().unwrap()])
        .output()
        .expect("nm must exist");
    let mut c_syms = std::collections::BTreeSet::new();
    for line in String::from_utf8_lossy(&nm.stdout).lines() {
        let mut parts = line.split_whitespace();
        let _ = parts.next();
        let _ = parts.next();
        if let Some(name) = parts.next() {
            c_syms.insert(name.to_string());
        }
    }
    for sym in ABI_SYMBOLS {
        if SHIM_ONLY_SYMBOLS.contains(sym) {
            continue; // checked against c_shim.c below
        }
        assert!(
            c_syms.contains(*sym),
            "v1 C reference does not define {sym} — the census table is wrong"
        );
    }

    // Shim-only symbols (e.g. eprint_str) are defined by c_shim.c, not by
    // lpp_runtime.c — verify them against the shim object.
    let shim_obj = dir.join("c_shim_ref.o");
    let shim_src = crate_root.join("crates/lpp-codegen-cranelift/tests/c_shim.c");
    let status = Command::new(cc)
        .args(["-O2", "-c"])
        .arg(&shim_src)
        .arg("-o")
        .arg(&shim_obj)
        .status()
        .unwrap();
    assert!(status.success(), "the c_shim reference must still compile");
    let nm = Command::new("nm")
        .args(["-g", "--defined-only", shim_obj.to_str().unwrap()])
        .output()
        .expect("nm must exist");
    let mut shim_syms = std::collections::BTreeSet::new();
    for line in String::from_utf8_lossy(&nm.stdout).lines() {
        let mut parts = line.split_whitespace();
        let _ = parts.next();
        let _ = parts.next();
        if let Some(name) = parts.next() {
            shim_syms.insert(name.to_string());
        }
    }
    for sym in SHIM_ONLY_SYMBOLS {
        assert!(
            shim_syms.contains(*sym),
            "c_shim reference does not define shim-only symbol {sym}"
        );
    }
}
