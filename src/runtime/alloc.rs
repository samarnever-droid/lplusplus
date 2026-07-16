// L++ Allocator runtime stubs.

/// Allocate `size` bytes on the L++ managed heap (ARC-prefixed block).
/// Returns a pointer to the usable region (after the ARC header).
#[no_mangle]
pub extern "C" fn lpp_alloc(size: usize) -> *mut u8 {
    // TODO: allocate ARC header + data block; return pointer past header
    let layout = std::alloc::Layout::from_size_align(size, 8).unwrap();
    unsafe { std::alloc::alloc_zeroed(layout) }
}

/// Free a previously allocated managed heap block.
/// # Safety
/// `ptr` must have been returned by `lpp_alloc`.
#[no_mangle]
pub unsafe extern "C" fn lpp_free(ptr: *mut u8, size: usize) {
    let layout = std::alloc::Layout::from_size_align(size, 8).unwrap();
    std::alloc::dealloc(ptr, layout);
}
