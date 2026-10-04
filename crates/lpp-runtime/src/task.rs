//! The single-threaded task runtime (Phase 6B.3d).
//!
//! Rust re-implementation of the `lpp_runtime.c` task primitives with an
//! identical ABI, layout, and atomic state machine. A task is an
//! ARC-managed object whose payload is the `LppTask` struct below; the
//! executor policy is the v1 first-tier one — deterministic run-to-completion
//! on the calling thread, a task polled at most once, double-poll/double-await
//! idempotent. The task owns its environment (transferred at creation) and,
//! when `result_managed`, one reference to its result; both are released by
//! the ARC destructor.

use std::sync::atomic::{AtomicI32, Ordering};

use libc::c_void;

use crate::arc::{lpp_arc_alloc_with_destructor, lpp_arc_release, lpp_arc_retain};
use crate::panic::runtime_panic;

/// The task code entry point: `int64_t (*)(void *environment)`.
type TaskCode = unsafe extern "C" fn(*mut c_void) -> i64;

/// The task payload. `#[repr(C)]` + field order match the C `LppTask`
/// exactly (code@0, environment@8, result@16, state@24, result_managed@28;
/// size 32). `state`: 0 pending, 1 running, 2 complete.
#[repr(C)]
pub struct LppTask {
    code: usize,
    environment: *mut c_void,
    result: i64,
    state: AtomicI32,
    result_managed: i32,
}

/// The ARC destructor: release the owned environment, and — once complete —
/// the owned managed result. Mirrors C `lpp_task_payload_destroy`.
unsafe extern "C" fn task_payload_destroy(payload: *mut c_void) {
    if payload.is_null() {
        return;
    }
    let task = unsafe { &mut *(payload as *mut LppTask) };
    if !task.environment.is_null() {
        unsafe { lpp_arc_release(task.environment) };
        task.environment = std::ptr::null_mut();
    }
    if task.state.load(Ordering::Acquire) == 2 && task.result_managed != 0 && task.result != 0 {
        unsafe { lpp_arc_release(task.result as usize as *mut c_void) };
        task.result = 0;
    }
}

/// Create a task. `code_ptr` and `environment` are both required; the
/// environment reference is transferred (owned by the task).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_new(
    code_ptr: *mut c_void,
    environment: *mut c_void,
    result_managed: i64,
) -> *mut c_void {
    if code_ptr.is_null() || environment.is_null() {
        runtime_panic("task creation requires code and environment");
    }
    let size = std::mem::size_of::<LppTask>() as i64;
    let raw = unsafe { lpp_arc_alloc_with_destructor(size, Some(task_payload_destroy)) };
    if raw.is_null() {
        runtime_panic("out of memory while allocating task");
    }
    let task = unsafe { &mut *(raw as *mut LppTask) };
    task.code = code_ptr as usize;
    task.environment = environment;
    task.result = 0;
    task.state.store(0, Ordering::Relaxed);
    task.result_managed = i32::from(result_managed != 0);
    raw
}

/// Poll the task to completion (run-to-completion executor). Returns 1.
/// Double-poll is idempotent; concurrent/recursive polling panics.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_poll(raw_task: *mut c_void) -> i64 {
    if raw_task.is_null() {
        runtime_panic("attempted to poll a null task");
    }
    let task = unsafe { &mut *(raw_task as *mut LppTask) };
    if task.state.load(Ordering::Acquire) == 2 {
        return 1; // double-poll is idempotent
    }
    match task
        .state
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
    {
        Ok(_) => {}
        Err(expected) => {
            if expected == 2 {
                return 1;
            }
            runtime_panic("concurrent or recursive polling of the same task");
        }
    }
    let code: TaskCode = unsafe { std::mem::transmute(task.code) };
    let env = task.environment;
    let result = unsafe { code(env) };
    task.result = result;
    task.state.store(2, Ordering::Release);
    1
}

/// Await the task: poll it, then return its result. Each await of a managed
/// result creates a fresh caller-owned reference (the task keeps its own).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_await(raw_task: *mut c_void) -> i64 {
    // executor_run: poll (null-checked inside) then read the result.
    unsafe { lpp_task_poll(raw_task) };
    let task = unsafe { &*(raw_task as *mut LppTask) };
    let result = task.result;
    if task.result_managed != 0 && result != 0 {
        unsafe { lpp_arc_retain(result as usize as *mut c_void) };
    }
    result
}

/// Destroy the task: a single ARC reference release (the destructor handles
/// the environment and managed result).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_destroy(raw_task: *mut c_void) {
    unsafe { lpp_arc_release(raw_task) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc::lpp_arc_alloc_with_destructor;
    use crate::numeric::lpp_vec_i64_checksum;
    use crate::tuple::lpp_tuple_alloc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn task_layout_matches_c() {
        assert_eq!(std::mem::size_of::<LppTask>(), 32, "LppTask size");
        assert_eq!(std::mem::align_of::<LppTask>(), 8, "LppTask align");
    }

    static DROPS: AtomicUsize = AtomicUsize::new(0);
    static SERIAL: Mutex<()> = Mutex::new(());

    unsafe extern "C" fn dtor(_p: *mut c_void) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
    unsafe extern "C" fn code_int(_env: *mut c_void) -> i64 {
        42
    }
    unsafe extern "C" fn code_managed(_env: *mut c_void) -> i64 {
        let o = unsafe { lpp_arc_alloc_with_destructor(8, Some(dtor)) };
        o as i64
    }

    fn mix(h: &mut i64, v: i64) {
        *h = (*h ^ v).wrapping_mul(1099511628211);
    }

    /// The golden fingerprint of the managed scenario (tasks + tuple + vec
    /// checksum), produced by the v1 C reference. The C side is pinned in
    /// `tests/runtime_gate.rs::c_reference_matches_the_managed_golden`.
    const MANAGED_GOLDEN: i64 = -1109191003795678529;

    fn scenario() -> i64 {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut h: i64 = 1469598103934665603;
        DROPS.store(0, Ordering::Relaxed);
        unsafe {
            // Task A: plain int result.
            let env_a = lpp_arc_alloc_with_destructor(8, Some(dtor));
            let t_a = lpp_task_new(code_int as TaskCode as *mut c_void, env_a, 0);
            mix(&mut h, lpp_task_poll(t_a));
            mix(&mut h, lpp_task_await(t_a));
            mix(&mut h, lpp_task_poll(t_a));
            lpp_task_destroy(t_a);
            mix(&mut h, DROPS.load(Ordering::Relaxed) as i64);

            // Task B: managed result (await retains for the caller).
            let env_b = lpp_arc_alloc_with_destructor(8, Some(dtor));
            let t_b = lpp_task_new(code_managed as TaskCode as *mut c_void, env_b, 1);
            let r = lpp_task_await(t_b);
            mix(&mut h, i64::from(r != 0));
            lpp_arc_release(r as usize as *mut c_void);
            lpp_task_destroy(t_b);
            mix(&mut h, DROPS.load(Ordering::Relaxed) as i64);

            // Tuple with one managed child slot at byte offset 16.
            let child = lpp_arc_alloc_with_destructor(8, Some(dtor));
            let tup = lpp_tuple_alloc(32, 1, 16);
            *(tup.cast::<u8>().add(16) as *mut *mut c_void) = child;
            lpp_arc_release(tup);
            mix(&mut h, DROPS.load(Ordering::Relaxed) as i64);

            // Vec checksum.
            mix(&mut h, lpp_vec_i64_checksum(0));
            mix(&mut h, lpp_vec_i64_checksum(1));
            mix(&mut h, lpp_vec_i64_checksum(10));
            mix(&mut h, lpp_vec_i64_checksum(100));
            mix(&mut h, lpp_vec_i64_checksum(-5));
            mix(&mut h, lpp_vec_i64_checksum(1000));
        }
        h
    }

    #[test]
    fn managed_scenario_is_golden() {
        assert_eq!(
            scenario(),
            MANAGED_GOLDEN,
            "Rust task/tuple/vec runtime diverges from the C reference"
        );
    }
}
