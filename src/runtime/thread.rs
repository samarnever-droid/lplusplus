// L++ Thread runtime stubs.
// `spawn` launches a closure on a new OS thread.

/// Spawn a new OS thread calling `func_ptr` with `env_ptr` as the argument.
/// Used for the `spawn fn() -> Void: ...` construct in L++.
/// # Safety
/// `func_ptr` must be a valid function pointer with signature `fn(*mut u8)`.
/// `env_ptr` must remain valid for the lifetime of the spawned thread.
#[no_mangle]
pub unsafe extern "C" fn lpp_thread_spawn(func_ptr: *const u8, env_ptr: *mut u8) {
    let f: fn(*mut u8) = std::mem::transmute(func_ptr);
    std::thread::spawn(move || f(env_ptr));
}
