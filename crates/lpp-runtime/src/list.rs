//! The `List[T]` runtime (Phase 6B.2).
//!
//! Rust re-implementation of the `lpp_runtime.c` list primitives with an
//! identical ABI and identical layout. A list is an ARC-managed object whose
//! payload is the `LppList` struct below; the ARC destructor frees the inner
//! `data` array (and drops owned elements) while the ARC free releases the
//! outer header+payload block — exactly the C ownership split.
//!
//! `List[Int]` stores raw `i64` values and owns no element references
//! (`retain_element`/`drop_element` are NULL). `List[ARC Object]` stores
//! pointers as `i64` and owns one retained reference per element. Floats and
//! bools are bit-packed into the same `i64` slots. The v1 C runtime remains
//! the behavioral reference; `tests/runtime_gate.rs` diffs a list/slice
//! scenario fingerprint against it.

use libc::{c_void, free, realloc};

use crate::arc::{lpp_arc_alloc_with_destructor, lpp_arc_release, lpp_arc_retain};
use crate::panic::runtime_panic;

/// Element retain/drop callback: takes the raw `i64` slot value.
type ElementFn = Option<unsafe extern "C" fn(i64)>;

/// The list payload. `#[repr(C)]` + field order match the C `LppList`
/// exactly (data@0, len@8, cap@16, retain@24, drop@32; size 40).
#[repr(C)]
pub struct LppList {
    data: *mut i64,
    len: i64,
    cap: i64,
    /// NULL for value elements; retain/drop callbacks for ARC pointer
    /// elements.
    retain_element: ElementFn,
    drop_element: ElementFn,
}

/// The ARC destructor for a list: drop owned elements, free the data array.
/// Does NOT free the `LppList` itself — the ARC free releases the whole
/// header+payload block (matching C `lpp_list_destroy`).
unsafe extern "C" fn list_destroy(payload: *mut c_void) {
    if payload.is_null() {
        return;
    }
    let l = unsafe { &mut *(payload as *mut LppList) };
    if let Some(drop) = l.drop_element {
        for i in 0..l.len {
            unsafe { drop(*l.data.add(i as usize)) };
        }
    }
    if !l.data.is_null() {
        unsafe { free(l.data as *mut c_void) };
    }
    l.data = std::ptr::null_mut();
    l.len = 0;
    l.cap = 0;
}

/// Retain one ARC element, skipping raw/static pointers (string literals,
/// small integers) that carry no ARC header. Mirrors C
/// `lpp_list_arc_retain_element` (8-aligned, >= 0x1000).
unsafe extern "C" fn arc_retain_element(value: i64) {
    let ptr = value as usize as *mut c_void;
    if value == 0 || value as usize % 8 != 0 || (value as usize) < 0x1000 {
        return;
    }
    unsafe { lpp_arc_retain(ptr) };
}

/// Release one ARC element, with the same raw-pointer skip.
unsafe extern "C" fn arc_drop_element(value: i64) {
    if value == 0 || value as usize % 8 != 0 || (value as usize) < 0x1000 {
        return;
    }
    unsafe { lpp_arc_release(value as usize as *mut c_void) };
}

unsafe fn new_with_ownership(retain: ElementFn, drop: ElementFn) -> *mut c_void {
    let size = std::mem::size_of::<LppList>() as i64;
    let raw = unsafe { lpp_arc_alloc_with_destructor(size, Some(list_destroy)) };
    if raw.is_null() {
        runtime_panic("out of memory while creating list");
    }
    let l = unsafe { &mut *(raw as *mut LppList) };
    l.data = std::ptr::null_mut();
    l.len = 0;
    l.cap = 0;
    l.retain_element = retain;
    l.drop_element = drop;
    raw
}

/// `List[Int]`: stores values, owns no element references.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_new() -> *mut c_void {
    unsafe { new_with_ownership(None, None) }
}

/// `List[ARC Object]`: owns one retained reference per element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_new_arc() -> *mut c_void {
    unsafe { new_with_ownership(Some(arc_retain_element), Some(arc_drop_element)) }
}

/// Grow the data array to hold at least one more element (C's doubling:
/// 0 -> 8 -> 16 -> …, with overflow guards).
unsafe fn grow(l: &mut LppList) {
    if l.len < l.cap {
        return;
    }
    if l.cap > i64::MAX / 2 {
        runtime_panic("list capacity overflow");
    }
    let new_cap = if l.cap == 0 { 8 } else { l.cap * 2 };
    if new_cap > i64::MAX / 8 {
        runtime_panic("list allocation size overflow");
    }
    let new_data =
        unsafe { realloc(l.data as *mut c_void, (new_cap as usize) * 8) } as *mut i64;
    if new_data.is_null() {
        runtime_panic("out of memory while growing list");
    }
    l.data = new_data;
    l.cap = new_cap;
}

/// Append a raw `i64` value (retaining first if this list owns elements).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_push(list: *mut c_void, value: i64) {
    if list.is_null() {
        runtime_panic("push attempted on null list pointer");
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    unsafe { grow(l) };
    if let Some(retain) = l.retain_element {
        unsafe { retain(value) };
    }
    unsafe {
        *l.data.add(l.len as usize) = value;
    }
    l.len += 1;
}

/// Store one ARC object reference in `List[T]` (promotes a value list to an
/// owning list on first use, exactly as C does).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_push_arc(list: *mut c_void, value: *mut c_void) {
    if !list.is_null() {
        let l = unsafe { &mut *(list as *mut LppList) };
        if l.retain_element.is_none() {
            l.retain_element = Some(arc_retain_element);
            l.drop_element = Some(arc_drop_element);
        }
    }
    unsafe { lpp_list_push(list, value as i64) };
}

/// Append a float (bit-packed into the `i64` slot).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_push_float(list: *mut c_void, value: f64) {
    unsafe { lpp_list_push(list, value.to_bits() as i64) };
}

/// Append a bool (stored as 1/0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_push_bool(list: *mut c_void, value: i8) {
    unsafe { lpp_list_push(list, i64::from(value != 0)) };
}

/// Read the raw `i64` at `index` (bounds-checked; panics like C on OOB).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_get(list: *mut c_void, index: i64) -> i64 {
    if list.is_null() {
        runtime_panic("list index access attempted on null list pointer");
    }
    let l = unsafe { &*(list as *mut LppList) };
    if index < 0 || index >= l.len {
        runtime_panic(&format!(
            "list index out of bounds: index {index}, len {}",
            l.len
        ));
    }
    unsafe { *l.data.add(index as usize) }
}

/// Read a float at `index` (bit-unpacked).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_get_float(list: *mut c_void, index: i64) -> f64 {
    f64::from_bits(unsafe { lpp_list_get(list, index) } as u64)
}

/// Read a bool at `index` (nonzero = true).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_get_bool(list: *mut c_void, index: i64) -> i8 {
    i8::from(unsafe { lpp_list_get(list, index) } != 0)
}

/// Read an ARC object reference at `index` (borrowed; caller retains only
/// when creating an additional owner).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_get_arc(list: *mut c_void, index: i64) -> *mut c_void {
    unsafe { lpp_list_get(list, index) as usize as *mut c_void }
}

/// Overwrite the slot at `index`. Retains the incoming edge BEFORE dropping
/// the old one — required for self-assignment such as
/// `set(xs, 0, get(xs, 0))` where the list may hold the last reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_set(list: *mut c_void, index: i64, value: i64) {
    if list.is_null() {
        runtime_panic("list set attempted on null list pointer");
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    if index < 0 || index >= l.len {
        runtime_panic(&format!(
            "list index out of bounds on set: index {index}, len {}",
            l.len
        ));
    }
    if let Some(retain) = l.retain_element {
        if value != 0 {
            unsafe { retain(value) };
        }
    }
    let old = unsafe { *l.data.add(index as usize) };
    if let Some(drop) = l.drop_element {
        if old != 0 {
            unsafe { drop(old) };
        }
    }
    unsafe { *l.data.add(index as usize) = value };
}

/// Overwrite with a bool.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_set_bool(list: *mut c_void, index: i64, value: i8) {
    unsafe { lpp_list_set(list, index, i64::from(value != 0)) };
}

/// Overwrite with a float.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_set_float(list: *mut c_void, index: i64, value: f64) {
    unsafe { lpp_list_set(list, index, value.to_bits() as i64) };
}

/// Overwrite with an ARC object reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_set_arc(list: *mut c_void, index: i64, value: *mut c_void) {
    unsafe { lpp_list_set(list, index, value as i64) };
}

/// The number of elements (0 for a null list).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_len(list: *mut c_void) -> i64 {
    if list.is_null() {
        return 0;
    }
    let l = unsafe { &*(list as *mut LppList) };
    l.len
}

/// Pop the last element (panics on empty/null, like C).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_pop(list: *mut c_void) -> i64 {
    if list.is_null() {
        runtime_panic("list pop attempted on empty or null list");
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    if l.len <= 0 {
        runtime_panic("list pop attempted on empty or null list");
    }
    l.len -= 1;
    unsafe { *l.data.add(l.len as usize) }
}

/// Compatibility entry point: a single ARC reference release (list lifetime
/// is automatic in ownership-aware AOT code, so this is never a raw free).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_free(list: *mut c_void) {
    unsafe { lpp_arc_release(list) };
}

/// Reserve capacity (no-op if already large enough).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_reserve(list: *mut c_void, capacity: i64) {
    if list.is_null() {
        return;
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    if capacity <= l.cap {
        return;
    }
    let new_data =
        unsafe { realloc(l.data as *mut c_void, (capacity as usize) * 8) } as *mut i64;
    if new_data.is_null() {
        runtime_panic("out of memory in list_reserve");
    }
    l.data = new_data;
    l.cap = capacity;
}

/// The current capacity (0 for a null list).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_capacity(list: *mut c_void) -> i64 {
    if list.is_null() {
        return 0;
    }
    let l = unsafe { &*(list as *mut LppList) };
    l.cap
}

/// Drop all elements (releasing owned references) and set len to 0, keeping
/// the allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_clear(list: *mut c_void) {
    if list.is_null() {
        return;
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    if let Some(drop) = l.drop_element {
        for i in 0..l.len {
            unsafe { drop(*l.data.add(i as usize)) };
        }
    }
    l.len = 0;
}

// ── List operations + sorting/search (E5003 list slice) ─────────────────────
//
// Every slot is a raw `i64` (values for `List[Int]`; ARC pointers for owning
// lists; bit-packed for float/bool). These ops therefore operate on the `i64`
// slots directly and route element ownership through the list's
// `retain_element`/`drop_element` callbacks, exactly like push/set/clear. The
// sort/search family orders the raw slots, which is the intended integer order
// for `List[Int]` (the only element class the corpus sorts).

/// Borrow the live slots as a mutable slice (empty when the list is empty or
/// its backing array has not been allocated yet).
unsafe fn slots_mut<'a>(l: &'a mut LppList) -> &'a mut [i64] {
    if l.data.is_null() || l.len <= 0 {
        // A zero-length slice still needs a non-null, aligned base pointer.
        return unsafe {
            std::slice::from_raw_parts_mut(std::ptr::NonNull::<i64>::dangling().as_ptr(), 0)
        };
    }
    unsafe { std::slice::from_raw_parts_mut(l.data, l.len as usize) }
}

/// `list_insert(list, index, value)` — insert `value` before `index`, shifting
/// the tail right. `index` is clamped to `[0, len]`. Owning lists retain the
/// inserted edge (like `push`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_insert(list: *mut c_void, index: i64, value: i64) {
    if list.is_null() {
        runtime_panic("list insert attempted on null list pointer");
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    let at = index.clamp(0, l.len);
    unsafe { grow(l) };
    let tail = (l.len - at) as usize;
    if tail > 0 {
        unsafe {
            std::ptr::copy(
                l.data.add(at as usize),
                l.data.add(at as usize + 1),
                tail,
            );
        }
    }
    if let Some(retain) = l.retain_element {
        unsafe { retain(value) };
    }
    unsafe {
        *l.data.add(at as usize) = value;
    }
    l.len += 1;
}

/// `list_remove(list, index) -> value` — remove and return the slot at `index`,
/// shifting the tail left. The reference (if any) is **transferred** to the
/// caller, not dropped. Panics on an out-of-bounds index (like `get`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_remove(list: *mut c_void, index: i64) -> i64 {
    if list.is_null() {
        runtime_panic("list remove attempted on null list pointer");
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    if index < 0 || index >= l.len {
        runtime_panic(&format!(
            "list index out of bounds on remove: index {index}, len {}",
            l.len
        ));
    }
    let removed = unsafe { *l.data.add(index as usize) };
    let tail = (l.len - index - 1) as usize;
    if tail > 0 {
        unsafe {
            std::ptr::copy(
                l.data.add(index as usize + 1),
                l.data.add(index as usize),
                tail,
            );
        }
    }
    l.len -= 1;
    removed
}

/// `list_swap(list, i, j)` — swap two slots in place. Panics on OOB.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_swap(list: *mut c_void, i: i64, j: i64) {
    if list.is_null() {
        runtime_panic("list swap attempted on null list pointer");
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    if i < 0 || i >= l.len || j < 0 || j >= l.len {
        runtime_panic(&format!(
            "list index out of bounds on swap: i {i}, j {j}, len {}",
            l.len
        ));
    }
    if i != j {
        unsafe {
            let a = *l.data.add(i as usize);
            *l.data.add(i as usize) = *l.data.add(j as usize);
            *l.data.add(j as usize) = a;
        }
    }
}

/// `list_reverse(list)` — reverse the slots in place.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_reverse(list: *mut c_void) {
    if list.is_null() {
        return;
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    unsafe { slots_mut(l) }.reverse();
}

/// `list_truncate(list, len)` — shrink to `len` elements, dropping owned
/// references beyond it. A `len >= current` (or negative) is a no-op / full
/// clear respectively.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_truncate(list: *mut c_void, len: i64) {
    if list.is_null() {
        return;
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    let keep = len.max(0);
    if keep >= l.len {
        return;
    }
    if let Some(drop) = l.drop_element {
        for i in keep..l.len {
            unsafe { drop(*l.data.add(i as usize)) };
        }
    }
    l.len = keep;
}

/// `list_sort(list)` — ascending signed order of the raw slots.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_sort(list: *mut c_void) {
    if list.is_null() {
        return;
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    unsafe { slots_mut(l) }.sort_unstable();
}

/// `list_sort_desc(list)` — descending signed order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_sort_desc(list: *mut c_void) {
    if list.is_null() {
        return;
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    unsafe { slots_mut(l) }.sort_unstable_by(|a, b| b.cmp(a));
}

/// `list_sort_u(list)` — ascending *unsigned* order (each slot compared as
/// `u64`, so negative two's-complement values sort after all non-negatives).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_sort_u(list: *mut c_void) {
    if list.is_null() {
        return;
    }
    let l = unsafe { &mut *(list as *mut LppList) };
    unsafe { slots_mut(l) }.sort_unstable_by(|a, b| (*a as u64).cmp(&(*b as u64)));
}

/// `list_index_of(list, key) -> Int` — first slot equal to `key`, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_index_of(list: *mut c_void, key: i64) -> i64 {
    if list.is_null() {
        return -1;
    }
    let l = unsafe { &*(list as *mut LppList) };
    for i in 0..l.len {
        if unsafe { *l.data.add(i as usize) } == key {
            return i;
        }
    }
    -1
}

/// `list_binary_search(list, key) -> Int` — search an ascending-sorted list.
/// Returns the match index, or `-(insertion_point + 1)` when absent (the Java/
/// .NET convention the corpus asserts: a miss with insertion point 5 → -6).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_binary_search(list: *mut c_void, key: i64) -> i64 {
    if list.is_null() {
        return -1;
    }
    let l = unsafe { &*(list as *mut LppList) };
    let mut lo = 0i64;
    let mut hi = l.len; // exclusive
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let v = unsafe { *l.data.add(mid as usize) };
        if v == key {
            return mid;
        } else if v < key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    -(lo + 1)
}

/// `list_extend(dst, src)` — append every element of `src` to `dst`. Owning
/// destinations retain each appended edge (via `push`). Safe when `dst == src`
/// (the source slots are snapshotted before any growth).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_list_extend(dst: *mut c_void, src: *mut c_void) {
    if dst.is_null() || src.is_null() {
        return;
    }
    let s = unsafe { &*(src as *mut LppList) };
    let snapshot: Vec<i64> = (0..s.len)
        .map(|i| unsafe { *s.data.add(i as usize) })
        .collect();
    for value in snapshot {
        unsafe { lpp_list_push(dst, value) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc::{lpp_arc_alloc_with_destructor, lpp_arc_release};
    use crate::slice::{lpp_slice_get, lpp_slice_init, lpp_slice_len};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// The v1 C reference produces this exact fingerprint for the
    /// list/slice/ARC-list scenario (pinned in
    /// `tests/runtime_gate.rs::c_reference_matches_the_list_slice_golden`).
    const GOLDEN: &str = "L len=3 g0=10 g2=30 set1=99 f3=1 b4=1 pop=1 len=4 cap=1 clear=0 S slen=3 sg0=2 sg2=4 A alive=0 dropped=1";

    static DROPS: AtomicUsize = AtomicUsize::new(0);
    static SERIAL: Mutex<()> = Mutex::new(());

    unsafe extern "C" fn dtor(_payload: *mut c_void) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }

    fn b2i(b: bool) -> i32 {
        i32::from(b)
    }

    fn scenario() -> String {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        DROPS.store(0, Ordering::Relaxed);
        unsafe {
            let mut out = String::new();
            // Value list: push / get / set / float / bool / pop / reserve / clear.
            let xs = lpp_list_new();
            lpp_list_push(xs, 10);
            lpp_list_push(xs, 20);
            lpp_list_push(xs, 30);
            out.push_str(&format!(
                "L len={} g0={} g2={} ",
                lpp_list_len(xs),
                lpp_list_get(xs, 0),
                lpp_list_get(xs, 2)
            ));
            lpp_list_set(xs, 1, 99);
            lpp_list_push_float(xs, 3.5);
            lpp_list_push_bool(xs, 1);
            let fok = lpp_list_get_float(xs, 3) == 3.5;
            let bok = lpp_list_get_bool(xs, 4) != 0;
            out.push_str(&format!(
                "set1={} f3={} b4={} ",
                lpp_list_get(xs, 1),
                b2i(fok),
                b2i(bok)
            ));
            let popped = lpp_list_pop(xs);
            out.push_str(&format!("pop={} len={} ", popped, lpp_list_len(xs)));
            lpp_list_reserve(xs, 100);
            let cok = lpp_list_capacity(xs) >= 100;
            lpp_list_clear(xs);
            out.push_str(&format!("cap={} clear={} ", b2i(cok), lpp_list_len(xs)));
            lpp_list_free(xs);

            // Slice over a list: start=1, len=3, kind=1.
            let ys = lpp_list_new();
            for i in 1..=5 {
                lpp_list_push(ys, i);
            }
            let mut storage = [0u64; 8]; // 64 bytes, 8-aligned (LppSlice is 40)
            let sl = lpp_slice_init(storage.as_mut_ptr().cast::<c_void>(), ys, 1, 3, 1);
            out.push_str(&format!(
                "S slen={} sg0={} sg2={} ",
                lpp_slice_len(sl),
                lpp_slice_get(sl, 0),
                lpp_slice_get(sl, 2)
            ));
            lpp_list_free(ys);

            // ARC-owning list: the list holds the only reference after the
            // local release; freeing the list drops the element exactly once.
            let al = lpp_list_new_arc();
            let obj = lpp_arc_alloc_with_destructor(8, Some(dtor));
            lpp_list_push_arc(al, obj);
            lpp_arc_release(obj);
            let alive = DROPS.load(Ordering::Relaxed);
            lpp_list_free(al);
            out.push_str(&format!(
                "A alive={} dropped={}",
                alive,
                DROPS.load(Ordering::Relaxed)
            ));
            out
        }
    }

    #[test]
    fn list_slice_scenario_is_golden() {
        assert_eq!(
            scenario(),
            GOLDEN,
            "Rust list/slice runtime diverges from the C reference"
        );
    }
}
