// L++ Allocator runtime stubs.

/// Allocate `size` bytes on the L++ managed heap (ARC-prefixed block).
/// Returns a pointer to the usable region (after the ARC header).
extern "C" {
    fn calloc(count: usize, size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
}

#[no_mangle]
pub extern "C" fn lpp_alloc(size: usize) -> *mut u8 {
    let header_size = 24;
    let ptr = unsafe { calloc(1, header_size + size) };
    if ptr.is_null() {
        return ptr;
    }
    // Set magic and initial refcount (not strictly needed for just memory leak fix, but good)
    unsafe {
        let magic_ptr = ptr as *mut u32;
        magic_ptr.write(0x41524331);
        let refcount_ptr = ptr.add(4) as *mut std::sync::atomic::AtomicU32;
        (*refcount_ptr).store(1, std::sync::atomic::Ordering::SeqCst);
        ptr.add(header_size)
    }
}

#[no_mangle]
pub unsafe extern "C" fn lpp_free(ptr: *mut u8, _size: usize) {
    if ptr.is_null() {
        return;
    }
    let header_size = 24;
    let base_ptr = ptr.sub(header_size);
    free(base_ptr);
}
