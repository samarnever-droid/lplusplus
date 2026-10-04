//! The structural tuple runtime (Phase 6B.3d).
//!
//! Rust re-implementation of the `lpp_runtime.c` tuple primitive with an
//! identical ABI and layout. A tuple is an ARC-managed object whose payload
//! begins with the `LppTuplePrefix` below; up to four managed child slots are
//! described by a 4-bit `managed_mask` and four 16-bit byte offsets packed
//! into `packed_offsets`. The ARC destructor releases each managed child.

use libc::c_void;

use crate::arc::{lpp_arc_alloc_with_destructor, lpp_arc_release};
use crate::panic::runtime_panic;

/// The tuple header. `#[repr(C)]` + field order match the C `LppTuplePrefix`
/// exactly (managed_mask@0, packed_offsets@8; size 16).
#[repr(C)]
pub struct LppTuplePrefix {
    managed_mask: u64,
    packed_offsets: u64,
}

/// The ARC destructor: release each managed child slot. Mirrors C
/// `lpp_tuple_destroy` (4 slots, 16-bit offsets, mask bit per slot).
unsafe extern "C" fn tuple_destroy(payload: *mut c_void) {
    if payload.is_null() {
        return;
    }
    let tuple = unsafe { &*(payload as *mut LppTuplePrefix) };
    for i in 0..4u32 {
        if tuple.managed_mask & (1u64 << i) == 0 {
            continue;
        }
        let offset = ((tuple.packed_offsets >> (i * 16)) & 0xffff) as usize;
        let slot = unsafe { payload.cast::<u8>().add(offset) } as *mut *mut c_void;
        let child = unsafe { *slot };
        unsafe { lpp_arc_release(child) };
    }
}

/// Allocate a tuple of `size` bytes (>= the 16-byte prefix) with the given
/// managed mask and packed offsets.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_tuple_alloc(
    size: i64,
    managed_mask: i64,
    packed_offsets: i64,
) -> *mut c_void {
    let prefix = std::mem::size_of::<LppTuplePrefix>() as i64;
    if size < prefix {
        runtime_panic(&format!("invalid tuple allocation size: {size}"));
    }
    let raw = unsafe { lpp_arc_alloc_with_destructor(size, Some(tuple_destroy)) };
    if raw.is_null() {
        runtime_panic("out of memory while allocating tuple");
    }
    let tuple = unsafe { &mut *(raw as *mut LppTuplePrefix) };
    tuple.managed_mask = managed_mask as u64;
    tuple.packed_offsets = packed_offsets as u64;
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_layout_matches_c() {
        assert_eq!(std::mem::size_of::<LppTuplePrefix>(), 16, "prefix size");
    }
}
