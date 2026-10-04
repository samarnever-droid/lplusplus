//! The `Slice[T]` / `StrSlice` runtime (Phase 6B.2).
//!
//! Rust re-implementation of the `lpp_runtime.c` slice primitives with an
//! identical ABI and layout. A slice view is a borrowed window over an
//! ARC-managed source (a UTF-8 string for `kind == 0`, a `List[T]` for
//! `kind == 1`); it stores the source's weak generation so a read after the
//! source dies is caught instead of dereferencing freed memory. The view
//! itself is plain caller-provided storage (not ARC-managed) — exactly the C
//! contract, where `lpp_slice_init` fills a `storage` block the compiler
//! reserved.

use libc::{c_char, c_void, strlen};

use crate::arc::{lpp_arc_alloc, lpp_weak_generation, lpp_weak_get};
use crate::list::{lpp_list_get, lpp_list_len};
use crate::panic::runtime_panic;

/// The slice view. `#[repr(C)]` + field order match the C `LppSlice`
/// exactly (base@0, start@8, length@16, generation@24, kind@32; size 40).
#[repr(C)]
pub struct LppSlice {
    base: *mut c_void,
    start: i64,
    length: i64,
    generation: i64,
    /// 0 = UTF-8 byte string, 1 = `List[T]` slots.
    kind: i64,
}

/// Validate the view and resolve its (still-live) base pointer, or panic.
/// Mirrors C `lpp_slice_checked_base`: the weak generation must still match,
/// else the borrowed source is gone.
unsafe fn checked_base(view: *const LppSlice) -> *mut c_void {
    if view.is_null() {
        runtime_panic("use of an uninitialized slice view");
    }
    let v = unsafe { &*view };
    if v.base.is_null() || v.generation == 0 {
        runtime_panic("use of an uninitialized slice view");
    }
    let raw = unsafe { lpp_weak_get(v.base as i64, v.generation) };
    if raw == 0 {
        runtime_panic("borrowed slice source is no longer live");
    }
    raw as usize as *mut c_void
}

/// Build a slice view over `base` (a string or a list) into caller-provided
/// `storage`. Range-checked against the source length; the source must be
/// ARC-managed (nonzero weak generation).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_slice_init(
    storage: *mut c_void,
    base: *mut c_void,
    start: i64,
    length: i64,
    kind: i64,
) -> *mut c_void {
    if storage.is_null() || base.is_null() {
        runtime_panic("slice construction requires live storage and base");
    }
    if start < 0 || length < 0 || start > i64::MAX - length {
        runtime_panic(&format!("invalid slice range: start {start}, len {length}"));
    }
    let source_length = if kind == 0 {
        unsafe { strlen(base as *const libc::c_char) as i64 }
    } else {
        unsafe { lpp_list_len(base) }
    };
    if start > source_length || length > source_length - start {
        runtime_panic(&format!(
            "slice range out of bounds: start {start}, len {length}, source len {source_length}"
        ));
    }
    let view = unsafe { &mut *(storage as *mut LppSlice) };
    view.base = base;
    view.start = start;
    view.length = length;
    view.generation = unsafe { lpp_weak_generation(base) };
    view.kind = kind;
    if view.generation == 0 {
        runtime_panic("slice source is not an ARC-managed value");
    }
    storage
}

/// The slice length (validates the source is still live first).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_slice_len(raw_view: *mut c_void) -> i64 {
    let view = raw_view as *const LppSlice;
    unsafe { checked_base(view) };
    unsafe { (*view).length }
}

/// Read the raw `i64` at `index` from a numeric (`kind == 1`) slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_slice_get(raw_view: *mut c_void, index: i64) -> i64 {
    let view = raw_view as *const LppSlice;
    let base = unsafe { checked_base(view) };
    let v = unsafe { &*view };
    if index < 0 || index >= v.length {
        runtime_panic(&format!(
            "slice index out of bounds: index {index}, len {}",
            v.length
        ));
    }
    if v.kind != 1 {
        runtime_panic("numeric slice_get requires Slice[T]");
    }
    unsafe { lpp_list_get(base, v.start + index) }
}

/// Read a float at `index` (bit-unpacked).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_slice_get_float(raw_view: *mut c_void, index: i64) -> f64 {
    f64::from_bits(unsafe { lpp_slice_get(raw_view, index) } as u64)
}

/// Read a bool at `index` (nonzero = true).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_slice_get_bool(raw_view: *mut c_void, index: i64) -> i8 {
    i8::from(unsafe { lpp_slice_get(raw_view, index) } != 0)
}

/// Read one byte at `index` from a string (`kind == 0`) slice as a fresh
/// 1-character ARC string. Mirrors C `lpp_str_slice_get`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_slice_get(raw_view: *mut c_void, index: i64) -> *mut c_char {
    let view = raw_view as *const LppSlice;
    let base = unsafe { checked_base(view) } as *const u8;
    let v = unsafe { &*view };
    if v.kind != 0 {
        runtime_panic("string slice_get requires StrSlice");
    }
    if index < 0 || index >= v.length {
        runtime_panic(&format!(
            "string slice index out of bounds: index {index}, len {}",
            v.length
        ));
    }
    let result = unsafe { lpp_arc_alloc(2) } as *mut u8;
    if result.is_null() {
        runtime_panic("out of memory while reading string slice");
    }
    unsafe {
        *result = *base.add((v.start + index) as usize);
        *result.add(1) = 0;
    }
    result as *mut c_char
}

/// Copy a string (`kind == 0`) slice into a fresh NUL-terminated ARC string.
/// Mirrors C `lpp_str_slice_to_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_slice_to_str(raw_view: *mut c_void) -> *mut c_char {
    let view = raw_view as *const LppSlice;
    let base = unsafe { checked_base(view) } as *const u8;
    let v = unsafe { &*view };
    if v.kind != 0 {
        runtime_panic("slice_to_str requires StrSlice");
    }
    let result = unsafe { lpp_arc_alloc(v.length + 1) } as *mut u8;
    if result.is_null() {
        runtime_panic("out of memory while copying string slice");
    }
    unsafe {
        std::ptr::copy_nonoverlapping(base.add(v.start as usize), result, v.length as usize);
        *result.add(v.length as usize) = 0;
    }
    result as *mut c_char
}
