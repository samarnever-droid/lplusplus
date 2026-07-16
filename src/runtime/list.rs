// L++ List runtime stubs.
// A List<T> is represented as a heap-allocated Vec-style buffer.

/// Create a new empty list. Returns an opaque pointer to the list header.
#[no_mangle]
pub extern "C" fn lpp_list_new() -> *mut u8 {
    // For MVP: Box a Vec<i64> and return the raw pointer.
    let v: Box<Vec<i64>> = Box::new(Vec::new());
    Box::into_raw(v) as *mut u8
}

/// Push an i64 element onto the list.
/// # Safety
/// `list` must have been returned by `lpp_list_new`.
#[no_mangle]
pub unsafe extern "C" fn lpp_list_push(list: *mut u8, value: i64) {
    let v = &mut *(list as *mut Vec<i64>);
    v.push(value);
}

/// Get the element at `index` from the list.
/// # Safety
/// `list` must have been returned by `lpp_list_new` and `index` must be in bounds.
#[no_mangle]
pub unsafe extern "C" fn lpp_list_get(list: *mut u8, index: i64) -> i64 {
    let v = &*(list as *const Vec<i64>);
    v[index as usize]
}

/// Returns the current length of the list.
/// # Safety
/// `list` must have been returned by `lpp_list_new`.
#[no_mangle]
pub unsafe extern "C" fn lpp_list_len(list: *mut u8) -> i64 {
    let v = &*(list as *const Vec<i64>);
    v.len() as i64
}

/// Free a list.
/// # Safety
/// `list` must have been returned by `lpp_list_new` and not used after this call.
#[no_mangle]
pub unsafe extern "C" fn lpp_list_free(list: *mut u8) {
    drop(Box::from_raw(list as *mut Vec<i64>));
}
