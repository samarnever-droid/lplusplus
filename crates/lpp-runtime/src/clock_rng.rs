//! Clock and random-number builtins for the rewrite runtime.
//!
//! This ports the frozen v1 `time_ms`, `random_*`, `rng_*`, and `clock_*` ABI
//! surface to Rust without changing `abi/builtins.toml`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static GLOBAL_RNG_STATE: AtomicU64 = AtomicU64::new(0x853c49e6748fea9b);

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn now_epoch_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis().min(i64::MAX as u128) as i64,
        Err(_) => 0,
    }
}

fn global_next() -> u64 {
    loop {
        let old = GLOBAL_RNG_STATE.load(Ordering::Relaxed);
        let mut next_state = old;
        let value = splitmix64(&mut next_state);
        if GLOBAL_RNG_STATE
            .compare_exchange_weak(old, next_state, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return value;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_time_ms() -> i64 {
    now_epoch_ms()
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_random_seed(seed: i64) {
    GLOBAL_RNG_STATE.store(seed as u64, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_random() -> i64 {
    (global_next() & 0x7fff_ffff_ffff_ffff) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_random_range(lo: i64, hi: i64) -> i64 {
    if lo >= hi {
        return lo;
    }
    let diff = (hi - lo) as u64;
    lo.wrapping_add((lpp_random() as u64 % diff) as i64)
}

struct Rng {
    state: u64,
}

fn rng_mut(handle: i64) -> Option<&'static mut Rng> {
    if handle == 0 {
        return None;
    }
    unsafe { (handle as *mut Rng).as_mut() }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rng_new(seed: i64) -> i64 {
    let state = if seed == 0 {
        0x853c49e6748fea9b
    } else {
        seed as u64
    };
    Box::into_raw(Box::new(Rng { state })) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rng_next(handle: i64) -> i64 {
    let Some(rng) = rng_mut(handle) else {
        return 0;
    };
    splitmix64(&mut rng.state) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rng_range(handle: i64, min_v: i64, max_v: i64) -> i64 {
    if min_v >= max_v {
        return min_v;
    }
    let diff = (max_v - min_v + 1) as u64;
    let value = lpp_rng_next(handle) as u64;
    min_v.wrapping_add((value % diff) as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rng_float(handle: i64) -> f64 {
    let value = lpp_rng_next(handle) as u64;
    ((value >> 11) as f64) * (1.0 / 9007199254740992.0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rng_free(handle: i64) {
    if handle != 0 {
        unsafe { drop(Box::from_raw(handle as *mut Rng)) };
    }
}

struct Clock {
    is_virtual: bool,
    current_time: i64,
}

fn clock_mut(handle: i64) -> Option<&'static mut Clock> {
    if handle == 0 {
        return None;
    }
    unsafe { (handle as *mut Clock).as_mut() }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_clock_new(initial_time: i64) -> i64 {
    Box::into_raw(Box::new(Clock {
        is_virtual: initial_time != 0,
        current_time: initial_time,
    })) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_clock_now(handle: i64) -> i64 {
    let Some(clock) = clock_mut(handle) else {
        return 0;
    };
    if clock.is_virtual {
        clock.current_time
    } else {
        now_epoch_ms()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_clock_advance(handle: i64, delta: i64) {
    if let Some(clock) = clock_mut(handle) {
        if clock.is_virtual {
            clock.current_time = clock.current_time.wrapping_add(delta);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_clock_free(handle: i64) {
    if handle != 0 {
        unsafe { drop(Box::from_raw(handle as *mut Clock)) };
    }
}
