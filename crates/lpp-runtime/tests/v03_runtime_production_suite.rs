//! Production integration test suite for `lpp-runtime` (v0.3).
//!
//! Directly exercises the C-ABI runtime surface across:
//! 1. ARC memory header layout, retain/release, local fast-paths, weak handles, closures
//! 2. Bacon-Rajan cycle collection (`lpp_arc_mark_collectible`, `lpp_trace_visit`,
//!    `lpp_arc_collect_cycles`, `lpp__cycle_collected_count`, tuple & list cycles)
//! 3. Strings (`lpp_str_*`, UTF-8 char helpers, numeric parsing/formatting) & zero-copy slices
//! 4. Dynamic lists (`lpp_list_*` int/float/bool/ARC, insert/remove/sort/search/extend)
//! 5. Open-addressing hash maps (`lpp_map_*` int & str keys, tombstones, rehash, float/ARC)
//! 6. Hash sets (`lpp_set_*`) and ring-buffer deques (`lpp_deque_*`)
//! 7. Byte buffers (`lpp_buf_*`, endianness, overlap copy, CRC32, file I/O) & JSON (`lpp_json_*`)
//! 8. Numeric intrinsics (`lpp_vec_i64_checksum`, unsigned/bit ops), clocks/RNG/C-mem, and
//!    subprocess `runtime_panic` / `lpp_panic_div_zero` / checked overflow / `LPP_BACKTRACE` gating

use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use libc::{c_char, c_void};

use lpp_runtime::arc::{
    lpp__runtime_drop_count, lpp_arc_alloc, lpp_arc_alloc_with_destructor, lpp_arc_release,
    lpp_arc_release_local, lpp_arc_retain, lpp_arc_retain_local, lpp_closure_destroy,
    lpp_closure_trace, lpp_empty_str, lpp_weak_generation, lpp_weak_get,
};
use lpp_runtime::buf::{
    lpp_buf_alloc, lpp_buf_copy, lpp_buf_crc32, lpp_buf_free, lpp_buf_free_at_exit, lpp_buf_get8,
    lpp_buf_get16be, lpp_buf_get16le, lpp_buf_get32be, lpp_buf_get32le, lpp_buf_get64be,
    lpp_buf_get64le, lpp_buf_len, lpp_buf_read, lpp_buf_read_str, lpp_buf_set8, lpp_buf_set16be,
    lpp_buf_set16le, lpp_buf_set32be, lpp_buf_set32le, lpp_buf_set64be, lpp_buf_set64le,
    lpp_buf_to_str, lpp_buf_write, lpp_buf_write_str,
};
use lpp_runtime::clock_rng::{
    lpp_c_free, lpp_c_load_i32, lpp_c_load_i64, lpp_c_load_u8, lpp_c_malloc, lpp_c_store_i32,
    lpp_c_store_i64, lpp_c_store_u8, lpp_clock_advance, lpp_clock_free, lpp_clock_new,
    lpp_clock_now, lpp_random, lpp_random_range, lpp_random_seed, lpp_rng_float, lpp_rng_free,
    lpp_rng_new, lpp_rng_next, lpp_rng_range, lpp_time_ms, lpp_time_ns,
};
use lpp_runtime::collect::{
    lpp__cycle_collected_count, lpp_arc_collect_cycles, lpp_arc_mark_collectible, lpp_trace_visit,
};
use lpp_runtime::collections_ext::{
    lpp_deque_clear, lpp_deque_free, lpp_deque_get, lpp_deque_len, lpp_deque_new,
    lpp_deque_peek_back, lpp_deque_peek_front, lpp_deque_pop_back, lpp_deque_pop_front,
    lpp_deque_push_back, lpp_deque_push_front, lpp_set_add, lpp_set_add_str, lpp_set_clear,
    lpp_set_free, lpp_set_has, lpp_set_has_str, lpp_set_intersect, lpp_set_len, lpp_set_new,
    lpp_set_remove, lpp_set_remove_str, lpp_set_union,
};
use lpp_runtime::json::{
    lpp_json_escape, lpp_json_free, lpp_json_get_int, lpp_json_get_obj, lpp_json_get_str,
    lpp_json_parse,
};
use lpp_runtime::layout::{
    ARC_CYCLE_FLAGS_OFFSET, ARC_DESTRUCTOR_PREFIX_SIZE, ARC_FLAG_COLLECTIBLE, ARC_FLAG_HAS_DTOR,
    ARC_GENERATION_OFFSET, ARC_HEADER_SIZE, ARC_IMMORTAL, ARC_MAGIC, ARC_MAGIC_OFFSET,
    ARC_REFCOUNT_OFFSET, MIN_VALID_ADDR,
};
use lpp_runtime::list::{
    lpp_list_binary_search, lpp_list_capacity, lpp_list_clear, lpp_list_extend, lpp_list_free,
    lpp_list_get, lpp_list_get_arc, lpp_list_get_bool, lpp_list_get_float, lpp_list_index_of,
    lpp_list_insert, lpp_list_len, lpp_list_new, lpp_list_new_arc, lpp_list_pop, lpp_list_push,
    lpp_list_push_arc, lpp_list_push_bool, lpp_list_push_float, lpp_list_remove, lpp_list_reserve,
    lpp_list_reverse, lpp_list_set, lpp_list_set_arc, lpp_list_set_bool, lpp_list_set_float,
    lpp_list_sort, lpp_list_sort_desc, lpp_list_sort_u, lpp_list_swap, lpp_list_trace,
    lpp_list_truncate,
};
use lpp_runtime::map::{
    lpp_map_capacity, lpp_map_clear, lpp_map_get, lpp_map_get_float, lpp_map_get_str,
    lpp_map_get_str_float, lpp_map_has, lpp_map_has_str, lpp_map_keys, lpp_map_len, lpp_map_new,
    lpp_map_new_arc, lpp_map_put, lpp_map_put_float, lpp_map_put_str, lpp_map_put_str_float,
    lpp_map_remove, lpp_map_remove_str, lpp_map_values,
};
use lpp_runtime::numeric::{
    lpp_abs, lpp_add_checked, lpp_add_overflows, lpp_add_wrap, lpp_bswap16, lpp_bswap32,
    lpp_bswap64, lpp_ceil, lpp_clz64, lpp_cos, lpp_ctz64, lpp_div_u, lpp_float_to_int, lpp_floor,
    lpp_ge_u, lpp_gt_u, lpp_int_pow, lpp_int_to_float, lpp_le_u, lpp_lt_u, lpp_max, lpp_max_u,
    lpp_min, lpp_min_u, lpp_mul_checked, lpp_mul_overflows, lpp_mul_wrap, lpp_popcount64, lpp_pow,
    lpp_rem_u, lpp_rotl32, lpp_rotl64, lpp_rotr32, lpp_rotr64, lpp_shl_u, lpp_shr_u, lpp_sin,
    lpp_sqrt, lpp_sub_checked, lpp_sub_wrap, lpp_tan, lpp_trunc_i8, lpp_trunc_i16, lpp_trunc_i32,
    lpp_trunc_u8, lpp_trunc_u16, lpp_trunc_u32, lpp_vec_i64_checksum,
};
use lpp_runtime::panic::{lpp_panic_div_zero, runtime_panic};
use lpp_runtime::slice::{
    lpp_slice_get, lpp_slice_get_bool, lpp_slice_get_float, lpp_slice_init, lpp_slice_len,
    lpp_str_slice_get, lpp_str_slice_to_str,
};
use lpp_runtime::string::{
    lpp_bool_to_str, lpp_char_at, lpp_chr, lpp_float_to_str, lpp_int_to_str, lpp_ord,
    lpp_parse_int, lpp_str_cmp, lpp_str_concat, lpp_str_contains, lpp_str_ends_with, lpp_str_eq,
    lpp_str_find, lpp_str_join, lpp_str_len, lpp_str_lines, lpp_str_lower, lpp_str_pad_left,
    lpp_str_pad_right, lpp_str_repeat, lpp_str_replace, lpp_str_split, lpp_str_starts_with,
    lpp_str_substr, lpp_str_to_int, lpp_str_to_u64, lpp_str_trim, lpp_str_upper, lpp_u64_to_hex,
    lpp_u64_to_str,
};
use lpp_runtime::tuple::{lpp_tuple_alloc, lpp_tuple_trace};

static COLLECT_SERIAL: Mutex<()> = Mutex::new(());
static DROP_SERIAL: Mutex<()> = Mutex::new(());
static RNG_SERIAL: Mutex<()> = Mutex::new(());

fn cstr(s: &str) -> CString {
    CString::new(s).expect("CString without interior NUL")
}

unsafe fn take_str(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null(), "runtime returned null string pointer");
    let out = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("valid UTF-8 C string")
        .to_owned();
    unsafe { lpp_arc_release(ptr.cast()) };
    out
}

unsafe fn read_borrowed_str(ptr: *const c_char) -> String {
    assert!(!ptr.is_null(), "null borrowed C string");
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("valid UTF-8 borrowed C string")
        .to_owned()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("lpp_v03_rt_{tag}_{}_{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. ARC Memory Header, Lifecycle, Weak Handles, Immortal & Closures
// ═══════════════════════════════════════════════════════════════════════════

static ARC_DTOR_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn test_arc_dtor(payload: *mut c_void) {
    assert!(!payload.is_null());
    // Payload tag written before release must still be intact inside dtor
    let tag = unsafe { *(payload as *const u64) };
    assert_eq!(tag, 0xCAFE_BABE_1234_5678);
    ARC_DTOR_CALLS.fetch_add(1, Ordering::Relaxed);
}

#[test]
fn arc_header_layout_refcount_weak_handles_and_guards() {
    let _guard = DROP_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        // Guarded no-ops for null and below MIN_VALID_ADDR
        lpp_arc_retain(std::ptr::null_mut());
        lpp_arc_release(std::ptr::null_mut());
        lpp_arc_retain_local(0x1000usize as *mut c_void);
        lpp_arc_release_local(0x1000usize as *mut c_void);
        assert_eq!(lpp_weak_generation(std::ptr::null_mut()), 0);
        assert_eq!(lpp_weak_generation((MIN_VALID_ADDR - 1) as *mut c_void), 0);
        assert_eq!(lpp_weak_get(0, 1), 0);
        assert_eq!(lpp_weak_get(0x20000, 0), 0);

        // Plain ARC allocation (16-byte header, no destructor prefix)
        let p = lpp_arc_alloc(32);
        assert!(!p.is_null());
        let hdr = (p as *const u8).sub(ARC_HEADER_SIZE);
        assert_eq!(
            *(hdr.add(ARC_MAGIC_OFFSET) as *const u32),
            ARC_MAGIC,
            "ARC magic matches header constant"
        );
        assert_eq!(*(hdr.add(ARC_REFCOUNT_OFFSET) as *const u32), 1);
        let gen0 = lpp_weak_generation(p);
        assert!(gen0 >= 1);
        assert_eq!(*(hdr.add(ARC_GENERATION_OFFSET) as *const u32) as i64, gen0);
        assert_eq!(*(hdr.add(ARC_CYCLE_FLAGS_OFFSET) as *const u32), 0);

        // Weak handle resolves while live
        assert_eq!(lpp_weak_get(p as usize as i64, gen0), p as usize as i64);
        assert_eq!(lpp_weak_get(p as usize as i64, gen0 + 99), 0);

        // Local and atomic retain/release balance
        lpp_arc_retain(p);
        lpp_arc_retain_local(p);
        assert_eq!(*(hdr.add(ARC_REFCOUNT_OFFSET) as *const u32), 3);
        lpp_arc_release_local(p);
        assert_eq!(*(hdr.add(ARC_REFCOUNT_OFFSET) as *const u32), 2);
        lpp_arc_release(p);
        assert_eq!(*(hdr.add(ARC_REFCOUNT_OFFSET) as *const u32), 1);

        let drops_before = lpp__runtime_drop_count.load(Ordering::Relaxed);
        lpp_arc_release(p);
        let drops_after = lpp__runtime_drop_count.load(Ordering::Relaxed);
        assert!(
            drops_after > drops_before,
            "drop count increments on final release"
        );

        // Destructor-prefixed allocation verifies 8-byte dtor prefix + invocation count
        let d_before = ARC_DTOR_CALLS.load(Ordering::Relaxed);
        let dp = lpp_arc_alloc_with_destructor(16, Some(test_arc_dtor));
        assert!(!dp.is_null());
        let dhdr = (dp as *const u8).sub(ARC_HEADER_SIZE);
        assert_eq!(
            *(dhdr.add(ARC_CYCLE_FLAGS_OFFSET) as *const u32) & ARC_FLAG_HAS_DTOR,
            ARC_FLAG_HAS_DTOR
        );
        // Preceding 8 bytes before header hold the destructor function pointer
        let dtor_slot = *(dhdr.sub(ARC_DESTRUCTOR_PREFIX_SIZE) as *const usize);
        assert_eq!(dtor_slot, test_arc_dtor as *const () as usize);

        *(dp as *mut u64) = 0xCAFE_BABE_1234_5678;
        let dp_gen = lpp_weak_generation(dp);
        lpp_arc_retain(dp);
        lpp_arc_release(dp);
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before);
        assert_eq!(lpp_weak_get(dp as usize as i64, dp_gen), dp as usize as i64);

        lpp_arc_release(dp);
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before + 1);
    }
}

#[test]
fn arc_immortal_empty_str_and_closure_destroy() {
    let _guard = DROP_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let empty = lpp_empty_str();
        assert!(!empty.is_null());
        assert_eq!(*(empty as *const u8), 0);
        assert_eq!(lpp_weak_generation(empty), i64::from(ARC_IMMORTAL));
        assert_eq!(
            lpp_weak_get(empty as usize as i64, i64::from(ARC_IMMORTAL)),
            empty as usize as i64
        );
        assert_eq!(lpp_weak_get(empty as usize as i64, 1), 0);
        let drops_before = lpp__runtime_drop_count.load(Ordering::Relaxed);
        for _ in 0..16 {
            lpp_arc_retain(empty);
            lpp_arc_release(empty);
            lpp_arc_release_local(empty);
        }
        assert_eq!(
            lpp__runtime_drop_count.load(Ordering::Relaxed),
            drops_before
        );

        // Closure payload: [code_ptr, env_ptr]; lpp_closure_destroy releases env_ptr
        let d_before = ARC_DTOR_CALLS.load(Ordering::Relaxed);
        let env = lpp_arc_alloc_with_destructor(8, Some(test_arc_dtor));
        *(env as *mut u64) = 0xCAFE_BABE_1234_5678;

        let closure = lpp_arc_alloc_with_destructor(
            2 * std::mem::size_of::<usize>() as i64,
            Some(lpp_closure_destroy),
        );
        let parts = closure as *mut *mut c_void;
        *parts = 0x1234usize as *mut c_void; // fake code ptr (non-owning)
        *parts.add(1) = env; // owned env ref (refcount = 1)

        lpp_arc_release(closure);
        assert_eq!(
            ARC_DTOR_CALLS.load(Ordering::Relaxed),
            d_before + 1,
            "closure destructor must release owned environment"
        );
        // Null guard
        lpp_closure_destroy(std::ptr::null_mut());
        lpp_closure_trace(std::ptr::null_mut(), std::ptr::null_mut());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Bacon-Rajan Cycle Collection & Tuple/List/Closure Cycles
// ═══════════════════════════════════════════════════════════════════════════

static NODE_DROPS: AtomicUsize = AtomicUsize::new(0);
static LAST_LIVE_SUM: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
struct CycleNode {
    child1: *mut c_void,
    child2: *mut c_void,
    tag: u64,
}

unsafe extern "C" fn cycle_node_dtor(payload: *mut c_void) {
    let n = unsafe { &mut *(payload as *mut CycleNode) };
    NODE_DROPS.fetch_add(1, Ordering::Relaxed);
    LAST_LIVE_SUM.fetch_add(n.tag as usize, Ordering::Relaxed);
    unsafe {
        if !n.child1.is_null() {
            lpp_arc_release(n.child1);
            n.child1 = std::ptr::null_mut();
        }
        if !n.child2.is_null() {
            lpp_arc_release(n.child2);
            n.child2 = std::ptr::null_mut();
        }
    }
}

unsafe extern "C" fn cycle_node_trace(payload: *mut c_void, ctx: *mut c_void) {
    let n = unsafe { &*(payload as *const CycleNode) };
    unsafe {
        if !n.child1.is_null() {
            lpp_trace_visit(ctx, n.child1);
        }
        if !n.child2.is_null() {
            lpp_trace_visit(ctx, n.child2);
        }
    }
}

unsafe fn alloc_cycle_node(tag: u64) -> *mut c_void {
    let p = unsafe {
        lpp_arc_alloc_with_destructor(
            std::mem::size_of::<CycleNode>() as i64,
            Some(cycle_node_dtor),
        )
    };
    let n = p as *mut CycleNode;
    unsafe {
        (*n).child1 = std::ptr::null_mut();
        (*n).child2 = std::ptr::null_mut();
        (*n).tag = tag;
        lpp_arc_mark_collectible(p, Some(cycle_node_trace));
    }
    p
}

#[test]
fn bacon_rajan_collects_self_two_and_three_node_cycles_without_double_free() {
    let _serial = COLLECT_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let drops0 = NODE_DROPS.load(Ordering::Relaxed);
        let collected0 = lpp__cycle_collected_count.load(Ordering::Relaxed);

        // 1-node self-cycle: s -> s
        let s = alloc_cycle_node(1);
        let hdr_s = (s as *const u8).sub(ARC_HEADER_SIZE);
        assert_ne!(
            *(hdr_s.add(ARC_CYCLE_FLAGS_OFFSET) as *const u32) & ARC_FLAG_COLLECTIBLE,
            0
        );
        (*(s as *mut CycleNode)).child1 = s;
        lpp_arc_retain(s); // refcount = 2 (stack + self)
        lpp_arc_release(s); // refcount = 1, buffered in root buffer

        // 2-node mutual cycle: a <-> b
        let a = alloc_cycle_node(10);
        let b = alloc_cycle_node(20);
        (*(a as *mut CycleNode)).child1 = b;
        lpp_arc_retain(b);
        (*(b as *mut CycleNode)).child1 = a;
        lpp_arc_retain(a);
        lpp_arc_release(a);
        lpp_arc_release(b);

        // 3-node ring: c1 -> c2 -> c3 -> c1
        let c1 = alloc_cycle_node(100);
        let c2 = alloc_cycle_node(200);
        let c3 = alloc_cycle_node(300);
        (*(c1 as *mut CycleNode)).child1 = c2;
        lpp_arc_retain(c2);
        (*(c2 as *mut CycleNode)).child1 = c3;
        lpp_arc_retain(c3);
        (*(c3 as *mut CycleNode)).child1 = c1;
        lpp_arc_retain(c1);
        lpp_arc_release(c1);
        lpp_arc_release(c2);
        lpp_arc_release(c3);

        lpp_arc_collect_cycles();

        assert_eq!(
            NODE_DROPS.load(Ordering::Relaxed) - drops0,
            6,
            "all 6 cyclic nodes (1+2+3) must be collected exactly once"
        );
        assert!(
            lpp__cycle_collected_count.load(Ordering::Relaxed) >= collected0 + 6,
            "cycle collected counter must advance by at least 6"
        );
    }
}

#[test]
fn bacon_rajan_preserves_externally_rooted_cycle_and_acyclic_dag() {
    let _serial = COLLECT_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        // Ensure clean root buffer before measurement
        lpp_arc_collect_cycles();
        let drops0 = NODE_DROPS.load(Ordering::Relaxed);

        // Cycle x <-> y where caller keeps external root on x
        let x = alloc_cycle_node(1);
        let y = alloc_cycle_node(2);
        (*(x as *mut CycleNode)).child1 = y;
        lpp_arc_retain(y);
        (*(y as *mut CycleNode)).child1 = x;
        lpp_arc_retain(x);
        lpp_arc_release(y); // y now only referenced by x (and buffered); x still has refcount 2

        // Acyclic diamond: root -> {left, right} -> shared_leaf
        let leaf = alloc_cycle_node(3);
        let left = alloc_cycle_node(4);
        let right = alloc_cycle_node(5);
        (*(left as *mut CycleNode)).child1 = leaf;
        lpp_arc_retain(leaf);
        (*(right as *mut CycleNode)).child1 = leaf;
        lpp_arc_retain(leaf);
        let root = alloc_cycle_node(6);
        (*(root as *mut CycleNode)).child1 = left;
        lpp_arc_retain(left);
        (*(root as *mut CycleNode)).child2 = right;
        lpp_arc_retain(right);
        lpp_arc_release(leaf);
        lpp_arc_release(left);
        lpp_arc_release(right);

        lpp_arc_collect_cycles();
        assert_eq!(
            NODE_DROPS.load(Ordering::Relaxed) - drops0,
            0,
            "neither externally-rooted cycle nor live diamond may be reclaimed"
        );

        // Dropping external root on `x` orphans the cycle; collecting now reclaims x and y
        lpp_arc_release(x);
        lpp_arc_collect_cycles();
        assert_eq!(NODE_DROPS.load(Ordering::Relaxed) - drops0, 2);

        // Dropping `root` deterministically drops the whole acyclic diamond without cycle collector
        lpp_arc_release(root);
        assert_eq!(NODE_DROPS.load(Ordering::Relaxed) - drops0, 6);
    }
}

#[test]
fn bacon_rajan_root_buffer_threshold_tuple_list_and_closure_cycles() {
    let _serial = COLLECT_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        lpp_arc_collect_cycles();
        let drops0 = NODE_DROPS.load(Ordering::Relaxed);

        // 1. Cycle through LppList (`lpp_list_new_arc` + `lpp_list_trace`): list -> node -> list
        let list = lpp_list_new_arc();
        lpp_arc_mark_collectible(list, Some(lpp_list_trace));
        let host = alloc_cycle_node(77);
        lpp_list_push_arc(list, host); // list owns ref to host
        (*(host as *mut CycleNode)).child1 = list;
        lpp_arc_retain(list); // host owns ref to list
        lpp_arc_release(host);
        lpp_arc_release(list);

        // 2. Cycle through LppTuple (`lpp_tuple_alloc` + `lpp_tuple_trace`): tuple(slot0=host2) -> tuple
        let tuple_ptr = lpp_tuple_alloc(16, 0b0001, 0);
        lpp_arc_mark_collectible(tuple_ptr, Some(lpp_tuple_trace));
        let host2 = alloc_cycle_node(88);
        *(tuple_ptr as *mut *mut c_void) = host2;
        lpp_arc_retain(host2);
        (*(host2 as *mut CycleNode)).child1 = tuple_ptr;
        lpp_arc_retain(tuple_ptr);
        lpp_arc_release(host2);
        lpp_arc_release(tuple_ptr);

        // 3. Cycle through Closure capsule (`lpp_closure_trace`): closure -> env(CycleNode) -> closure
        let env = alloc_cycle_node(99);
        let closure = lpp_arc_alloc_with_destructor(
            2 * std::mem::size_of::<usize>() as i64,
            Some(lpp_closure_destroy),
        );
        lpp_arc_mark_collectible(closure, Some(lpp_closure_trace));
        let cparts = closure as *mut *mut c_void;
        *cparts = 0xDEADBEEFusize as *mut c_void;
        *cparts.add(1) = env; // refcount already 1 from alloc_cycle_node
        (*(env as *mut CycleNode)).child1 = closure;
        lpp_arc_retain(closure);
        lpp_arc_release(closure);

        lpp_arc_collect_cycles();
        assert_eq!(
            NODE_DROPS.load(Ordering::Relaxed) - drops0,
            3,
            "list, tuple, and closure cycle hosts must all be collected"
        );

        // 4. Exceed COLLECT_THRESHOLD (512) buffered roots with 300 2-node cycles to trigger
        // automatic collection inside release path
        let auto_drops0 = NODE_DROPS.load(Ordering::Relaxed);
        for k in 0..300u64 {
            let p = alloc_cycle_node(k);
            let q = alloc_cycle_node(k + 1000);
            (*(p as *mut CycleNode)).child1 = q;
            lpp_arc_retain(q);
            (*(q as *mut CycleNode)).child1 = p;
            lpp_arc_retain(p);
            lpp_arc_release(p);
            lpp_arc_release(q);
        }
        // Threshold 512 triggered automatically during the 600 releases; flush remainder too
        let after_auto = NODE_DROPS.load(Ordering::Relaxed) - auto_drops0;
        assert!(
            after_auto > 0,
            "auto-collection should fire once root buffer reaches threshold"
        );
        lpp_arc_collect_cycles();
        assert_eq!(NODE_DROPS.load(Ordering::Relaxed) - auto_drops0, 600);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Strings & Zero-Copy Slices
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn strings_concat_repeat_split_find_replace_trim_case_and_padding() {
    unsafe {
        let a = cstr("Hello, ");
        let b = cstr("L++!");
        let joined = take_str(lpp_str_concat(a.as_ptr(), b.as_ptr()));
        assert_eq!(joined, "Hello, L++!");
        assert_eq!(
            take_str(lpp_str_concat(std::ptr::null(), b.as_ptr())),
            "L++!"
        );
        assert_eq!(
            take_str(lpp_str_concat(a.as_ptr(), std::ptr::null())),
            "Hello, "
        );

        let r = cstr("ab");
        assert_eq!(take_str(lpp_str_repeat(r.as_ptr(), 0)), "");
        assert_eq!(take_str(lpp_str_repeat(r.as_ptr(), -5)), "");
        assert_eq!(take_str(lpp_str_repeat(r.as_ptr(), 4)), "abababab");

        // Split on comma including leading, consecutive, and trailing delimiters
        let csv = cstr(",alpha,,beta,gamma,");
        let list = lpp_str_split(csv.as_ptr(), i64::from(b','));
        assert_eq!(lpp_list_len(list), 6);
        let expected = ["", "alpha", "", "beta", "gamma", ""];
        for (i, exp) in expected.iter().enumerate() {
            let elem = lpp_list_get_arc(list, i as i64) as *const c_char;
            assert_eq!(read_borrowed_str(elem), *exp);
        }
        let sep = cstr("|");
        assert_eq!(
            take_str(lpp_str_join(list, sep.as_ptr())),
            "|alpha||beta|gamma|"
        );
        lpp_arc_release(list);

        // Find & contains & starts_with & ends_with
        let hay = cstr("the quick brown fox jumps over the lazy dog");
        let fox = cstr("fox");
        let cat = cstr("cat");
        let prefix = cstr("the quick");
        let suffix = cstr("lazy dog");
        let empty = cstr("");
        assert_eq!(lpp_str_find(hay.as_ptr(), fox.as_ptr()), 16);
        assert_eq!(lpp_str_find(hay.as_ptr(), cat.as_ptr()), -1);
        assert_eq!(lpp_str_contains(hay.as_ptr(), fox.as_ptr()), 1);
        assert_eq!(lpp_str_contains(hay.as_ptr(), cat.as_ptr()), 0);
        assert_eq!(lpp_str_starts_with(hay.as_ptr(), prefix.as_ptr()), 1);
        assert_eq!(lpp_str_starts_with(hay.as_ptr(), suffix.as_ptr()), 0);
        assert_eq!(lpp_str_ends_with(hay.as_ptr(), suffix.as_ptr()), 1);
        assert_eq!(lpp_str_ends_with(hay.as_ptr(), empty.as_ptr()), 1);

        // Replace: multiple, non-matching, empty pattern
        let rep_src = cstr("foo bar foo baz foo");
        let pat = cstr("foo");
        let sub = cstr("quux");
        assert_eq!(
            take_str(lpp_str_replace(
                rep_src.as_ptr(),
                pat.as_ptr(),
                sub.as_ptr()
            )),
            "quux bar quux baz quux"
        );
        assert_eq!(
            take_str(lpp_str_replace(
                rep_src.as_ptr(),
                empty.as_ptr(),
                sub.as_ptr()
            )),
            "foo bar foo baz foo"
        );

        // Trim, upper, lower, pad_left, pad_right
        let ws = cstr(" \t\r\n  MiXeD Case 123 \n\t ");
        let trimmed = cstr(&take_str(lpp_str_trim(ws.as_ptr())));
        assert_eq!(read_borrowed_str(trimmed.as_ptr()), "MiXeD Case 123");
        assert_eq!(take_str(lpp_str_upper(trimmed.as_ptr())), "MIXED CASE 123");
        assert_eq!(take_str(lpp_str_lower(trimmed.as_ptr())), "mixed case 123");

        let num_s = cstr("42");
        let zero_p = cstr("0");
        assert_eq!(
            take_str(lpp_str_pad_left(num_s.as_ptr(), 6, zero_p.as_ptr())),
            "000042"
        );
        assert_eq!(
            take_str(lpp_str_pad_right(num_s.as_ptr(), 5, zero_p.as_ptr())),
            "42000"
        );
        assert_eq!(
            take_str(lpp_str_pad_left(num_s.as_ptr(), 1, zero_p.as_ptr())),
            "42"
        );
    }
}

#[test]
fn strings_utf8_chars_lines_substr_eq_cmp_and_conversions() {
    unsafe {
        // char_at / chr / ord / len
        let s = cstr("A\u{03BB}\u{1F680}Z");
        assert_eq!(lpp_str_len(s.as_ptr()), 8);
        let ascii = cstr("ABCD");
        assert_eq!(take_str(lpp_char_at(ascii.as_ptr(), 0)), "A");
        assert_eq!(take_str(lpp_char_at(ascii.as_ptr(), 1)), "B");
        assert_eq!(take_str(lpp_char_at(ascii.as_ptr(), 2)), "C");
        assert_eq!(take_str(lpp_char_at(ascii.as_ptr(), 3)), "D");
        assert_eq!(take_str(lpp_char_at(ascii.as_ptr(), 99)), "");
        assert_eq!(take_str(lpp_char_at(ascii.as_ptr(), -1)), "");

        let rocket = take_str(lpp_chr(0x1F680));
        assert_eq!(rocket, "\u{1F680}");
        let rocket_c = cstr(&rocket);
        assert_eq!(lpp_ord(rocket_c.as_ptr()), 0x1F680);
        assert_eq!(lpp_ord(std::ptr::null()), 0);
        assert_eq!(take_str(lpp_chr(-1)), "");
        assert_eq!(take_str(lpp_chr(0xD800)), ""); // invalid surrogate

        // substr bounds & clamping
        let sub_s = cstr("abcdef");
        assert_eq!(take_str(lpp_str_substr(sub_s.as_ptr(), 2, 3)), "cde");
        assert_eq!(take_str(lpp_str_substr(sub_s.as_ptr(), 4, 50)), "ef");
        assert_eq!(take_str(lpp_str_substr(sub_s.as_ptr(), -2, 3)), "");
        assert_eq!(take_str(lpp_str_substr(sub_s.as_ptr(), 2, 0)), "");

        // eq & cmp
        let s1 = cstr("apple");
        let s2 = cstr("apple");
        let s3 = cstr("banana");
        assert_eq!(lpp_str_eq(s1.as_ptr(), s2.as_ptr()), 1);
        assert_eq!(lpp_str_eq(s1.as_ptr(), s3.as_ptr()), 0);
        assert!(lpp_str_cmp(s1.as_ptr(), s3.as_ptr()) < 0);
        assert!(lpp_str_cmp(s3.as_ptr(), s1.as_ptr()) > 0);
        assert_eq!(lpp_str_cmp(s1.as_ptr(), s2.as_ptr()), 0);

        // lines with LF and CRLF
        let ml = cstr("first\r\nsecond\nthird\n");
        let lines = lpp_str_lines(ml.as_ptr());
        assert_eq!(lpp_list_len(lines), 3);
        assert_eq!(
            read_borrowed_str(lpp_list_get_arc(lines, 0) as *const c_char),
            "first"
        );
        assert_eq!(
            read_borrowed_str(lpp_list_get_arc(lines, 1) as *const c_char),
            "second"
        );
        assert_eq!(
            read_borrowed_str(lpp_list_get_arc(lines, 2) as *const c_char),
            "third"
        );
        lpp_arc_release(lines);

        // Numeric <-> string conversions
        assert_eq!(take_str(lpp_int_to_str(i64::MIN)), i64::MIN.to_string());
        assert_eq!(take_str(lpp_int_to_str(i64::MAX)), i64::MAX.to_string());
        assert_eq!(take_str(lpp_int_to_str(0)), "0");
        assert_eq!(take_str(lpp_u64_to_str(-1)), u64::MAX.to_string());
        assert_eq!(
            take_str(lpp_u64_to_hex(0xDEAD_BEEF_CAFEu64 as i64)),
            "deadbeefcafe"
        );
        assert_eq!(take_str(lpp_bool_to_str(1)), "true");
        assert_eq!(take_str(lpp_bool_to_str(0)), "false");
        assert!(take_str(lpp_float_to_str(3.5)).starts_with("3.5"));

        let p_neg = cstr("  -987654321 ");
        let p_u64 = cstr("18446744073709551615");
        let p_bad = cstr("not_a_number");
        assert_eq!(lpp_str_to_int(p_neg.as_ptr()), -987654321);
        assert_eq!(lpp_parse_int(p_neg.as_ptr()), -987654321);
        assert_eq!(lpp_str_to_u64(p_u64.as_ptr()) as u64, u64::MAX);
        assert_eq!(lpp_str_to_int(p_bad.as_ptr()), 0);
    }
}

#[test]
fn zero_copy_string_and_list_slices() {
    unsafe {
        // String slice (kind = 0)
        let base_str = lpp_str_concat(cstr("Hello ").as_ptr(), cstr("World!").as_ptr());
        let mut s_storage = [0u64; 8];
        let sv = lpp_slice_init(s_storage.as_mut_ptr().cast(), base_str.cast(), 6, 5, 0);
        assert_eq!(lpp_slice_len(sv), 5);
        assert_eq!(take_str(lpp_str_slice_get(sv, 0)), "W");
        assert_eq!(take_str(lpp_str_slice_get(sv, 4)), "d");
        assert_eq!(take_str(lpp_str_slice_to_str(sv)), "World");

        // Valid tail slice up to exact end of string
        let mut s_tail = [0u64; 8];
        let sv_tail = lpp_slice_init(s_tail.as_mut_ptr().cast(), base_str.cast(), 9, 3, 0);
        assert_eq!(lpp_slice_len(sv_tail), 3);
        assert_eq!(take_str(lpp_str_slice_to_str(sv_tail)), "ld!");
        lpp_arc_release(base_str.cast());

        // Int / Float / Bool list slice (kind = 1)
        let ilist = lpp_list_new();
        for v in [10, 20, 30, 40, 50] {
            lpp_list_push(ilist, v);
        }
        let mut l_storage = [0u64; 8];
        let lv = lpp_slice_init(l_storage.as_mut_ptr().cast(), ilist, 1, 3, 1);
        assert_eq!(lpp_slice_len(lv), 3);
        assert_eq!(lpp_slice_get(lv, 0), 20);
        assert_eq!(lpp_slice_get(lv, 2), 40);
        lpp_arc_release(ilist);

        let flist = lpp_list_new();
        lpp_list_push_float(flist, 1.25);
        lpp_list_push_float(flist, 2.5);
        lpp_list_push_float(flist, 3.75);
        let mut f_storage = [0u64; 8];
        let fv = lpp_slice_init(f_storage.as_mut_ptr().cast(), flist, 1, 2, 1);
        assert!((lpp_slice_get_float(fv, 0) - 2.5).abs() < 1e-12);
        assert!((lpp_slice_get_float(fv, 1) - 3.75).abs() < 1e-12);
        lpp_arc_release(flist);

        let blist = lpp_list_new();
        lpp_list_push_bool(blist, 0);
        lpp_list_push_bool(blist, 1);
        lpp_list_push_bool(blist, 1);
        let mut b_storage = [0u64; 8];
        let bv = lpp_slice_init(b_storage.as_mut_ptr().cast(), blist, 0, 2, 1);
        assert_eq!(lpp_slice_get_bool(bv, 0), 0);
        assert_eq!(lpp_slice_get_bool(bv, 1), 1);
        lpp_arc_release(blist);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Dynamic Lists (Int, Float, Bool, ARC ownership, Sort, Search, Extend)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn list_int_float_bool_operations_and_algorithms() {
    unsafe {
        let xs = lpp_list_new();
        lpp_list_reserve(xs, 64);
        assert!(lpp_list_capacity(xs) >= 64);
        assert_eq!(lpp_list_len(xs), 0);

        for i in [50, -10, 30, 0, 20, -40, 10] {
            lpp_list_push(xs, i);
        }
        assert_eq!(lpp_list_len(xs), 7);
        assert_eq!(lpp_list_index_of(xs, 30), 2);
        assert_eq!(lpp_list_index_of(xs, 999), -1);

        // Insert head, middle, tail
        lpp_list_insert(xs, 0, 100);
        lpp_list_insert(xs, 4, 25);
        lpp_list_insert(xs, lpp_list_len(xs), -99);
        assert_eq!(lpp_list_get(xs, 0), 100);
        assert_eq!(lpp_list_get(xs, 4), 25);
        assert_eq!(lpp_list_get(xs, lpp_list_len(xs) - 1), -99);

        // Swap, remove, pop, reverse
        lpp_list_swap(xs, 0, 1);
        assert_eq!(lpp_list_get(xs, 0), 50);
        assert_eq!(lpp_list_get(xs, 1), 100);
        let removed = lpp_list_remove(xs, 1);
        assert_eq!(removed, 100);
        let popped = lpp_list_pop(xs);
        assert_eq!(popped, -99);

        // Sort ascending + binary_search
        lpp_list_sort(xs);
        let mut sorted = Vec::new();
        for i in 0..lpp_list_len(xs) {
            sorted.push(lpp_list_get(xs, i));
        }
        assert_eq!(sorted, vec![-40, -10, 0, 10, 20, 25, 30, 50]);
        assert_eq!(lpp_list_binary_search(xs, 20), 4);
        assert_eq!(lpp_list_binary_search(xs, 15), -5);

        // Sort descending + reverse back
        lpp_list_sort_desc(xs);
        assert_eq!(lpp_list_get(xs, 0), 50);
        assert_eq!(lpp_list_get(xs, 7), -40);
        lpp_list_reverse(xs);
        assert_eq!(lpp_list_get(xs, 0), -40);
        assert_eq!(lpp_list_get(xs, 7), 50);

        // Unsigned sort places negative bit-patterns (high u64) after positive ints
        lpp_list_sort_u(xs);
        assert_eq!(lpp_list_get(xs, 0), 0);
        assert_eq!(lpp_list_get(xs, 5), 50);
        assert_eq!(lpp_list_get(xs, 6), -40);
        assert_eq!(lpp_list_get(xs, 7), -10);

        // Extend with a second list + truncate + clear
        let ys = lpp_list_new();
        lpp_list_push(ys, 77);
        lpp_list_push(ys, 88);
        lpp_list_extend(xs, ys);
        assert_eq!(lpp_list_len(xs), 10);
        assert_eq!(lpp_list_get(xs, 9), 88);
        lpp_list_truncate(xs, 3);
        assert_eq!(lpp_list_len(xs), 3);
        lpp_list_clear(xs);
        assert_eq!(lpp_list_len(xs), 0);
        lpp_list_free(ys);
        lpp_list_free(xs);

        // Float & bool list helpers
        let fs = lpp_list_new();
        lpp_list_push_float(fs, 1.5);
        lpp_list_push_float(fs, 2.5);
        lpp_list_set_float(fs, 0, -4.25);
        assert!((lpp_list_get_float(fs, 0) + 4.25).abs() < 1e-12);
        lpp_list_set(fs, 1, 99);
        assert_eq!(lpp_list_get(fs, 1), 99);
        lpp_arc_release(fs);

        let bs = lpp_list_new();
        lpp_list_push_bool(bs, 0);
        lpp_list_push_bool(bs, 1);
        lpp_list_set_bool(bs, 0, 5);
        assert_eq!(lpp_list_get_bool(bs, 0), 1);
        assert_eq!(lpp_list_get_bool(bs, 1), 1);
        lpp_arc_release(bs);
    }
}

#[test]
fn list_arc_ownership_self_overwrite_remove_and_drop() {
    let _guard = DROP_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let d_before = ARC_DTOR_CALLS.load(Ordering::Relaxed);
        let list = lpp_list_new_arc();

        let mk = || {
            let p = lpp_arc_alloc_with_destructor(8, Some(test_arc_dtor));
            *(p as *mut u64) = 0xCAFE_BABE_1234_5678;
            p
        };

        let o1 = mk();
        let o2 = mk();
        let o3 = mk();
        lpp_list_push_arc(list, o1); // refcount(o1) = 2
        lpp_list_push_arc(list, o2); // refcount(o2) = 2
        lpp_list_push_arc(list, o3); // refcount(o3) = 2

        // Release caller handles so list is the sole owner of o1, o2, o3
        lpp_arc_release(o1);
        lpp_arc_release(o2);
        lpp_arc_release(o3);
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before);

        // Critical self-assignment edge case: set_arc(list, 0, get_arc(list, 0))
        // when refcount == 1 must retain before dropping old element
        let borrowed0 = lpp_list_get_arc(list, 0);
        lpp_list_set_arc(list, 0, borrowed0);
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before);
        let hdr0 = (borrowed0 as *const u8).sub(ARC_HEADER_SIZE);
        assert_eq!(*(hdr0.add(ARC_REFCOUNT_OFFSET) as *const u32), 1);

        // Removing transfers ownership to caller (does NOT drop o2 inside remove)
        let removed = lpp_list_remove(list, 1) as *mut c_void;
        assert_eq!(removed, o2);
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before);
        lpp_arc_release(removed);
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before + 1);

        // Releasing list drops remaining o1 & o3
        lpp_arc_release(list);
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before + 3);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Hash Maps (Int/Str key isolation, Tombstones, Rehash, ARC/Float Values)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn map_int_and_str_keys_tombstone_reuse_rehash_and_float_arc() {
    let _guard = DROP_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let m = lpp_map_new();
        assert_eq!(lpp_map_len(m), 0);
        let cap0 = lpp_map_capacity(m);
        assert!(cap0 >= 8);

        // Integer key and C-string key whose pointer bit pattern matches integer key
        // must remain distinct via `is_str_key`
        let key_cstr = cstr("alpha_key");
        let ptr_as_int = key_cstr.as_ptr() as usize as i64;
        lpp_map_put(m, ptr_as_int, 111);
        lpp_map_put_str(m, key_cstr.as_ptr(), 222);
        assert_eq!(lpp_map_len(m), 2);
        assert_eq!(lpp_map_has(m, ptr_as_int), 1);
        assert_eq!(lpp_map_has_str(m, key_cstr.as_ptr()), 1);
        assert_eq!(lpp_map_get(m, ptr_as_int), 111);
        assert_eq!(lpp_map_get_str(m, key_cstr.as_ptr()), 222);

        // Removing string key leaves int key intact and is idempotent on missing keys
        lpp_map_remove_str(m, key_cstr.as_ptr());
        lpp_map_remove_str(m, key_cstr.as_ptr());
        assert_eq!(lpp_map_has_str(m, key_cstr.as_ptr()), 0);
        assert_eq!(lpp_map_get_str(m, key_cstr.as_ptr()), 0);
        assert_eq!(lpp_map_get(m, ptr_as_int), 111);
        lpp_map_remove(m, ptr_as_int);
        assert_eq!(lpp_map_len(m), 0);

        // Stress rehash + tombstone reuse across 200 keys
        for i in 0..200i64 {
            lpp_map_put(m, i * 17 - 50, i * 100);
        }
        assert_eq!(lpp_map_len(m), 200);
        assert!(lpp_map_capacity(m) > cap0);
        for i in (0..200i64).step_by(2) {
            lpp_map_remove(m, i * 17 - 50);
            assert_eq!(lpp_map_has(m, i * 17 - 50), 0);
        }
        assert_eq!(lpp_map_len(m), 100);
        // Remaining odd keys still probe past tombstones cleanly
        for i in (1..200i64).step_by(2) {
            assert_eq!(lpp_map_get(m, i * 17 - 50), i * 100);
        }
        // Re-insert into tombstones
        for i in (0..200i64).step_by(2) {
            lpp_map_put(m, i * 17 - 50, i + 7);
        }
        assert_eq!(lpp_map_len(m), 200);

        // keys() and values() snapshot lengths match map_len
        let ks = lpp_map_keys(m);
        let vs = lpp_map_values(m);
        assert_eq!(lpp_list_len(ks), 200);
        assert_eq!(lpp_list_len(vs), 200);
        lpp_arc_release(ks);
        lpp_arc_release(vs);

        // Float convenience wrappers
        let fk = cstr("tau");
        lpp_map_put_float(m, 9999, 1.5);
        lpp_map_put_str_float(m, fk.as_ptr(), 6.25);
        assert!((lpp_map_get_float(m, 9999) - 1.5).abs() < 1e-12);
        assert!((lpp_map_get_str_float(m, fk.as_ptr()) - 6.25).abs() < 1e-12);

        lpp_map_clear(m);
        assert_eq!(lpp_map_len(m), 0);
        lpp_arc_release(m);

        // ARC-value map releases overwritten/removed/remaining values deterministically
        let d_before = ARC_DTOR_CALLS.load(Ordering::Relaxed);
        let ma = lpp_map_new_arc();
        let v1 = lpp_arc_alloc_with_destructor(8, Some(test_arc_dtor));
        let v2 = lpp_arc_alloc_with_destructor(8, Some(test_arc_dtor));
        *(v1 as *mut u64) = 0xCAFE_BABE_1234_5678;
        *(v2 as *mut u64) = 0xCAFE_BABE_1234_5678;
        lpp_map_put(ma, 1, v1 as usize as i64);
        lpp_arc_release(v1);
        lpp_map_put(ma, 1, v2 as usize as i64); // overwrites v1 -> drops v1
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before + 1);
        lpp_arc_release(v2);
        lpp_arc_release(ma); // drops v2
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before + 2);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Sets & Ring-Buffer Deques
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn set_int_and_str_algebra_clear_resize_and_free() {
    unsafe {
        let s1 = lpp_set_new();
        let s2 = lpp_set_new();

        // Insert + duplicate detection + resize over 500 elements
        for i in 0..500i64 {
            assert_eq!(lpp_set_add(s1, i), 1);
            assert_eq!(lpp_set_add(s1, i), 0, "duplicate add must return 0");
        }
        assert_eq!(lpp_set_len(s1), 500);
        assert_eq!(lpp_set_has(s1, 0), 1);
        assert_eq!(lpp_set_has(s1, 499), 1);
        assert_eq!(lpp_set_has(s1, 500), 0);

        for i in 250..500i64 {
            assert_eq!(lpp_set_remove(s1, i), 1);
            assert_eq!(lpp_set_remove(s1, i), 0);
        }
        assert_eq!(lpp_set_len(s1), 250);

        // String keys mixed with int keys
        let ka = cstr("red");
        let kb = cstr("green");
        let kc = cstr("blue");
        assert_eq!(lpp_set_add_str(s1, ka.as_ptr()), 1);
        assert_eq!(lpp_set_add_str(s1, kb.as_ptr()), 1);
        assert_eq!(lpp_set_add_str(s1, std::ptr::null()), 0);
        assert_eq!(lpp_set_has_str(s1, ka.as_ptr()), 1);
        assert_eq!(lpp_set_has_str(s1, kc.as_ptr()), 0);

        for i in 200..300i64 {
            lpp_set_add(s2, i);
        }
        lpp_set_add_str(s2, kb.as_ptr());
        lpp_set_add_str(s2, kc.as_ptr());

        // Union: s1 (0..250, red, green) U s2 (200..300, green, blue) -> 300 ints + 3 strs = 303
        let su = lpp_set_union(s1, s2);
        assert_eq!(lpp_set_len(su), 303);
        assert_eq!(lpp_set_has(su, 0), 1);
        assert_eq!(lpp_set_has(su, 299), 1);
        assert_eq!(lpp_set_has_str(su, ka.as_ptr()), 1);
        assert_eq!(lpp_set_has_str(su, kc.as_ptr()), 1);

        // Intersect: (200..250) + green -> 51
        let si = lpp_set_intersect(s1, s2);
        assert_eq!(lpp_set_len(si), 51);
        assert_eq!(lpp_set_has(si, 200), 1);
        assert_eq!(lpp_set_has(si, 199), 0);
        assert_eq!(lpp_set_has(si, 250), 0);
        assert_eq!(lpp_set_has_str(si, kb.as_ptr()), 1);
        assert_eq!(lpp_set_has_str(si, ka.as_ptr()), 0);

        // Remove str + clear + free + invalid handle guards
        assert_eq!(lpp_set_remove_str(si, kb.as_ptr()), 1);
        assert_eq!(lpp_set_len(si), 50);
        lpp_set_clear(si);
        assert_eq!(lpp_set_len(si), 0);

        lpp_set_free(su);
        lpp_set_free(si);
        lpp_set_free(s2);
        lpp_set_free(s1);
        assert_eq!(lpp_set_len(s1), 0);
        assert_eq!(lpp_set_has(s1, 0), 0);
    }
}

#[test]
fn deque_front_back_wraparound_grow_peek_and_clear() {
    let dq = lpp_deque_new();
    assert_eq!(lpp_deque_len(dq), 0);
    assert_eq!(lpp_deque_peek_front(dq), 0);
    assert_eq!(lpp_deque_peek_back(dq), 0);
    assert_eq!(lpp_deque_pop_front(dq), 0);
    assert_eq!(lpp_deque_pop_back(dq), 0);

    // Alternating front/back pushes across 500 elements to trigger multiple ring buffer growths
    for i in 1..=250i64 {
        lpp_deque_push_front(dq, -i);
        lpp_deque_push_back(dq, i);
    }
    assert_eq!(lpp_deque_len(dq), 500);
    assert_eq!(lpp_deque_peek_front(dq), -250);
    assert_eq!(lpp_deque_peek_back(dq), 250);
    assert_eq!(lpp_deque_get(dq, 0), -250);
    assert_eq!(lpp_deque_get(dq, 249), -1);
    assert_eq!(lpp_deque_get(dq, 250), 1);
    assert_eq!(lpp_deque_get(dq, 499), 250);
    assert_eq!(lpp_deque_get(dq, -1), 0);
    assert_eq!(lpp_deque_get(dq, 500), 0);

    // Sliding window wraparound churn: push back, pop front
    for round in 0..200i64 {
        lpp_deque_push_back(dq, 1000 + round);
        let front = lpp_deque_pop_front(dq);
        assert_eq!(front, -250 + round);
    }
    lpp_deque_clear(dq);
    assert_eq!(lpp_deque_len(dq), 0);

    // Exact wraparound FIFO check against std::collections::VecDeque oracle
    let mut model = std::collections::VecDeque::new();
    for step in 0..300i64 {
        if step % 3 != 0 {
            lpp_deque_push_back(dq, step);
            model.push_back(step);
        } else {
            lpp_deque_push_front(dq, -step);
            model.push_front(-step);
        }
        if step % 2 == 1 {
            assert_eq!(lpp_deque_pop_front(dq), model.pop_front().unwrap());
        }
    }
    assert_eq!(lpp_deque_len(dq), model.len() as i64);
    for (i, &v) in model.iter().enumerate() {
        assert_eq!(lpp_deque_get(dq, i as i64), v);
    }

    lpp_deque_free(dq);
    assert_eq!(lpp_deque_len(dq), 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. Byte Buffers, CRC32, File I/O & JSON Parser/Escaper
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn buffers_endian_reads_writes_overlap_copy_crc32_and_file_io() {
    let td = TempDir::new("buf_io");
    unsafe {
        let b = lpp_buf_alloc(64);
        assert_ne!(b, 0);
        assert_eq!(lpp_buf_len(b), 64);

        // 8/16/32/64 LE & BE roundtrips
        lpp_buf_set8(b, 0, 0xAB);
        assert_eq!(lpp_buf_get8(b, 0), 0xAB);

        lpp_buf_set16le(b, 2, 0x1234);
        lpp_buf_set16be(b, 4, 0x1234);
        assert_eq!(lpp_buf_get16le(b, 2), 0x1234);
        assert_eq!(lpp_buf_get16be(b, 2), 0x3412);
        assert_eq!(lpp_buf_get16be(b, 4), 0x1234);
        assert_eq!(lpp_buf_get16le(b, 4), 0x3412);

        lpp_buf_set32le(b, 8, 0x89AB_CDEFu32 as i64);
        lpp_buf_set32be(b, 12, 0x89AB_CDEFu32 as i64);
        assert_eq!(lpp_buf_get32le(b, 8) as u32, 0x89AB_CDEF);
        assert_eq!(lpp_buf_get32be(b, 8) as u32, 0xEFCD_AB89);
        assert_eq!(lpp_buf_get32be(b, 12) as u32, 0x89AB_CDEF);

        let v64 = 0x0123_4567_89AB_CDEFu64 as i64;
        lpp_buf_set64le(b, 16, v64);
        lpp_buf_set64be(b, 24, v64);
        assert_eq!(lpp_buf_get64le(b, 16), v64);
        assert_eq!(lpp_buf_get64be(b, 24), v64);
        assert_eq!(lpp_buf_get64be(b, 16) as u64, 0xEFCD_AB89_6745_2301);

        // Out-of-bounds guards return 0 and do not fault
        assert_eq!(lpp_buf_get8(b, -1), 0);
        assert_eq!(lpp_buf_get64le(b, 60), 0);
        lpp_buf_set32be(b, 62, 0x5555_5555);

        // IEEE 802.3 CRC-32 standard check vector: "123456789" -> 0xCBF43926
        let vec_str = cstr("123456789");
        let n_written = lpp_buf_write_str(b, 32, vec_str.as_ptr());
        assert_eq!(n_written, 9);
        assert_eq!(lpp_buf_crc32(b, 32, 9) as u32, 0xCBF4_3926);
        assert_eq!(take_str(lpp_buf_to_str(b, 32, 9)), "123456789");
        assert_eq!(take_str(lpp_buf_read_str(b, 35, 4)), "4567");

        // Overlapping memmove copy forward (dst > src) and backward (dst < src)
        lpp_buf_copy(b, 34, b, 32, 7); // "12" + "1234567"
        assert_eq!(take_str(lpp_buf_to_str(b, 32, 9)), "121234567");
        lpp_buf_copy(b, 32, b, 34, 7);
        assert_eq!(take_str(lpp_buf_to_str(b, 32, 7)), "1234567");

        // File write + read roundtrip
        let file_p = td.path("sub/blob.bin");
        let file_c = cstr(&file_p.to_string_lossy());
        assert_eq!(lpp_buf_write(file_c.as_ptr(), b), 0);
        let b2 = lpp_buf_read(file_c.as_ptr());
        assert_ne!(b2, 0);
        assert_eq!(lpp_buf_len(b2), 64);
        assert_eq!(lpp_buf_get64le(b2, 16), v64);
        assert_eq!(lpp_buf_get64be(b2, 24), v64);
        lpp_buf_free(b2);

        // Register for at-exit cleanup & explicit free
        let b3 = lpp_buf_alloc(16);
        lpp_buf_free_at_exit(b3);
        lpp_buf_free(b);
    }
}

#[test]
fn json_complex_escapes_nesting_depth_and_error_paths() {
    unsafe {
        let doc = cstr(
            r#"{
                "int": -42,
                "flag_t": true,
                "flag_f": false,
                "nothing": null,
                "flt": 12.75,
                "esc": "line1\nline2\t\"quoted\"\\slash\/\b\f\r\u0041\u00e9",
                "outer": { "inner": { "val": 9001 } },
                "arr": [1, 2, {"x": 3}]
            }"#,
        );
        let root = lpp_json_parse(doc.as_ptr());
        assert_ne!(root, 0);

        assert_eq!(lpp_json_get_int(root, cstr("int").as_ptr()), -42);
        assert_eq!(lpp_json_get_int(root, cstr("flag_t").as_ptr()), 1);
        assert_eq!(lpp_json_get_int(root, cstr("flag_f").as_ptr()), 0);
        assert_eq!(lpp_json_get_int(root, cstr("flt").as_ptr()), 12);
        assert_eq!(lpp_json_get_int(root, cstr("missing").as_ptr()), 0);

        let esc_s = take_str(lpp_json_get_str(root, cstr("esc").as_ptr()));
        assert_eq!(esc_s, "line1\nline2\t\"quoted\"\\slash/\x08\x0c\rA\u{00e9}");

        let outer = lpp_json_get_obj(root, cstr("outer").as_ptr());
        assert_ne!(outer, 0);
        let inner = lpp_json_get_obj(outer, cstr("inner").as_ptr());
        assert_ne!(inner, 0);
        assert_eq!(lpp_json_get_int(inner, cstr("val").as_ptr()), 9001);
        assert_eq!(lpp_json_get_obj(root, cstr("missing_obj").as_ptr()), 0);

        // Non-root free is a safe no-op; double-free on root is also guarded
        lpp_json_free(inner);
        lpp_json_free(outer);
        lpp_json_free(root);
        lpp_json_free(root);

        // Malformed JSON and excessive nesting (> MAX_DEPTH = 256) return 0
        assert_eq!(lpp_json_parse(std::ptr::null()), 0);
        assert_eq!(lpp_json_parse(cstr("[1, 2").as_ptr()), 0);
        assert_eq!(lpp_json_parse(cstr("invalid_json").as_ptr()), 0);

        let deep = format!("{}{}", "[".repeat(300), "]".repeat(300));
        assert_eq!(lpp_json_parse(cstr(&deep).as_ptr()), 0);

        // Escaper covers control bytes (< 0x20), quotes, backslashes, and normal UTF-8
        let raw = cstr("hi\"\\world\n\t\r\x08\x0c\x01");
        assert_eq!(
            take_str(lpp_json_escape(raw.as_ptr())),
            r#"hi\"\\world\n\t\r\b\f\u0001"#
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. Numerics, Clocks/RNG/Raw-C-Mem & Subprocess Panic/Backtrace Gating
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn numeric_math_overflow_unsigned_bits_trunc_and_avx2_checksum() {
    assert_eq!(lpp_abs(-99), 99);
    assert_eq!(lpp_abs(42), 42);
    assert_eq!(lpp_min(-5, 5), -5);
    assert_eq!(lpp_max(-5, 5), 5);
    assert!((lpp_sqrt(144.0) - 12.0).abs() < 1e-12);
    assert_eq!(lpp_floor(3.9), 3.0);
    assert_eq!(lpp_ceil(3.1), 4.0);
    assert!((lpp_pow(2.0, 10.0) - 1024.0).abs() < 1e-12);
    assert!(lpp_sin(0.0).abs() < 1e-12);
    assert!((lpp_cos(0.0) - 1.0).abs() < 1e-12);
    assert!(lpp_tan(0.0).abs() < 1e-12);
    assert_eq!(lpp_int_to_float(-77), -77.0);
    assert_eq!(lpp_float_to_int(-77.9), -77);

    // Overflow predicates & int_pow
    assert_eq!(lpp_add_overflows(i64::MAX, 1), 1);
    assert_eq!(lpp_add_overflows(i64::MIN, -1), 1);
    assert_eq!(lpp_add_overflows(i64::MAX, 0), 0);
    assert_eq!(lpp_mul_overflows(i64::MAX, 2), 1);
    assert_eq!(lpp_mul_overflows(1000, 1000), 0);
    assert_eq!(lpp_int_pow(3, 5), 243);
    assert_eq!(lpp_int_pow(5, 0), 1);
    assert_eq!(lpp_int_pow(2, -1), 0);

    // Unsigned arithmetic & comparisons
    let neg1 = -1i64; // u64::MAX
    assert_eq!(lpp_shr_u(neg1, 1) as u64, u64::MAX >> 1);
    assert_eq!(lpp_shl_u(1, 63), i64::MIN);
    assert_eq!(lpp_div_u(neg1, 2) as u64, u64::MAX / 2);
    assert_eq!(lpp_rem_u(neg1, 10), (u64::MAX % 10) as i64);
    assert_eq!(lpp_lt_u(10, neg1), 1);
    assert_eq!(lpp_le_u(neg1, neg1), 1);
    assert_eq!(lpp_gt_u(neg1, 10), 1);
    assert_eq!(lpp_ge_u(10, neg1), 0);
    assert_eq!(lpp_min_u(10, neg1), 10);
    assert_eq!(lpp_max_u(10, neg1), neg1);

    // Bit rotates, clz/ctz/popcount, bswap
    assert_eq!(
        lpp_rotl64(0x0123_4567_89AB_CDEFu64 as i64, 8) as u64,
        0x2345_6789_ABCD_EF01
    );
    assert_eq!(
        lpp_rotr64(0x2345_6789_ABCD_EF01u64 as i64, 8) as u64,
        0x0123_4567_89AB_CDEF
    );
    assert_eq!(lpp_rotl32(0x1234_5678, 16), 0x5678_1234);
    assert_eq!(lpp_rotr32(0x5678_1234, 16), 0x1234_5678);
    assert_eq!(lpp_clz64(0), 64);
    assert_eq!(lpp_ctz64(0), 64);
    assert_eq!(lpp_clz64(1), 63);
    assert_eq!(lpp_ctz64(0x80), 7);
    assert_eq!(lpp_popcount64(0x5555_5555_5555_5555u64 as i64), 32);
    assert_eq!(lpp_bswap16(0x1234), 0x3412);
    assert_eq!(lpp_bswap32(0x1234_5678), 0x7856_3412);
    assert_eq!(lpp_bswap64(0x0102_0304_0506_0708), 0x0807_0605_0403_0201);

    // C-style truncations & checked/wrapping non-overflow paths
    assert_eq!(lpp_trunc_u8(0x1FF), 0xFF);
    assert_eq!(lpp_trunc_u16(0x1_2345), 0x2345);
    assert_eq!(lpp_trunc_u32(0x1_ABCD_EF01), 0xABCD_EF01);
    assert_eq!(lpp_trunc_i8(0xFF), -1);
    assert_eq!(lpp_trunc_i16(0xFFFF), -1);
    assert_eq!(lpp_trunc_i32(0xFFFF_FFFF), -1);

    assert_eq!(lpp_add_checked(i64::MAX, 0), i64::MAX);
    assert_eq!(lpp_sub_checked(i64::MIN, 0), i64::MIN);
    assert_eq!(lpp_mul_checked(12345, -2), -24690);
    assert_eq!(lpp_add_wrap(i64::MAX, 1), i64::MIN);
    assert_eq!(lpp_sub_wrap(i64::MIN, 1), i64::MAX);
    assert_eq!(lpp_mul_wrap(i64::MAX, 2), -2);

    // Vectorized AVX2 vs scalar oracle checksum across boundary lengths (0, <64, 64, 65, 513)
    for n in [-3i64, 0, 1, 63, 64, 65, 131, 513] {
        let mut expected = 0i64;
        if n > 0 {
            for i in 0..n {
                expected = expected.wrapping_add(i.wrapping_mul(3) ^ (i >> 1));
            }
        }
        assert_eq!(
            lpp_vec_i64_checksum(n),
            expected,
            "checksum mismatch for n={n}"
        );
    }
}

#[test]
fn clock_rng_isolated_handles_and_c_raw_memory() {
    // Global RNG determinism (serialized across tests)
    {
        let _rng_guard = RNG_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        lpp_random_seed(424242);
        let a1 = lpp_random();
        let a2 = lpp_random();
        lpp_random_seed(424242);
        assert_eq!(lpp_random(), a1);
        assert_eq!(lpp_random(), a2);
        for _ in 0..64 {
            let v = lpp_random_range(-5, 15);
            assert!((-5..=15).contains(&v));
        }
    }

    // Isolated RNG instances do not interfere with each other or global RNG
    let r1 = lpp_rng_new(777);
    let r2 = lpp_rng_new(777);
    assert_ne!(r1, 0);
    assert_ne!(r2, 0);
    for _ in 0..32 {
        assert_eq!(lpp_rng_next(r1), lpp_rng_next(r2));
        let bounded = lpp_rng_range(r1, 10, 20);
        let _ = lpp_rng_range(r2, 10, 20);
        assert!((10..=20).contains(&bounded));
        let f = lpp_rng_float(r1);
        let _ = lpp_rng_float(r2);
        assert!((0.0..1.0).contains(&f));
    }
    lpp_rng_free(r1);
    lpp_rng_free(r2);

    // Mock clock + wall clock monotonicity
    let t0 = lpp_time_ms();
    let n0 = lpp_time_ns();
    assert!(t0 > 0 && n0 > 0);
    let t1 = lpp_time_ms();
    let n1 = lpp_time_ns();
    assert!(t1 >= t0 && n1 >= n0);

    let clk = lpp_clock_new(1_000_000);
    assert_eq!(lpp_clock_now(clk), 1_000_000);
    lpp_clock_advance(clk, 5_500);
    assert_eq!(lpp_clock_now(clk), 1_005_500);
    lpp_clock_free(clk);

    // Raw C malloc + unaligned u8/i32/i64 load/store + null guards
    unsafe {
        assert_eq!(lpp_c_load_u8(0, 0), 0);
        assert_eq!(lpp_c_load_i32(0, 0), 0);
        assert_eq!(lpp_c_load_i64(0, 0), 0);
        lpp_c_store_u8(0, 0, 1);
        lpp_c_store_i32(0, 0, 1);
        lpp_c_store_i64(0, 0, 1);

        let mem = lpp_c_malloc(32);
        assert_ne!(mem, 0);
        lpp_c_store_u8(mem, 1, 0xFE);
        assert_eq!(lpp_c_load_u8(mem, 1), 0xFE);
        lpp_c_store_i32(mem, 3, -123456);
        assert_eq!(lpp_c_load_i32(mem, 3), -123456);
        lpp_c_store_i64(mem, 9, i64::MIN + 77);
        assert_eq!(lpp_c_load_i64(mem, 9), i64::MIN + 77);
        lpp_c_free(mem);
    }
}

#[test]
fn panic_and_backtrace_gating_subprocess() {
    if let Ok(mode) = std::env::var("LPP_V03_PANIC_SUBPROC") {
        match mode.as_str() {
            "reason" => runtime_panic("synthetic v03 panic reason"),
            "div_zero" => lpp_panic_div_zero(),
            "add_overflow" => {
                let _ = lpp_add_checked(i64::MAX, 1);
            }
            "sub_overflow" => {
                let _ = lpp_sub_checked(i64::MIN, 1);
            }
            "mul_overflow" => {
                let _ = lpp_mul_checked(i64::MAX, 2);
            }
            other => panic!("unexpected subprocess mode: {other}"),
        }
    }

    let exe = std::env::current_exe().expect("current test executable path");
    let run_case = |mode: &str, backtrace: Option<&str>| -> (i32, String) {
        let mut cmd = Command::new(&exe);
        cmd.args([
            "--exact",
            "panic_and_backtrace_gating_subprocess",
            "--nocapture",
        ]);
        cmd.env("LPP_V03_PANIC_SUBPROC", mode);
        match backtrace {
            Some(v) => {
                cmd.env("LPP_BACKTRACE", v);
            }
            None => {
                cmd.env_remove("LPP_BACKTRACE");
            }
        }
        let out = cmd.output().expect("spawn subprocess panic case");
        let code = out.status.code().expect("process exit code");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (code, stderr)
    };

    // Unset and "0" suppress Backtrace
    for bt in [None, Some("0"), Some("false")] {
        let (code, stderr) = run_case("reason", bt);
        assert_eq!(code, 101);
        assert!(stderr.contains("L++ RUNTIME PANIC"));
        assert!(stderr.contains("Reason: synthetic v03 panic reason"));
        assert!(
            !stderr.contains("Backtrace:"),
            "Backtrace must stay suppressed for LPP_BACKTRACE={bt:?}, got:\n{stderr}"
        );
    }

    // "1", "full", and "FULL" enable Backtrace
    for bt in ["1", "full", "FULL"] {
        let (code, stderr) = run_case("reason", Some(bt));
        assert_eq!(code, 101);
        assert!(stderr.contains("L++ RUNTIME PANIC"));
        assert!(stderr.contains("Reason: synthetic v03 panic reason"));
        assert!(
            stderr.contains("Backtrace:"),
            "Backtrace must be emitted for LPP_BACKTRACE={bt}, got:\n{stderr}"
        );
    }

    // Division by zero & checked arithmetic overflows exit 101 with expected reasons
    let (code_dz, err_dz) = run_case("div_zero", Some("1"));
    assert_eq!(code_dz, 101);
    assert!(err_dz.contains("Reason: integer division by zero"));
    assert!(err_dz.contains("Backtrace:"));

    let (code_add, err_add) = run_case("add_overflow", None);
    assert_eq!(code_add, 101);
    assert!(err_add.contains("integer addition overflow"));

    let (code_sub, err_sub) = run_case("sub_overflow", None);
    assert_eq!(code_sub, 101);
    assert!(err_sub.contains("integer subtraction overflow"));

    let (code_mul, err_mul) = run_case("mul_overflow", None);
    assert_eq!(code_mul, 101);
    assert!(err_mul.contains("integer multiplication overflow"));
}

#[test]
fn multithreaded_arc_retain_release_and_tuple_mixed_slot_cycle() {
    let _serial = COLLECT_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _drop_guard = DROP_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        // Atomic ARC retain/release contention across 8 worker threads on one shared object
        let d_before = ARC_DTOR_CALLS.load(Ordering::Relaxed);
        let shared = lpp_arc_alloc_with_destructor(8, Some(test_arc_dtor));
        *(shared as *mut u64) = 0xCAFE_BABE_1234_5678;

        let shared_addr = shared as usize;
        let mut handles = Vec::new();
        for _ in 0..8 {
            lpp_arc_retain(shared);
            handles.push(std::thread::spawn(move || {
                let ptr = shared_addr as *mut c_void;
                for _ in 0..2_000 {
                    lpp_arc_retain(ptr);
                    lpp_arc_release(ptr);
                }
                lpp_arc_release(ptr);
            }));
        }
        for h in handles {
            h.join().expect("worker join");
        }
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before);
        lpp_arc_release(shared);
        assert_eq!(ARC_DTOR_CALLS.load(Ordering::Relaxed), d_before + 1);

        // Mixed-slot tuple: LppTuplePrefix occupies bytes 0..16; user slots start at @16:
        // slot 0 @16 (managed), slot 1 @24 (unmanaged scalar 0xBAD), slot 2 @32 (managed)
        lpp_arc_collect_cycles();
        let drops0 = NODE_DROPS.load(Ordering::Relaxed);
        let packed_offsets = 16i64 | (24i64 << 16) | (32i64 << 32);
        let tuple_ptr = lpp_tuple_alloc(40, 0b0101, packed_offsets);
        lpp_arc_mark_collectible(tuple_ptr, Some(lpp_tuple_trace));

        let n1 = alloc_cycle_node(11);
        let n2 = alloc_cycle_node(22);
        let slots = tuple_ptr as *mut usize;
        *slots.add(2) = n1 as usize;
        lpp_arc_retain(n1);
        *slots.add(3) = 0xDEAD_BEEF_usize; // unmanaged scalar must NOT be traced
        *slots.add(4) = n2 as usize;
        lpp_arc_retain(n2);

        (*(n1 as *mut CycleNode)).child1 = tuple_ptr;
        lpp_arc_retain(tuple_ptr);
        (*(n2 as *mut CycleNode)).child1 = n1;
        lpp_arc_retain(n1);

        lpp_arc_release(n1);
        lpp_arc_release(n2);
        lpp_arc_release(tuple_ptr);

        lpp_arc_collect_cycles();
        assert_eq!(
            NODE_DROPS.load(Ordering::Relaxed) - drops0,
            2,
            "mixed managed/unmanaged tuple cycle must reclaim both nodes cleanly"
        );
    }
}
