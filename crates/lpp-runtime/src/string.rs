//! The string builtins (Phase 6B.3b).
//!
//! Rust re-implementation of the string primitives from `runtime/lpp_str.c`
//! and `runtime/lpp_int.c`, ABI-identical to the v1 C reference. Every
//! string-producing builtin returns a NUL-terminated, ARC-allocated `char *`
//! (falling back to the immortal empty string on allocation failure, exactly
//! as C does); the predicates return `i64` 1/0. Float formatting goes through
//! libc `snprintf("%g")` so it is byte-identical to the C reference (Rust's
//! native float `Display` uses the shortest-round-trip form, which differs).
//!
//! The string-slice reads (`lpp_str_slice_get` / `lpp_str_slice_to_str`) live
//! in the slice module (they need the slice view internals); IO in `io.rs`.

use libc::{c_char, c_void, snprintf, strlen, strstr, strtoll};

use crate::arc::{lpp_arc_alloc, lpp_empty_str};
use crate::list::{lpp_list_new_arc, lpp_list_push_arc};
use crate::panic::runtime_panic;

/// The bytes of a NUL-terminated C string (empty for NULL), without the
/// terminator. Mirrors the C builtins' `if (!s) s = ""` normalization.
unsafe fn cstr_bytes<'a>(s: *const c_char) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(s as *const u8, strlen(s)) }
}

/// Allocate an ARC string holding `bytes` plus a NUL terminator. Returns the
/// immortal empty string on allocation failure (matching every C builtin's
/// `if (!out) return lpp_empty_str();`).
pub(crate) unsafe fn arc_string(bytes: &[u8]) -> *mut c_char {
    let p = unsafe { lpp_arc_alloc((bytes.len() + 1) as i64) } as *mut u8;
    if p.is_null() {
        return unsafe { lpp_empty_str() } as *mut c_char;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        *p.add(bytes.len()) = 0;
    }
    p as *mut c_char
}

fn is_ws(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r'
}

/// `str_concat(a, b)` — ARC-allocated concatenation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_concat(a: *const c_char, b: *const c_char) -> *mut c_char {
    let ab = unsafe { cstr_bytes(a) };
    let bb = unsafe { cstr_bytes(b) };
    let mut out = Vec::with_capacity(ab.len() + bb.len());
    out.extend_from_slice(ab);
    out.extend_from_slice(bb);
    unsafe { arc_string(&out) }
}

/// `str_repeat(s, n)` — ARC-allocated `s` repeated `n` times. `n <= 0` (or an
/// empty/NULL `s`) yields the empty string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_repeat(s: *const c_char, n: i64) -> *mut c_char {
    let sb = unsafe { cstr_bytes(s) };
    if n <= 0 || sb.is_empty() {
        return unsafe { arc_string(&[]) };
    }
    let mut out = Vec::new();
    for _ in 0..n {
        out.extend_from_slice(sb);
    }
    unsafe { arc_string(&out) }
}

/// `str_split(s, delim)` — split `s` on the byte `delim` (a character code),
/// returning an owning ARC list of ARC strings. Always yields at least one
/// element (the whole string when `delim` is absent), matching the C builtin:
/// a trailing delimiter produces a final empty piece.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_split(s: *const c_char, delim: i64) -> *mut c_void {
    let sb = unsafe { cstr_bytes(s) };
    let list = unsafe { lpp_list_new_arc() };
    let d = delim as u8;
    let mut start = 0usize;
    let mut i = 0usize;
    while i <= sb.len() {
        if i == sb.len() || sb[i] == d {
            let piece = unsafe { arc_string(&sb[start..i]) };
            unsafe { lpp_list_push_arc(list, piece as *mut c_void) };
            start = i + 1;
        }
        i += 1;
    }
    list
}

/// `str_find(haystack, needle)` — first index via `strstr`, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_find(haystack: *const c_char, needle: *const c_char) -> i64 {
    if haystack.is_null() || needle.is_null() {
        return -1;
    }
    let found = unsafe { strstr(haystack, needle) };
    if found.is_null() {
        -1
    } else {
        (found as usize - haystack as usize) as i64
    }
}

/// `str_replace(s, old, new)` — ARC-allocated with all occurrences replaced.
/// Always returns a NEW owned string (never the argument), so the caller can
/// release it independently of the input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_replace(
    s: *const c_char,
    old: *const c_char,
    new_: *const c_char,
) -> *mut c_char {
    let sb = unsafe { cstr_bytes(s) };
    let ob = unsafe { cstr_bytes(old) };
    // Empty/NULL `old`: return a plain copy (C special-cases this).
    if ob.is_empty() {
        return unsafe { arc_string(sb) };
    }
    let nb = unsafe { cstr_bytes(new_) };

    // Count non-overlapping occurrences (C advances by olen after each hit).
    let mut count = 0i64;
    let mut scan = sb;
    while let Some(pos) = find_sub(scan, &ob) {
        count += 1;
        scan = &scan[pos + ob.len()..];
    }

    let delta = nb.len() as i64 - ob.len() as i64;
    if count > 0 && delta > 0 && sb.len() as i64 > i64::MAX - count * delta {
        runtime_panic("str_replace: output size overflow");
    }
    let mut outlen = sb.len() as i64 + count * delta;
    if outlen < 0 {
        outlen = 0;
    }
    let mut out = Vec::with_capacity(outlen as usize);
    let mut src = sb;
    while !src.is_empty() {
        match find_sub(src, &ob) {
            None => {
                out.extend_from_slice(src);
                break;
            }
            Some(pos) => {
                out.extend_from_slice(&src[..pos]);
                out.extend_from_slice(nb);
                src = &src[pos + ob.len()..];
            }
        }
    }
    unsafe { arc_string(&out) }
}

/// First index of `needle` in `haystack` (byte slice equivalent of `strstr`).
fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `char_at(s, index)` — the byte at `index` as a one-character ARC string, or
/// the immortal empty string when `index` is out of range. Byte-indexed, as in
/// the v1 C reference (`char_at("Hello, World!", 7)` is `"W"`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_char_at(s: *const c_char, index: i64) -> *mut c_char {
    let bytes = unsafe { cstr_bytes(s) };
    if index < 0 || (index as usize) >= bytes.len() {
        return unsafe { lpp_empty_str() } as *mut c_char;
    }
    let i = index as usize;
    unsafe { arc_string(&bytes[i..i + 1]) }
}

/// `chr(code)` — the UTF-8 encoding of the Unicode scalar `code` as an ARC
/// string, or the immortal empty string when `code` is not a valid scalar.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_chr(code: i64) -> *mut c_char {
    match u32::try_from(code).ok().and_then(char::from_u32) {
        Some(c) => {
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            unsafe { arc_string(encoded.as_bytes()) }
        }
        None => (unsafe { lpp_empty_str() }) as *mut c_char,
    }
}

/// `ord(s)` — the Unicode scalar value of the first character of `s` (the
/// inverse of `chr`), or 0 for an empty/NULL string. A malformed lead byte
/// falls back to its raw value rather than trapping.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_ord(s: *const c_char) -> i64 {
    let bytes = unsafe { cstr_bytes(s) };
    let Some((&lead, _)) = bytes.split_first() else {
        return 0;
    };
    let extra = if lead < 0x80 {
        0
    } else if lead & 0xE0 == 0xC0 {
        1
    } else if lead & 0xF0 == 0xE0 {
        2
    } else if lead & 0xF8 == 0xF0 {
        3
    } else {
        return i64::from(lead);
    };
    if bytes.len() > extra {
        if let Ok(head) = std::str::from_utf8(&bytes[..extra + 1]) {
            if let Some(c) = head.chars().next() {
                return c as u32 as i64;
            }
        }
    }
    i64::from(lead)
}

/// `str_substr(s, start, len)` — the ARC-allocated substring of `s` from byte
/// `start`, `len` bytes long, clamped to the string's bounds. The immortal
/// empty string results when `start` is out of range or `len` is non-positive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_substr(s: *const c_char, start: i64, len: i64) -> *mut c_char {
    let bytes = unsafe { cstr_bytes(s) };
    if start < 0 || len <= 0 || (start as usize) >= bytes.len() {
        return unsafe { lpp_empty_str() } as *mut c_char;
    }
    let start = start as usize;
    let end = start.saturating_add(len as usize).min(bytes.len());
    unsafe { arc_string(&bytes[start..end]) }
}

/// `str_trim(s)` — ARC-allocated with leading/trailing whitespace removed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_trim(s: *const c_char) -> *mut c_char {
    if s.is_null() {
        return unsafe { lpp_empty_str() } as *mut c_char;
    }
    let b = unsafe { cstr_bytes(s) };
    let start = b.iter().take_while(|&&c| is_ws(c)).count();
    let mut end = b.len();
    while end > start && is_ws(b[end - 1]) {
        end -= 1;
    }
    unsafe { arc_string(&b[start..end]) }
}

/// `str_contains(haystack, needle)` — 1/0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_contains(haystack: *const c_char, needle: *const c_char) -> i64 {
    if haystack.is_null() || needle.is_null() {
        return 0;
    }
    i64::from(!unsafe { strstr(haystack, needle) }.is_null())
}

/// `str_starts_with(s, prefix)` — 1/0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_starts_with(s: *const c_char, prefix: *const c_char) -> i64 {
    if s.is_null() || prefix.is_null() {
        return 0;
    }
    let sb = unsafe { cstr_bytes(s) };
    let pb = unsafe { cstr_bytes(prefix) };
    i64::from(sb.starts_with(pb))
}

/// `str_ends_with(s, suffix)` — 1/0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_ends_with(s: *const c_char, suffix: *const c_char) -> i64 {
    if s.is_null() || suffix.is_null() {
        return 0;
    }
    let sb = unsafe { cstr_bytes(s) };
    let xb = unsafe { cstr_bytes(suffix) };
    i64::from(sb.ends_with(xb))
}

/// `str_upper(s)` — ASCII uppercase copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_upper(s: *const c_char) -> *mut c_char {
    if s.is_null() {
        return std::ptr::null_mut();
    }
    let b = unsafe { cstr_bytes(s) };
    let out: Vec<u8> = b
        .iter()
        .map(|&c| if (b'a'..=b'z').contains(&c) { c - 32 } else { c })
        .collect();
    unsafe { arc_string(&out) }
}

/// `str_lower(s)` — ASCII lowercase copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_lower(s: *const c_char) -> *mut c_char {
    if s.is_null() {
        return std::ptr::null_mut();
    }
    let b = unsafe { cstr_bytes(s) };
    let out: Vec<u8> = b
        .iter()
        .map(|&c| if (b'A'..=b'Z').contains(&c) { c + 32 } else { c })
        .collect();
    unsafe { arc_string(&out) }
}

/// `int_to_str(val)` — decimal, matching C `snprintf("%lld")`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_int_to_str(val: i64) -> *mut c_char {
    let s = val.to_string();
    unsafe { arc_string(s.as_bytes()) }
}

/// `u64_to_str(a)` — unsigned decimal, matching C `snprintf("%llu")`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_u64_to_str(a: i64) -> *mut c_char {
    let s = (a as u64).to_string();
    unsafe { arc_string(s.as_bytes()) }
}

/// `u64_to_hex(a)` — lowercase hex, matching C `snprintf("%llx")`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_u64_to_hex(a: i64) -> *mut c_char {
    let s = format!("{:x}", a as u64);
    unsafe { arc_string(s.as_bytes()) }
}

/// `float_to_str(val)` — C `snprintf("%g")`. Delegated to libc snprintf so
/// the output is byte-identical to the C reference (Rust's float Display
/// uses the shortest-round-trip form, which differs from %g).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_float_to_str(val: f64) -> *mut c_char {
    let mut buf = [0u8; 64];
    let n = unsafe {
        snprintf(
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len(),
            b"%g\0".as_ptr().cast::<c_char>(),
            val,
        )
    };
    let len = if n < 0 {
        0
    } else {
        (n as usize).min(buf.len() - 1)
    };
    unsafe { arc_string(&buf[..len]) }
}

/// `bool_to_str(val)` — "true"/"false". The parameter is `i8` (L++ `Bool`
/// lowers to i8); reading it wider made garbage high bits decide the branch
/// in the C history, so the ABI keeps it narrow.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_bool_to_str(val: i8) -> *mut c_char {
    let s = if val != 0 { "true" } else { "false" };
    unsafe { arc_string(s.as_bytes()) }
}

/// `str_to_int(s)` — `strtoll(s, NULL, 10)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_to_int(s: *const c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    unsafe { strtoll(s, std::ptr::null_mut(), 10) }
}

/// `str_to_u64(s)` — the C hand-rolled parser: skip leading whitespace, then
/// `0x`/`0X` hex or plain decimal, stopping at the first non-digit. All
/// accumulation wraps in u64 (matching C).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_to_u64(s: *const c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    let b = unsafe { cstr_bytes(s) };
    let mut i = 0;
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    let mut val: u64 = 0;
    if i + 1 < b.len() && b[i] == b'0' && (b[i + 1] == b'x' || b[i + 1] == b'X') {
        i += 2;
        while i < b.len() {
            let c = b[i];
            i += 1;
            let d = if (b'0'..=b'9').contains(&c) {
                u64::from(c - b'0')
            } else if (b'a'..=b'f').contains(&c) {
                u64::from(c - b'a') + 10
            } else if (b'A'..=b'F').contains(&c) {
                u64::from(c - b'A') + 10
            } else {
                break;
            };
            val = (val << 4) | d;
        }
    } else {
        while i < b.len() && (b'0'..=b'9').contains(&b[i]) {
            val = val.wrapping_mul(10).wrapping_add(u64::from(b[i] - b'0'));
            i += 1;
        }
    }
    val as i64
}

/// `str_eq(a, b)` — 1/0. Pointer-equal short-circuit, then byte compare
/// (matching C, including both-NULL == equal).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_eq(a: *const c_char, b: *const c_char) -> i64 {
    if a == b {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }
    i64::from(unsafe { cstr_bytes(a) } == unsafe { cstr_bytes(b) })
}

/// `parse_int(s)` — registry alias of `str_to_int`: parse a base-10 integer
/// from `s`. The ABI table gives `parse_int` its own symbol (`lpp_parse_int`)
/// but identical semantics, so this delegates to the existing implementation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_parse_int(s: *const c_char) -> i64 {
    unsafe { lpp_str_to_int(s) }
}

/// `str_len(s)` — byte length (0 for NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_str_len(s: *const c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    unsafe { strlen(s) as i64 }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::slice::{lpp_slice_init, lpp_str_slice_get, lpp_str_slice_to_str};
    use libc::c_void;

    fn mix(h: &mut i64, v: i64) {
        *h = (*h ^ v).wrapping_mul(1099511628211);
    }

    /// Mirror of the C scenario's `mixs`: NULL -> mix(-1); else mix the byte
    /// length then each byte (unsigned).
    fn mixs(h: &mut i64, s: *const c_char) {
        if s.is_null() {
            mix(h, -1);
            return;
        }
        let b = unsafe { cstr_bytes(s) };
        mix(h, b.len() as i64);
        for &byte in b {
            mix(h, i64::from(byte));
        }
    }

    /// The golden fingerprint of the string scenario, produced by the v1 C
    /// reference (`lpp_runtime.c`). The C side is pinned in
    /// `tests/runtime_gate.rs::c_reference_matches_the_string_golden`.
    const STR_GOLDEN: i64 = 5679634501925218854;

    fn scenario() -> i64 {
        let mut h: i64 = 1469598103934665603;
        unsafe {
            mixs(&mut h, lpp_str_concat(c"Hello, ".as_ptr(), c"world".as_ptr()));
            mix(&mut h, lpp_str_find(c"abcdef".as_ptr(), c"cd".as_ptr()));
            mix(&mut h, lpp_str_find(c"abcdef".as_ptr(), c"zz".as_ptr()));
            mixs(&mut h, lpp_str_replace(c"a-b-c".as_ptr(), c"-".as_ptr(), c"+".as_ptr()));
            mixs(&mut h, lpp_str_replace(c"aaa".as_ptr(), c"a".as_ptr(), c"bb".as_ptr()));
            mixs(&mut h, lpp_str_trim(c"  hi\t".as_ptr()));
            mix(&mut h, lpp_str_contains(c"hello".as_ptr(), c"ell".as_ptr()));
            mix(&mut h, lpp_str_contains(c"hello".as_ptr(), c"xyz".as_ptr()));
            mix(&mut h, lpp_str_starts_with(c"hello".as_ptr(), c"he".as_ptr()));
            mix(&mut h, lpp_str_ends_with(c"hello".as_ptr(), c"lo".as_ptr()));
            mixs(&mut h, lpp_str_upper(c"aBc".as_ptr()));
            mixs(&mut h, lpp_str_lower(c"aBc".as_ptr()));
            mixs(&mut h, lpp_int_to_str(-42));
            mixs(&mut h, lpp_int_to_str(i64::MAX));
            mixs(&mut h, lpp_int_to_str(0));
            mixs(&mut h, lpp_float_to_str(3.5));
            mixs(&mut h, lpp_float_to_str(0.1));
            mixs(&mut h, lpp_float_to_str(1e20));
            mixs(&mut h, lpp_float_to_str(-2.25));
            mixs(&mut h, lpp_bool_to_str(1));
            mixs(&mut h, lpp_bool_to_str(0));
            mix(&mut h, lpp_str_to_int(c"  -123abc".as_ptr()));
            mix(&mut h, lpp_str_to_int(c"9223372036854775807".as_ptr()));
            mix(&mut h, lpp_str_to_int(c"xyz".as_ptr()));
            mix(&mut h, lpp_str_to_u64(c"0xff".as_ptr()));
            mix(&mut h, lpp_str_to_u64(c"1234".as_ptr()));
            mix(&mut h, lpp_str_to_u64(c"0X1A".as_ptr()));
            mix(&mut h, lpp_str_eq(c"abc".as_ptr(), c"abc".as_ptr()));
            mix(&mut h, lpp_str_eq(c"abc".as_ptr(), c"abd".as_ptr()));
            mix(&mut h, lpp_str_eq(c"".as_ptr(), c"".as_ptr()));
            mix(&mut h, lpp_str_len(c"hello".as_ptr()));
            mix(&mut h, lpp_str_len(c"".as_ptr()));
            mixs(&mut h, lpp_u64_to_str(-1));
            mixs(&mut h, lpp_u64_to_str(0));
            mixs(&mut h, lpp_u64_to_hex(255));
            mixs(&mut h, lpp_u64_to_hex(0));
            mixs(&mut h, lpp_u64_to_hex(-1));

            // String slice over an ARC-allocated source string.
            let src = lpp_arc_alloc(16) as *mut u8;
            let bytes = b"abcdefg\0";
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), src, bytes.len());
            let mut sv = [0u64; 8]; // 64 bytes, 8-aligned (LppSlice is 40)
            let ss = lpp_slice_init(
                sv.as_mut_ptr().cast::<c_void>(),
                src.cast::<c_void>(),
                2,
                3,
                0,
            );
            mixs(&mut h, lpp_str_slice_to_str(ss));
            mixs(&mut h, lpp_str_slice_get(ss, 1));
        }
        h
    }

    #[test]
    fn string_scenario_is_golden() {
        assert_eq!(
            scenario(),
            STR_GOLDEN,
            "Rust string builtins diverge from the C reference"
        );
    }
}
