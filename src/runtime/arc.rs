// L++ ARC runtime stubs.
// In the final linker pass these will be proper reference-counted
// allocation helpers. For MVP they are no-ops so the build succeeds.

/// Increment the reference count of a managed heap allocation.
/// # Safety
/// `ptr` must point to an L++ managed object with an ARC header.
#[no_mangle]
pub extern "C" fn lpp_arc_retain(ptr: *mut u8) {
    // TODO: atomically increment refcount at *(ptr - header_offset)
    let _ = ptr;
}

/// Decrement the reference count; free if it reaches zero.
/// # Safety
/// `ptr` must point to an L++ managed object with an ARC header.
#[no_mangle]
pub extern "C" fn lpp_arc_release(ptr: *mut u8) {
    // TODO: atomically decrement refcount; if zero, call destructor and free
    let _ = ptr;
}
