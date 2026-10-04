//! The ARC core of the L++ runtime (Phase 6B.1).
//!
//! Rust re-implementation of the `lpp_runtime.c` ARC primitives with
//! **identical ABI**: same header layout, same immortal sentinel, same
//! memory ordering (acq_rel atomic retain/release, relaxed local fast
//! path), same generation protocol for weak handles, same allocator
//! (libc calloc/free — the C runtime's own allocator, so object
//! identity and recycling behavior match byte for byte). The v1 C
//! runtime remains the behavioral reference — see
//! `tests/runtime_gate.rs`, which runs differential sequences against
//! both implementations.

use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

use libc::{c_void, calloc, free};

use crate::layout::{
    ARC_DESTRUCTOR_OFFSET, ARC_GENERATION_OFFSET, ARC_HEADER_SIZE, ARC_MAGIC,
    ARC_MAGIC_OFFSET, ARC_REFCOUNT_OFFSET, ARC_IMMORTAL, EMPTY_STR_BLOB_WORDS,
    EMPTY_STR_PAYLOAD_WORD, MIN_VALID_ADDR,
};

/// Monotonic source of object generations. Internal to the runtime — the v1
/// C reference keeps it `static` (unexported), and generated objects reach
/// generations only through `lpp_weak_generation`, never this global, so it
/// is deliberately NOT part of the exported ABI. First allocation gets
/// generation 2 (`fetch_add(1) + 1`, starting at 1) — the same off-by-one
/// the C runtime has, deliberately.
static GENERATION_COUNTER: AtomicI32 = AtomicI32::new(1);

fn next_generation() -> i32 {
    GENERATION_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
}

// The hidden header lives `ARC_HEADER_SIZE` bytes before the payload.

fn header_of(payload: *const c_void) -> *mut c_void {
    unsafe { payload.cast::<u8>().sub(ARC_HEADER_SIZE) }.cast_mut().cast::<c_void>()
}

fn magic_cell(payload: *const c_void) -> *mut u32 {
    let hdr = header_of(payload).cast::<u32>();
    unsafe { hdr.add(ARC_MAGIC_OFFSET / 4) }
}

fn refcount_cell(payload: *const c_void) -> *mut AtomicI32 {
    let hdr = header_of(payload).cast::<AtomicI32>();
    unsafe { hdr.add(ARC_REFCOUNT_OFFSET / 4) }
}

fn generation_cell(payload: *const c_void) -> *mut AtomicI32 {
    let hdr = header_of(payload).cast::<AtomicI32>();
    unsafe { hdr.add(ARC_GENERATION_OFFSET / 4) }
}

fn destructor_cell(payload: *const c_void) -> *mut Option<unsafe extern "C" fn(*mut c_void)> {
    // The destructor sits at byte offset 16 — 8-byte aligned (the C struct
    // pads generation@8..12 so the pointer lands on a boundary). One clean
    // pointer-sized step past the header base.
    let hdr = header_of(payload)
        .cast::<Option<unsafe extern "C" fn(*mut c_void)>>();
    unsafe { hdr.add(ARC_DESTRUCTOR_OFFSET / std::mem::size_of::<*const c_void>()) }
}

fn is_immortal(payload: *const c_void) -> bool {
    let rc = unsafe { (*refcount_cell(payload)).load(Ordering::Relaxed) };
    rc as u32 == ARC_IMMORTAL
}

/// Mirrors C `lpp__is_valid_arc_ptr`: non-null, 8-aligned, beyond the guard
/// pages, and a live magic in the hidden header.
fn is_valid_arc_ptr(ptr: *const c_void) -> bool {
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;
    if addr % 8 != 0 || addr < MIN_VALID_ADDR {
        return false;
    }
    let magic = unsafe { magic_cell(ptr).read_volatile() };
    magic == ARC_MAGIC
}

/// Allocate an ARC object with an optional type-specific destructor.
///
/// Returns the payload pointer (past the hidden 24-byte header), refcount
/// 1, generation from the process-global counter. Mirrors C
/// `lpp_arc_alloc_with_destructor` exactly (calloc-zeroed, same field
/// order, same generation protocol, same allocator).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_arc_alloc_with_destructor(
    size: i64,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> *mut c_void {
    let base = unsafe { calloc(1, (ARC_HEADER_SIZE + size.max(0) as usize) as libc::size_t) };
    if base.is_null() {
        return std::ptr::null_mut();
    }
    // The cell helpers map a PAYLOAD pointer back to the hidden header, so
    // compute the payload first and initialize through it (passing `base`
    // here would write 24 bytes *before* the allocation).
    let payload = unsafe { base.add(ARC_HEADER_SIZE) }.cast::<c_void>();
    unsafe {
        magic_cell(payload).write_volatile(ARC_MAGIC);
        (*refcount_cell(payload)).store(1, Ordering::Relaxed);
        (*generation_cell(payload)).store(next_generation(), Ordering::Relaxed);
        *destructor_cell(payload) = destructor;
    }
    payload
}

/// Backwards-compatible allocation with no destructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_arc_alloc(size: i64) -> *mut c_void {
    unsafe { lpp_arc_alloc_with_destructor(size, None) }
}

/// Increment the refcount. No-op for NULL, foreign, or immortal pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_arc_retain(ptr: *mut c_void) {
    if !is_valid_arc_ptr(ptr) || is_immortal(ptr) {
        return;
    }
    unsafe { (*refcount_cell(ptr)).fetch_add(1, Ordering::AcqRel) };
}

/// Decrement the refcount. Frees (generation bump, magic clear, destructor,
/// then deallocation) when it reaches zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_arc_release(ptr: *mut c_void) {
    if !is_valid_arc_ptr(ptr) || is_immortal(ptr) {
        return;
    }
    let prev = unsafe { (*refcount_cell(ptr)).fetch_sub(1, Ordering::AcqRel) };
    if prev == 1 {
        unsafe { drop_arc(ptr) };
    }
}

/// Free-path tail shared by both release variants: bump the generation
/// BEFORE freeing (load-bearing: a weak reader that still sees the old
/// generation is guaranteed to predate the deallocation), clear the magic,
/// run the destructor exactly once, then deallocate the header+payload
/// block through the same allocator it came from.
unsafe fn drop_arc(ptr: *mut c_void) {
    lpp__runtime_drop_count.fetch_add(1, Ordering::Relaxed);
    unsafe {
        (*generation_cell(ptr)).fetch_add(1, Ordering::Release);
        magic_cell(ptr).write_volatile(0);
        let destructor = *destructor_cell(ptr);
        if let Some(f) = destructor {
            f(ptr);
        }
        free(header_of(ptr.cast::<c_void>()));
    }
}

/// Thread-local ARC fast path: identical semantics to [`lpp_arc_retain`]
/// with relaxed ordering, emitted by the compiler for programs proven
/// single-threaded (no `spawn`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_arc_retain_local(ptr: *mut c_void) {
    if !is_valid_arc_ptr(ptr) || is_immortal(ptr) {
        return;
    }
    unsafe { (*refcount_cell(ptr)).fetch_add(1, Ordering::Relaxed) };
}

/// Thread-local release: relaxed ordering, same free contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_arc_release_local(ptr: *mut c_void) {
    if !is_valid_arc_ptr(ptr) || is_immortal(ptr) {
        return;
    }
    let prev = unsafe { (*refcount_cell(ptr)).fetch_sub(1, Ordering::Relaxed) };
    if prev == 1 {
        unsafe { drop_arc(ptr) };
    }
}

/// The current generation of a live object (for weak handles). Immortal
/// objects report the sentinel itself; invalid pointers report 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_weak_generation(ptr: *mut c_void) -> i64 {
    if !is_valid_arc_ptr(ptr) {
        return 0;
    }
    if is_immortal(ptr) {
        return ARC_IMMORTAL as i64;
    }
    i64::from(unsafe { (*generation_cell(ptr)).load(Ordering::Acquire) })
}

/// Dereference a weak handle: the pointer if the target is still the same
/// live object, else 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_weak_get(raw: i64, expected_generation: i64) -> i64 {
    let ptr = raw as *mut c_void;
    if raw == 0 || expected_generation == 0 {
        return 0;
    }
    if !is_valid_arc_ptr(ptr) {
        return 0;
    }
    if is_immortal(ptr) {
        return if expected_generation == ARC_IMMORTAL as i64 { raw } else { 0 };
    }
    let now = i64::from(unsafe { (*generation_cell(ptr)).load(Ordering::Acquire) });
    if now != expected_generation {
        return 0;
    }
    raw
}

/// The shared immortal empty string: one 16-aligned blob whose first two
/// words hold the magic/immortal constants, payload `""` at word 6.
#[repr(align(16))]
pub struct EmptyStrBlob(pub [u32; EMPTY_STR_BLOB_WORDS]);

#[unsafe(no_mangle)]
pub static lpp__empty_str_blob: EmptyStrBlob = EmptyStrBlob([
    ARC_MAGIC,
    ARC_IMMORTAL,
    0,
    0,
    0,
    0,
    0,
    0,
]);

/// Pointer to the (empty) payload of the immortal empty-string blob.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_empty_str() -> *mut c_void {
    let base = lpp__empty_str_blob.0.as_ptr() as *mut u8;
    unsafe { base.add(EMPTY_STR_PAYLOAD_WORD * 4) }.cast::<c_void>()
}

/// An ARC-managed closure payload is two pointer-sized words:
/// `[code pointer, environment pointer]`. The code pointer is non-owning;
/// the environment is an owned ARC reference released here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_closure_destroy(closure: *mut c_void) {
    if closure.is_null() {
        return;
    }
    let parts = closure as *mut *mut c_void;
    unsafe {
        lpp_arc_release(*parts.add(1));
    }
}

/// Observability hook for the differential gates: how many ARC drops have
/// run in this process. Not part of the v1 ABI (the C reference counts
/// destructors through its own test shim), so the name is internal.
#[unsafe(no_mangle)]
pub static lpp__runtime_drop_count: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The v1 C reference produces this exact fingerprint for the ARC
    /// scenario (pinned in `tests/runtime_gate.rs`). Bool fields are printed
    /// as 1/0 to match C's `%d`, so the two runtimes are byte-comparable.
    const GOLDEN: &str = "A genpos=1 live=1 drops=1 dead=1 B drops=1 emptypos=1";

    static DTOR_COUNT: AtomicUsize = AtomicUsize::new(0);

    /// The destructor-count tests share one process-global counter, so they
    /// must not run concurrently. Each grabs this lock for its duration.
    static SERIAL: Mutex<()> = Mutex::new(());

    unsafe extern "C" fn test_destructor(_payload: *mut c_void) {
        DTOR_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    fn b2i(b: bool) -> i32 {
        i32::from(b)
    }

    fn scenario_fingerprint() -> String {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        DTOR_COUNT.store(0, Ordering::Relaxed);
        unsafe {
            let a = lpp_arc_alloc_with_destructor(32, Some(test_destructor));
            let ga = lpp_weak_generation(a);
            let live = lpp_weak_get(a as i64, ga) == a as i64;
            let mut out = format!("A genpos={} live={} ", b2i(ga > 0), b2i(live));
            lpp_arc_retain(a);
            lpp_arc_release(a);
            lpp_arc_release(a);
            let dead = lpp_weak_get(a as i64, ga) == 0;
            out.push_str(&format!(
                "drops={} dead={} ",
                DTOR_COUNT.load(Ordering::Relaxed),
                b2i(dead)
            ));
            let b = lpp_arc_alloc(16);
            lpp_arc_retain_local(b);
            lpp_arc_release_local(b);
            lpp_arc_release_local(b);
            let e = lpp_empty_str();
            lpp_arc_retain(e);
            lpp_arc_release(e);
            let epos = lpp_weak_generation(e) == 0x4152_4331_i64;
            out.push_str(&format!(
                "B drops={} emptypos={}",
                DTOR_COUNT.load(Ordering::Relaxed),
                b2i(epos)
            ));
            out
        }
    }

    #[test]
    fn scenario_fingerprint_is_golden() {
        assert_eq!(
            scenario_fingerprint(),
            GOLDEN,
            "Rust runtime diverges from the C reference"
        );
    }

    #[test]
    fn arc_retain_release_accounting() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        DTOR_COUNT.store(0, Ordering::Relaxed);
        unsafe {
            let p = lpp_arc_alloc_with_destructor(32, Some(test_destructor));
            assert!(!p.is_null());
            let g = lpp_weak_generation(p);
            assert!(g > 0, "fresh object must have a nonzero generation");
            assert_eq!(lpp_weak_get(p as i64, g), p as i64, "live weak handle resolves");

            // rc 1 -> 3 -> 2 -> 1 -> 0: destructor fires exactly once, at the end.
            lpp_arc_retain(p);
            lpp_arc_retain(p);
            lpp_arc_release(p);
            assert_eq!(DTOR_COUNT.load(Ordering::Relaxed), 0, "alive at rc 2");
            lpp_arc_release(p);
            assert_eq!(DTOR_COUNT.load(Ordering::Relaxed), 0, "alive at rc 1");
            lpp_arc_release(p);
            assert_eq!(DTOR_COUNT.load(Ordering::Relaxed), 1, "freed at rc 0");

            // A stale weak handle must not resolve (generation bumped at free).
            assert_eq!(lpp_weak_get(p as i64, g), 0, "stale weak handle rejects");
        }
    }

    #[test]
    fn arc_local_fast_path_matches() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        DTOR_COUNT.store(0, Ordering::Relaxed);
        unsafe {
            let p = lpp_arc_alloc_with_destructor(16, Some(test_destructor));
            lpp_arc_retain_local(p);
            lpp_arc_retain_local(p);
            lpp_arc_release_local(p);
            assert_eq!(DTOR_COUNT.load(Ordering::Relaxed), 0, "alive at rc 2");
            lpp_arc_release_local(p);
            lpp_arc_release_local(p);
            assert_eq!(DTOR_COUNT.load(Ordering::Relaxed), 1, "freed at rc 0");
        }
    }

    #[test]
    fn foreign_and_null_pointers_are_tolerated() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        DTOR_COUNT.store(0, Ordering::Relaxed);
        unsafe {
            lpp_arc_retain(std::ptr::null_mut());
            lpp_arc_release(std::ptr::null_mut());
            assert_eq!(lpp_weak_generation(std::ptr::null_mut()), 0);
            assert_eq!(lpp_weak_get(0, 1), 0);
            assert_eq!(lpp_weak_get(1, 0), 0);

            // A stack buffer is 8-aligned but has no header: must be rejected.
            let mut buf = [0u8; 64];
            let p = buf.as_mut_ptr().cast::<c_void>();
            lpp_arc_retain(p);
            lpp_arc_release(p);
            assert_eq!(DTOR_COUNT.load(Ordering::Relaxed), 0);
            assert_eq!(lpp_weak_generation(p), 0);
        }
    }

    #[test]
    fn empty_str_is_immortal_and_valid() {
        unsafe {
            let e = lpp_empty_str();
            assert!(!e.is_null());
            let hdr = e.cast::<u8>().sub(24);
            assert_eq!(*(hdr as *const u32), 0x4152_4331, "magic at offset 0");
            assert_eq!(*(hdr.add(4) as *const u32), 0x4152_4331, "immortal at offset 4");
            lpp_arc_retain(e);
            lpp_arc_release(e);
            assert_eq!(lpp_weak_generation(e), 0x4152_4331_i64);
            assert_eq!(lpp_weak_get(e as i64, 0x4152_4331_i64), e as i64);
            assert_eq!(lpp_weak_get(e as i64, 42), 0, "wrong generation rejects");
        }
    }

    #[test]
    fn closure_destroy_releases_the_environment() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        DTOR_COUNT.store(0, Ordering::Relaxed);
        unsafe {
            let env = lpp_arc_alloc_with_destructor(8, Some(test_destructor));
            let code = 0x1234usize as *mut c_void;
            let closure = Box::into_raw(Box::new([code, env]));
            lpp_closure_destroy(closure as *mut c_void);
            assert_eq!(DTOR_COUNT.load(Ordering::Relaxed), 1, "environment released");
            std::mem::drop(Box::from_raw(closure));
        }
    }
}
