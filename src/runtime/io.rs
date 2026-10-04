// L++ I/O runtime stubs.

/// Print an integer to stdout.
#[no_mangle]
pub extern "C" fn lpp_print_int(value: i64) {
    println!("{}", value);
}

/// Print a null-terminated string to stdout.
/// # Safety
/// `ptr` must be a valid null-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lpp_print_str(ptr: *const u8) {
    if ptr.is_null() { return; }
    let c_str = std::ffi::CStr::from_ptr(ptr as *const i8);
    println!("{}", c_str.to_string_lossy());
}

/// Read a line from stdin. Returns a heap-allocated null-terminated string.
/// Caller must free with `lpp_free_str`.
#[no_mangle]
pub extern "C" fn lpp_input() -> *mut u8 {
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).ok();
    let c = std::ffi::CString::new(buf.trim_end_matches('\n')).unwrap();
    c.into_raw() as *mut u8
}

/// Free a string returned by `lpp_input`.
/// # Safety
/// `ptr` must have been returned by `lpp_input`.
#[no_mangle]
pub unsafe extern "C" fn lpp_free_str(ptr: *mut u8) {
    if !ptr.is_null() {
        drop(std::ffi::CString::from_raw(ptr as *mut i8));
    }
}
