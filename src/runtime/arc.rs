// L++ ARC runtime stubs.
// In the final linker pass these will be proper reference-counted
// allocation helpers. For MVP they are no-ops so the build succeeds.

/// Increment the reference count of a managed heap allocation.
/// # Safety
/// `ptr` must point to an L++ managed object with an ARC header.
extern "C" {
    fn free(ptr: *mut u8);
}

#[no_mangle]
pub extern "C" fn lpp_arc_retain(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let header_ptr = ptr.sub(24);
        let refcount_ptr = header_ptr.add(4) as *mut std::sync::atomic::AtomicU32;
        (*refcount_ptr).fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[no_mangle]
pub extern "C" fn lpp_arc_release(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let header_ptr = ptr.sub(24);
        let refcount_ptr = header_ptr.add(4) as *mut std::sync::atomic::AtomicU32;
        if (*refcount_ptr).fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            // Reached zero. Free it.
            free(header_ptr);
        }
    }
}

#[no_mangle]
pub extern "C" fn lpp_arc_retain_local(ptr: *mut u8) {
    lpp_arc_retain(ptr);
}

#[no_mangle]
pub extern "C" fn lpp_arc_release_local(ptr: *mut u8) {
    lpp_arc_release(ptr);
}
