// L++ String runtime stubs.
// Full implementation pending once the type system tracks string lengths.

/// Concatenate two L++ strings. Both inputs are null-terminated.
/// Returns a heap-allocated null-terminated result string.
/// # Safety
/// Both `a` and `b` must be valid null-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn lpp_str_concat(a: *const u8, b: *const u8) -> *mut u8 {
    let sa = std::ffi::CStr::from_ptr(a as *const i8).to_string_lossy();
    let sb = std::ffi::CStr::from_ptr(b as *const i8).to_string_lossy();
    let result = format!("{}{}", sa, sb);
    let c = std::ffi::CString::new(result).unwrap();
    c.into_raw() as *mut u8
}
