//! The numeric builtins (Phase 6B.3a).
//!
//! Rust re-implementation of the pure-numeric primitives from the v1 C
//! runtime (`runtime/lpp_int.c` + the math/`abs`/`min`/`max` helpers in
//! `runtime/lpp_str.c` and `lpp_runtime.c`), ABI-identical: every function
//! takes/returns `i64` (or `f64` for the float helpers) exactly as the C
//! reference, with the same edge-case behavior — unsigned shifts clamp the
//! shift count, `div_u`/`rem_u` panic on zero, the `*_checked` ops panic on
//! overflow, the `*_wrap` ops wrap, `clz`/`ctz` of zero are 64, and the
//! truncate/bswap/rotate ops reproduce the C unsigned-cast semantics.
//!
//! The string-returning numeric conversions (`lpp_u64_to_str`,
//! `lpp_u64_to_hex`, `lpp_str_to_u64`) live in the string module (6B.3b).

use crate::panic::runtime_panic;

// ── signed helpers ────────────────────────────────────────────────────────

/// `x < 0 ? -x : x`. For `i64::MIN` the C expression is UB but evaluates to
/// `i64::MIN` in practice; `wrapping_neg` reproduces that.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_abs(x: i64) -> i64 {
    if x < 0 { x.wrapping_neg() } else { x }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_min(a: i64, b: i64) -> i64 {
    if a < b { a } else { b }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_max(a: i64, b: i64) -> i64 {
    if a > b { a } else { b }
}

// ── float helpers (libm-equivalent via f64 intrinsics) ────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_sqrt(x: f64) -> f64 {
    // The host C runtime is `sqrt(x)` (libm); for x <= 0 libm returns NaN/0
    // per IEEE. Rust's f64::sqrt matches libm sqrt bit-for-bit.
    x.sqrt()
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_floor(x: f64) -> f64 {
    x.floor()
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_ceil(x: f64) -> f64 {
    x.ceil()
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_pow(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_sin(x: f64) -> f64 {
    x.sin()
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_cos(x: f64) -> f64 {
    x.cos()
}

/// `int_pow(base, exp)` — integer exponentiation by repeated multiplication.
/// `exp == 0` is 1; a negative exponent on integers cannot be represented, so
/// it yields 0 (matching the C reference). Multiplication wraps so a debug
/// build never panics on overflow.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_int_pow(base: i64, exp: i64) -> i64 {
    if exp < 0 {
        return 0;
    }
    let mut result: i64 = 1;
    for _ in 0..exp {
        result = result.wrapping_mul(base);
    }
    result
}

// ── unsigned shifts / division / comparison ───────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_shr_u(a: i64, b: i64) -> i64 {
    if b < 0 || b >= 64 {
        return 0;
    }
    ((a as u64) >> b) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_shl_u(a: i64, b: i64) -> i64 {
    if b < 0 || b >= 64 {
        return 0;
    }
    ((a as u64) << b) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_div_u(a: i64, b: i64) -> i64 {
    if b == 0 {
        runtime_panic("unsigned integer division by zero");
    }
    ((a as u64) / (b as u64)) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rem_u(a: i64, b: i64) -> i64 {
    if b == 0 {
        runtime_panic("unsigned integer modulo by zero");
    }
    ((a as u64) % (b as u64)) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_lt_u(a: i64, b: i64) -> i64 {
    i64::from((a as u64) < (b as u64))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_le_u(a: i64, b: i64) -> i64 {
    i64::from((a as u64) <= (b as u64))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_gt_u(a: i64, b: i64) -> i64 {
    i64::from((a as u64) > (b as u64))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_ge_u(a: i64, b: i64) -> i64 {
    i64::from((a as u64) >= (b as u64))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_min_u(a: i64, b: i64) -> i64 {
    if (a as u64) < (b as u64) { a } else { b }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_max_u(a: i64, b: i64) -> i64 {
    if (a as u64) > (b as u64) { a } else { b }
}

// ── rotates ───────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rotl64(a: i64, b: i64) -> i64 {
    (a as u64).rotate_left((b & 63) as u32) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rotr64(a: i64, b: i64) -> i64 {
    (a as u64).rotate_right((b & 63) as u32) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rotl32(a: i64, b: i64) -> i64 {
    i64::from((a as u32).rotate_left((b & 31) as u32))
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_rotr32(a: i64, b: i64) -> i64 {
    i64::from((a as u32).rotate_right((b & 31) as u32))
}

// ── bit counting ──────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_clz64(a: i64) -> i64 {
    if a == 0 {
        64
    } else {
        i64::from((a as u64).leading_zeros())
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_ctz64(a: i64) -> i64 {
    if a == 0 {
        64
    } else {
        i64::from((a as u64).trailing_zeros())
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_popcount64(a: i64) -> i64 {
    i64::from((a as u64).count_ones())
}

// ── byte swaps ────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_bswap16(a: i64) -> i64 {
    i64::from((a as u16).swap_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_bswap32(a: i64) -> i64 {
    i64::from((a as u32).swap_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_bswap64(a: i64) -> i64 {
    (a as u64).swap_bytes() as i64
}

// ── truncations (C unsigned/signed cast semantics) ────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_trunc_u8(a: i64) -> i64 {
    i64::from(a as u8)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_trunc_u16(a: i64) -> i64 {
    i64::from(a as u16)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_trunc_u32(a: i64) -> i64 {
    i64::from(a as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_trunc_i8(a: i64) -> i64 {
    i64::from(a as i8)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_trunc_i16(a: i64) -> i64 {
    i64::from(a as i16)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_trunc_i32(a: i64) -> i64 {
    i64::from(a as i32)
}

// ── checked arithmetic (panic on overflow, like C) ────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_add_checked(a: i64, b: i64) -> i64 {
    match a.checked_add(b) {
        Some(r) => r,
        None => runtime_panic(&format!("integer addition overflow: {a} + {b}")),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_sub_checked(a: i64, b: i64) -> i64 {
    match a.checked_sub(b) {
        Some(r) => r,
        None => runtime_panic(&format!("integer subtraction overflow: {a} - {b}")),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_mul_checked(a: i64, b: i64) -> i64 {
    match a.checked_mul(b) {
        Some(r) => r,
        None => runtime_panic(&format!("integer multiplication overflow: {a} * {b}")),
    }
}

// ── wrapping arithmetic ───────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_add_wrap(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_sub_wrap(a: i64, b: i64) -> i64 {
    a.wrapping_sub(b)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_mul_wrap(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b)
}

/// `vec_i64_checksum(n)` — `sum_{i<n} (i*3) ^ (i>>1)`, the scalar reference
/// for the explicit SIMD vector intrinsic (the AVX2 path in the C runtime
/// computes the identical value; vectorization does not change the result).
/// Negative `n` yields 0. All arithmetic wraps, matching C.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_vec_i64_checksum(n: i64) -> i64 {
    if n < 0 {
        return 0;
    }
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        total = total.wrapping_add(i.wrapping_mul(3) ^ (i >> 1));
        i += 1;
    }
    total
}

/// `prefetch` is an optimization hint in the C runtime. It must be ABI-visible
/// for generated objects, but it has no semantic effect.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_prefetch(_addr: i64) {}

/// Write-prefetch twin of `lpp_prefetch`; also intentionally a no-op on hosts
/// where the Rust runtime does not issue architecture-specific prefetch hints.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_prefetch_write(_addr: i64) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_builtins_match_the_c_semantics() {
        // A spot-check of the edge cases that are easy to get wrong; the
        // exhaustive differential lives in tests/runtime_gate.rs (golden
        // fingerprint vs the v1 C reference).
        assert_eq!(lpp_abs(-5), 5);
        assert_eq!(lpp_abs(i64::MIN), i64::MIN, "wrapping neg of MIN");
        assert_eq!(lpp_min(3, 7), 3);
        assert_eq!(lpp_max(3, 7), 7);

        // Unsigned shift clamping.
        assert_eq!(lpp_shr_u(-1, 64), 0);
        assert_eq!(lpp_shr_u(-1, 0), -1);
        assert_eq!(lpp_shl_u(1, 63), i64::MIN);
        assert_eq!(lpp_shl_u(1, 64), 0);

        // Unsigned comparison: -1 is the largest unsigned.
        assert_eq!(lpp_gt_u(-1, 1), 1);
        assert_eq!(lpp_lt_u(-1, 1), 0);
        assert_eq!(lpp_max_u(-1, 1), -1);

        // div/rem.
        assert_eq!(lpp_div_u(-1, 2), (u64::MAX / 2) as i64);
        assert_eq!(lpp_rem_u(7, 3), 1);

        // Rotates.
        assert_eq!(lpp_rotl64(1, 63), i64::MIN);
        assert_eq!(lpp_rotr64(i64::MIN, 63), 1);
        assert_eq!(
            lpp_rotl32(1, 31),
            0x8000_0000i64,
            "C zero-extends (int64_t)(uint32_t)"
        );
        assert_eq!(lpp_rotl64(5, 0), 5);

        // Bit counting.
        assert_eq!(lpp_clz64(0), 64);
        assert_eq!(lpp_clz64(1), 63);
        assert_eq!(lpp_ctz64(0), 64);
        assert_eq!(lpp_ctz64(8), 3);
        assert_eq!(lpp_popcount64(-1), 64);
        assert_eq!(lpp_popcount64(0b1011), 3);

        // Byte swaps.
        assert_eq!(lpp_bswap16(0x1234), 0x3412);
        assert_eq!(lpp_bswap32(0x1234_5678), 0x7856_3412);
        assert_eq!(
            lpp_bswap64(0x0123_4567_89ab_cdef),
            0xefcd_ab89_6745_2301u64 as i64
        );

        // Truncations.
        assert_eq!(lpp_trunc_u8(0x1ff), 0xff);
        assert_eq!(lpp_trunc_i8(0xff), -1);
        assert_eq!(lpp_trunc_u32(-1), 0xffff_ffff);
        assert_eq!(lpp_trunc_i32(0xffff_ffff), -1);

        // Checked arithmetic (in-range).
        assert_eq!(lpp_add_checked(2, 3), 5);
        assert_eq!(lpp_sub_checked(2, 3), -1);
        assert_eq!(lpp_mul_checked(-2, 3), -6);

        // Wrapping arithmetic.
        assert_eq!(lpp_add_wrap(i64::MAX, 1), i64::MIN);
        assert_eq!(lpp_sub_wrap(i64::MIN, 1), i64::MAX);
        assert_eq!(lpp_mul_wrap(i64::MAX, 2), -2);

        // Floats (libm-equivalent).
        assert_eq!(lpp_sqrt(9.0), 3.0);
        assert_eq!(lpp_floor(-1.5), -2.0);
        assert_eq!(lpp_ceil(1.2), 2.0);
        assert_eq!(lpp_pow(2.0, 10.0), 1024.0);
    }

    /// FNV-1a-style fold matching the C scenario's `mix`: H = (H ^ v) * P,
    /// wrapping in i64 (the C reference overflows signed long long, which
    /// gcc wraps as two's complement — identical bits).
    fn fnv_fold(values: &[i64]) -> i64 {
        let mut h: i64 = 1469598103934665603;
        for &v in values {
            h = (h ^ v).wrapping_mul(1099511628211);
        }
        h
    }

    /// The golden fingerprint of the numeric scenario, produced by the v1 C
    /// reference (`lpp_runtime.c` -> `runtime/lpp_int.c` + the math/abs/min/
    /// max helpers). The C side is pinned in
    /// `tests/runtime_gate.rs::c_reference_matches_the_numeric_golden`.
    const NUM_GOLDEN: i64 = 8794162424430263925;

    fn numeric_scenario() -> i64 {
        fnv_fold(&[
            lpp_abs(-5),
            lpp_abs(123456789012),
            lpp_min(3, 7),
            lpp_max(3, 7),
            lpp_min(-1, -2),
            lpp_max(-1, -2),
            lpp_shr_u(-1, 4),
            lpp_shr_u(-1, 64),
            lpp_shr_u(255, -3),
            lpp_shl_u(1, 63),
            lpp_shl_u(1, 64),
            lpp_shl_u(3, 10),
            lpp_div_u(-1, 2),
            lpp_div_u(100, 7),
            lpp_rem_u(7, 3),
            lpp_rem_u(-1, 16),
            lpp_lt_u(-1, 1),
            lpp_le_u(5, 5),
            lpp_gt_u(-1, 1),
            lpp_ge_u(2, 3),
            lpp_min_u(-1, 1),
            lpp_max_u(-1, 1),
            lpp_rotl64(1, 63),
            lpp_rotr64(1, 1),
            lpp_rotl64(5, 0),
            lpp_rotl32(1, 31),
            lpp_rotr32(0x8000_0000, 31),
            lpp_rotl32(0x1234_5678, 8),
            lpp_clz64(0),
            lpp_clz64(1),
            lpp_ctz64(0),
            lpp_ctz64(8),
            lpp_popcount64(-1),
            lpp_popcount64(11),
            lpp_bswap16(0x1234),
            lpp_bswap32(0x1234_5678),
            lpp_bswap64(0x0123_4567_89ab_cdef),
            lpp_trunc_u8(0x1ff),
            lpp_trunc_u16(0x1ffff),
            lpp_trunc_u32(-1),
            lpp_trunc_i8(0xff),
            lpp_trunc_i16(0xffff),
            lpp_trunc_i32(0xffff_ffff),
            lpp_add_checked(2, 3),
            lpp_sub_checked(2, 3),
            lpp_mul_checked(-2, 3),
            lpp_add_wrap(i64::MAX, 1),
            lpp_sub_wrap(i64::MIN, 1),
            lpp_mul_wrap(i64::MAX, 2),
            lpp_sqrt(9.0) as i64,
            lpp_floor(-1.5) as i64,
            lpp_ceil(1.2) as i64,
            lpp_pow(2.0, 10.0) as i64,
        ])
    }

    #[test]
    fn numeric_scenario_is_golden() {
        assert_eq!(
            numeric_scenario(),
            NUM_GOLDEN,
            "Rust numeric builtins diverge from the C reference"
        );
    }
}
