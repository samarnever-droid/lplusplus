//! Atomics, mutexes, rwlocks, and small thread/environment builtins.
//!
//! These are host-backed implementations of the frozen v1 concurrency ABI.
//! They are intentionally thin: handles are opaque process-local pointers, just
//! as in the legacy C runtime.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering, fence};

fn atomic64(handle: i64) -> Option<&'static AtomicI64> {
    if handle == 0 {
        return None;
    }
    unsafe { (handle as *const AtomicI64).as_ref() }
}

fn atomic32(handle: i64) -> Option<&'static AtomicI32> {
    if handle == 0 {
        return None;
    }
    unsafe { (handle as *const AtomicI32).as_ref() }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_new(init_val: i64) -> i64 {
    Box::into_raw(Box::new(AtomicI64::new(init_val))) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_free(handle: i64) {
    if handle != 0 {
        unsafe { drop(Box::from_raw(handle as *mut AtomicI64)) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_load(handle: i64) -> i64 {
    atomic64(handle).map_or(0, |value| value.load(Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_load_acq(handle: i64) -> i64 {
    atomic64(handle).map_or(0, |value| value.load(Ordering::Acquire))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_load_relaxed(handle: i64) -> i64 {
    atomic64(handle).map_or(0, |value| value.load(Ordering::Relaxed))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_store(handle: i64, value: i64) {
    if let Some(cell) = atomic64(handle) {
        cell.store(value, Ordering::SeqCst);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_store_rel(handle: i64, value: i64) {
    if let Some(cell) = atomic64(handle) {
        cell.store(value, Ordering::Release);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_store_relaxed(handle: i64, value: i64) {
    if let Some(cell) = atomic64(handle) {
        cell.store(value, Ordering::Relaxed);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_add(handle: i64, value: i64) -> i64 {
    atomic64(handle).map_or(0, |cell| cell.fetch_add(value, Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_sub(handle: i64, value: i64) -> i64 {
    atomic64(handle).map_or(0, |cell| cell.fetch_sub(value, Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_and(handle: i64, value: i64) -> i64 {
    atomic64(handle).map_or(0, |cell| cell.fetch_and(value, Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_or(handle: i64, value: i64) -> i64 {
    atomic64(handle).map_or(0, |cell| cell.fetch_or(value, Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_xor(handle: i64, value: i64) -> i64 {
    atomic64(handle).map_or(0, |cell| cell.fetch_xor(value, Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_swap(handle: i64, value: i64) -> i64 {
    atomic64(handle).map_or(0, |cell| cell.swap(value, Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_cas(handle: i64, expected: i64, desired: i64) -> i64 {
    let Some(cell) = atomic64(handle) else {
        return 0;
    };
    match cell.compare_exchange(expected, desired, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(old) | Err(old) => old,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_cas_weak(handle: i64, expected: i64, desired: i64) -> i64 {
    let Some(cell) = atomic64(handle) else {
        return 0;
    };
    match cell.compare_exchange_weak(expected, desired, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(old) | Err(old) => old,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_load32(handle: i64) -> i64 {
    atomic32(handle).map_or(0, |value| i64::from(value.load(Ordering::SeqCst)))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_store32(handle: i64, value: i64) {
    if let Some(cell) = atomic32(handle) {
        cell.store(value as i32, Ordering::SeqCst);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_add32(handle: i64, value: i64) -> i64 {
    atomic32(handle).map_or(0, |cell| {
        i64::from(cell.fetch_add(value as i32, Ordering::SeqCst))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_cas32(handle: i64, expected: i64, desired: i64) -> i64 {
    let Some(cell) = atomic32(handle) else {
        return 0;
    };
    match cell.compare_exchange(
        expected as i32,
        desired as i32,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(old) | Err(old) => i64::from(old),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_fence() {
    fence(Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_fence_acq() {
    fence(Ordering::Acquire);
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_atomic_fence_rel() {
    fence(Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_cpu_pause() {
    std::hint::spin_loop();
}

#[cfg(unix)]
mod pthread_sync {
    use libc::{
        pthread_mutex_destroy, pthread_mutex_init, pthread_mutex_lock, pthread_mutex_t,
        pthread_mutex_trylock, pthread_mutex_unlock, pthread_rwlock_destroy, pthread_rwlock_init,
        pthread_rwlock_rdlock, pthread_rwlock_t, pthread_rwlock_unlock, pthread_rwlock_wrlock,
    };
    use std::mem::MaybeUninit;

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_new() -> i64 {
        unsafe {
            let mut boxed = Box::<pthread_mutex_t>::new_uninit();
            let ptr = boxed.as_mut_ptr();
            if pthread_mutex_init(ptr, std::ptr::null()) != 0 {
                return 0;
            }
            Box::into_raw(boxed) as i64
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_lock(handle: i64) {
        if handle != 0 {
            unsafe { pthread_mutex_lock((handle as *mut MaybeUninit<pthread_mutex_t>).cast()) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_trylock(handle: i64) -> i64 {
        if handle == 0 {
            return 0;
        }
        let rc =
            unsafe { pthread_mutex_trylock((handle as *mut MaybeUninit<pthread_mutex_t>).cast()) };
        i64::from(rc == 0)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_unlock(handle: i64) {
        if handle != 0 {
            unsafe { pthread_mutex_unlock((handle as *mut MaybeUninit<pthread_mutex_t>).cast()) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_free(handle: i64) {
        if handle != 0 {
            unsafe {
                let ptr = handle as *mut MaybeUninit<pthread_mutex_t>;
                pthread_mutex_destroy(ptr.cast());
                drop(Box::from_raw(ptr));
            }
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_new() -> i64 {
        unsafe {
            let mut boxed = Box::<pthread_rwlock_t>::new_uninit();
            let ptr = boxed.as_mut_ptr();
            if pthread_rwlock_init(ptr, std::ptr::null()) != 0 {
                return 0;
            }
            Box::into_raw(boxed) as i64
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_rdlock(handle: i64) {
        if handle != 0 {
            unsafe { pthread_rwlock_rdlock((handle as *mut MaybeUninit<pthread_rwlock_t>).cast()) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_wrlock(handle: i64) {
        if handle != 0 {
            unsafe { pthread_rwlock_wrlock((handle as *mut MaybeUninit<pthread_rwlock_t>).cast()) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_rdunlock(handle: i64) {
        if handle != 0 {
            unsafe { pthread_rwlock_unlock((handle as *mut MaybeUninit<pthread_rwlock_t>).cast()) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_wrunlock(handle: i64) {
        lpp_rwlock_rdunlock(handle);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_free(handle: i64) {
        if handle != 0 {
            unsafe {
                let ptr = handle as *mut MaybeUninit<pthread_rwlock_t>;
                pthread_rwlock_destroy(ptr.cast());
                drop(Box::from_raw(ptr));
            }
        }
    }
}

#[cfg(windows)]
mod windows_sync {
    use std::ffi::c_void;

    #[repr(C)]
    struct CriticalSection {
        debug_info: *mut c_void,
        lock_count: i32,
        recursion_count: i32,
        owning_thread: *mut c_void,
        lock_semaphore: *mut c_void,
        spin_count: usize,
    }

    #[repr(C)]
    struct SrwLock {
        ptr: *mut c_void,
    }

    unsafe extern "system" {
        fn InitializeCriticalSection(cs: *mut CriticalSection);
        fn EnterCriticalSection(cs: *mut CriticalSection);
        fn TryEnterCriticalSection(cs: *mut CriticalSection) -> i32;
        fn LeaveCriticalSection(cs: *mut CriticalSection);
        fn DeleteCriticalSection(cs: *mut CriticalSection);

        fn InitializeSRWLock(srw: *mut SrwLock);
        fn AcquireSRWLockShared(srw: *mut SrwLock);
        fn AcquireSRWLockExclusive(srw: *mut SrwLock);
        fn ReleaseSRWLockShared(srw: *mut SrwLock);
        fn ReleaseSRWLockExclusive(srw: *mut SrwLock);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_new() -> i64 {
        let boxed = Box::new(CriticalSection {
            debug_info: std::ptr::null_mut(),
            lock_count: 0,
            recursion_count: 0,
            owning_thread: std::ptr::null_mut(),
            lock_semaphore: std::ptr::null_mut(),
            spin_count: 0,
        });
        let raw = Box::into_raw(boxed);
        unsafe { InitializeCriticalSection(raw) };
        raw as i64
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_lock(handle: i64) {
        if handle != 0 {
            unsafe { EnterCriticalSection(handle as *mut CriticalSection) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_trylock(handle: i64) -> i64 {
        if handle == 0 {
            return 0;
        }
        let rc = unsafe { TryEnterCriticalSection(handle as *mut CriticalSection) };
        i64::from(rc != 0)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_unlock(handle: i64) {
        if handle != 0 {
            unsafe { LeaveCriticalSection(handle as *mut CriticalSection) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_free(handle: i64) {
        if handle != 0 {
            let ptr = handle as *mut CriticalSection;
            unsafe {
                DeleteCriticalSection(ptr);
                drop(Box::from_raw(ptr));
            }
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_new() -> i64 {
        let boxed = Box::new(SrwLock {
            ptr: std::ptr::null_mut(),
        });
        let raw = Box::into_raw(boxed);
        unsafe { InitializeSRWLock(raw) };
        raw as i64
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_rdlock(handle: i64) {
        if handle != 0 {
            unsafe { AcquireSRWLockShared(handle as *mut SrwLock) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_wrlock(handle: i64) {
        if handle != 0 {
            unsafe { AcquireSRWLockExclusive(handle as *mut SrwLock) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_rdunlock(handle: i64) {
        if handle != 0 {
            unsafe { ReleaseSRWLockShared(handle as *mut SrwLock) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_wrunlock(handle: i64) {
        if handle != 0 {
            unsafe { ReleaseSRWLockExclusive(handle as *mut SrwLock) };
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_free(handle: i64) {
        if handle != 0 {
            let ptr = handle as *mut SrwLock;
            unsafe {
                drop(Box::from_raw(ptr));
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod stub_sync {
    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_new() -> i64 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_lock(_handle: i64) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_trylock(_handle: i64) -> i64 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_unlock(_handle: i64) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_mutex_free(_handle: i64) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_new() -> i64 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_rdlock(_handle: i64) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_wrlock(_handle: i64) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_rdunlock(_handle: i64) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_wrunlock(_handle: i64) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn lpp_rwlock_free(_handle: i64) {}
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_cpu_count() -> i64 {
    std::thread::available_parallelism().map_or(1, |count| count.get() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_thread_spawn(fn_ptr: i64, arg: i64) -> i64 {
    if fn_ptr == 0 {
        return 0;
    }
    let fn_addr = fn_ptr as usize;
    match std::thread::Builder::new().spawn(move || {
        let f: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(fn_addr) };
        f(arg)
    }) {
        Ok(handle) => Box::into_raw(Box::new(handle)) as i64,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_thread_join(handle: i64) -> i64 {
    if handle == 0 {
        return 0;
    }
    let join = unsafe { Box::from_raw(handle as *mut std::thread::JoinHandle<i64>) };
    join.join().unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_thread_pin(core_id: i64) -> i64 {
    let _ = core_id;
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_thread_id() -> i64 {
    let mut hasher = DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    let value = (hasher.finish() & 0x7fff_ffff_ffff_ffff) as i64;
    if value == 0 { 1 } else { value }
}
