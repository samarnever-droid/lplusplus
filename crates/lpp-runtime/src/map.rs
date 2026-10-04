//! Hash map runtime — a Rust re-implementation of `runtime/lpp_map.c` with an
//! identical ABI.
//!
//! Open-addressing, linear-probing hash map supporting `Int` and `Str` keys and
//! `Int`/`Float`/ARC-object values. The map object itself is ARC-allocated (so
//! it participates in the same ownership discipline as lists and strings) and
//! carries a destructor that frees the entry table and releases any managed
//! values. Behaviour — hash functions, load factor, growth policy, tombstone
//! reuse, probe order — mirrors the C reference byte for byte so programs
//! observe the same iteration/þlookup semantics.
//!
//! Key encoding: an `Int` key is stored directly; a `Str` key stores the raw
//! pointer (`i64`) to the NUL-terminated payload and is compared with `strcmp`.
//! The `is_str_key` flag distinguishes the two so an integer key and a string
//! key that happen to share a bit pattern never collide.

use libc::{c_char, c_void, calloc, free, memcpy, strcmp, strlen};

use crate::arc::{lpp_arc_alloc, lpp_arc_alloc_with_destructor, lpp_arc_release, lpp_arc_retain};
use crate::list::{lpp_list_new, lpp_list_new_arc, lpp_list_push, lpp_list_push_arc};
use crate::panic::runtime_panic;

const OCCUPIED_EMPTY: i32 = 0;
const OCCUPIED_LIVE: i32 = 1;
const OCCUPIED_TOMB: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct LppMapEntry {
    key: i64,
    val: i64,
    is_str_key: i32,
    /// 0 = empty, 1 = occupied, 2 = deleted (tombstone).
    occupied: i32,
}

#[repr(C)]
struct LppMap {
    entries: *mut LppMapEntry,
    cap: i64,
    len: i64,
    /// 1 = values are ARC-managed pointers; retain on insert, release on
    /// overwrite / remove / destroy.
    arc_values: i32,
}

fn hash_str(s: *const c_char) -> u64 {
    if s.is_null() {
        return 0;
    }
    let mut hash: u64 = 14695981039346656037;
    let mut p = s.cast::<u8>();
    unsafe {
        while *p != 0 {
            hash ^= *p as u64;
            hash = hash.wrapping_mul(1099511628211);
            p = p.add(1);
        }
    }
    hash
}

fn hash_int(key: i64) -> u64 {
    let mut k = key as u64;
    k = (!k).wrapping_add(k << 21);
    k ^= k >> 24;
    k = k.wrapping_add(k << 3).wrapping_add(k << 8);
    k ^= k >> 14;
    k = k.wrapping_add(k << 2).wrapping_add(k << 4);
    k ^= k >> 28;
    k = k.wrapping_add(k << 31);
    k
}

unsafe fn entry_at(m: &LppMap, idx: i64) -> *mut LppMapEntry {
    unsafe { m.entries.add(idx as usize) }
}

/// ARC destructor for a map payload: release managed values, then free the
/// entry table.
unsafe extern "C" fn map_destroy(payload: *mut c_void) {
    if payload.is_null() {
        return;
    }
    let m = unsafe { &mut *(payload as *mut LppMap) };
    if m.arc_values != 0 && !m.entries.is_null() {
        for i in 0..m.cap {
            let e = unsafe { &*entry_at(m, i) };
            if e.occupied == OCCUPIED_LIVE {
                unsafe { lpp_arc_release(e.val as *mut c_void) };
            }
        }
    }
    if !m.entries.is_null() {
        unsafe { free(m.entries.cast::<c_void>()) };
    }
    m.entries = std::ptr::null_mut();
    m.cap = 0;
    m.len = 0;
}

unsafe fn alloc_entries(cap: i64) -> *mut LppMapEntry {
    let ptr = unsafe {
        calloc(
            cap.max(0) as libc::size_t,
            std::mem::size_of::<LppMapEntry>() as libc::size_t,
        )
    };
    ptr.cast::<LppMapEntry>()
}

unsafe fn new_with_mode(arc_values: i32) -> *mut c_void {
    let payload = unsafe {
        lpp_arc_alloc_with_destructor(std::mem::size_of::<LppMap>() as i64, Some(map_destroy))
    };
    if payload.is_null() {
        return std::ptr::null_mut();
    }
    let m = unsafe { &mut *(payload as *mut LppMap) };
    m.cap = 16;
    m.len = 0;
    m.arc_values = arc_values;
    m.entries = unsafe { alloc_entries(m.cap) };
    if m.entries.is_null() {
        runtime_panic("out of memory while allocating map");
    }
    payload
}

/// `map_new()` — a fresh map holding plain `i64` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_new() -> *mut c_void {
    unsafe { new_with_mode(0) }
}

/// `map_new_arc()` — a map whose values are ARC-managed pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_new_arc() -> *mut c_void {
    unsafe { new_with_mode(1) }
}

unsafe fn rehash(m: &mut LppMap, new_cap: i64) {
    let old_cap = m.cap;
    let old_entries = m.entries;

    m.cap = new_cap;
    m.entries = unsafe { alloc_entries(m.cap) };
    if m.entries.is_null() {
        runtime_panic("out of memory while rehashing map");
    }
    m.len = 0;

    for i in 0..old_cap {
        let src = unsafe { &*old_entries.add(i as usize) };
        if src.occupied == OCCUPIED_LIVE {
            let key = src.key;
            let val = src.val;
            let is_str = src.is_str_key;
            let h = if is_str != 0 {
                hash_str(key as *const c_char)
            } else {
                hash_int(key)
            };
            let mut idx = (h % m.cap as u64) as i64;
            while unsafe { &*entry_at(m, idx) }.occupied == OCCUPIED_LIVE {
                idx = (idx + 1) % m.cap;
            }
            let dst = unsafe { &mut *entry_at(m, idx) };
            dst.key = key;
            dst.val = val;
            dst.is_str_key = is_str;
            dst.occupied = OCCUPIED_LIVE;
            m.len += 1;
        }
    }
    unsafe { free(old_entries.cast::<c_void>()) };
}

unsafe fn put_internal(m: *mut LppMap, key: i64, val: i64, is_str: i32) {
    if m.is_null() {
        return;
    }
    let m = unsafe { &mut *m };

    let mut occupied_slots = 0i64;
    for i in 0..m.cap {
        if unsafe { &*entry_at(m, i) }.occupied != OCCUPIED_EMPTY {
            occupied_slots += 1;
        }
    }
    if occupied_slots * 10 >= m.cap * 7 {
        let new_cap = if m.len * 100 < m.cap * 35 {
            m.cap
        } else {
            m.cap * 2
        };
        let new_cap = if new_cap < 16 { 16 } else { new_cap };
        unsafe { rehash(m, new_cap) };
    }

    let h = if is_str != 0 {
        hash_str(key as *const c_char)
    } else {
        hash_int(key)
    };
    let mut idx = (h % m.cap as u64) as i64;
    let mut first_tombstone: i64 = -1;

    loop {
        let e = unsafe { &mut *entry_at(m, idx) };
        if e.occupied == OCCUPIED_EMPTY {
            break;
        }
        if e.occupied == OCCUPIED_LIVE && e.is_str_key == is_str {
            let matched = if is_str != 0 {
                unsafe { strcmp(e.key as *const c_char, key as *const c_char) == 0 }
            } else {
                e.key == key
            };
            if matched {
                if m.arc_values != 0 {
                    unsafe {
                        lpp_arc_retain(val as *mut c_void);
                        lpp_arc_release(e.val as *mut c_void);
                    }
                }
                e.val = val;
                return;
            }
        }
        if e.occupied == OCCUPIED_TOMB && first_tombstone == -1 {
            first_tombstone = idx;
        }
        idx = (idx + 1) % m.cap;
    }

    if first_tombstone != -1 {
        idx = first_tombstone;
    }

    if m.arc_values != 0 {
        unsafe { lpp_arc_retain(val as *mut c_void) };
    }
    let e = unsafe { &mut *entry_at(m, idx) };
    e.key = key;
    e.val = val;
    e.is_str_key = is_str;
    e.occupied = OCCUPIED_LIVE;
    m.len += 1;
}

/// `map_put(m, int_key, int_val)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_put(map: *mut c_void, key: i64, val: i64) {
    unsafe { put_internal(map as *mut LppMap, key, val, 0) };
}

/// `map_put(m, str_key, int_val)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_put_str(map: *mut c_void, key: *const c_char, val: i64) {
    unsafe { put_internal(map as *mut LppMap, key as i64, val, 1) };
}

/// Shared lookup body for integer keys. Returns the stored value or `0`.
unsafe fn get_int(map: *mut c_void, key: i64) -> i64 {
    if map.is_null() {
        return 0;
    }
    let m = unsafe { &*(map as *mut LppMap) };
    if m.len == 0 {
        return 0;
    }
    let mut idx = (hash_int(key) % m.cap as u64) as i64;
    let start = idx;
    loop {
        let e = unsafe { &*entry_at(m, idx) };
        if e.occupied == OCCUPIED_EMPTY {
            break;
        }
        if e.occupied == OCCUPIED_LIVE && e.is_str_key == 0 && e.key == key {
            return e.val;
        }
        idx = (idx + 1) % m.cap;
        if idx == start {
            break;
        }
    }
    0
}

unsafe fn get_str(map: *mut c_void, key: *const c_char) -> i64 {
    if map.is_null() || key.is_null() {
        return 0;
    }
    let m = unsafe { &*(map as *mut LppMap) };
    if m.len == 0 {
        return 0;
    }
    let mut idx = (hash_str(key) % m.cap as u64) as i64;
    let start = idx;
    loop {
        let e = unsafe { &*entry_at(m, idx) };
        if e.occupied == OCCUPIED_EMPTY {
            break;
        }
        if e.occupied == OCCUPIED_LIVE
            && e.is_str_key == 1
            && unsafe { strcmp(e.key as *const c_char, key) == 0 }
        {
            return e.val;
        }
        idx = (idx + 1) % m.cap;
        if idx == start {
            break;
        }
    }
    0
}

/// `map_get(m, int_key)` → value or `0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_get(map: *mut c_void, key: i64) -> i64 {
    unsafe { get_int(map, key) }
}

/// `map_get(m, str_key)` → value or `0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_get_str(map: *mut c_void, key: *const c_char) -> i64 {
    unsafe { get_str(map, key) }
}

/// `map_has(m, int_key)` → `1`/`0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_has(map: *mut c_void, key: i64) -> i64 {
    if map.is_null() {
        return 0;
    }
    let m = unsafe { &*(map as *mut LppMap) };
    if m.len == 0 {
        return 0;
    }
    let mut idx = (hash_int(key) % m.cap as u64) as i64;
    let start = idx;
    loop {
        let e = unsafe { &*entry_at(m, idx) };
        if e.occupied == OCCUPIED_EMPTY {
            break;
        }
        if e.occupied == OCCUPIED_LIVE && e.is_str_key == 0 && e.key == key {
            return 1;
        }
        idx = (idx + 1) % m.cap;
        if idx == start {
            break;
        }
    }
    0
}

/// `map_has(m, str_key)` → `1`/`0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_has_str(map: *mut c_void, key: *const c_char) -> i64 {
    if map.is_null() || key.is_null() {
        return 0;
    }
    let m = unsafe { &*(map as *mut LppMap) };
    if m.len == 0 {
        return 0;
    }
    let mut idx = (hash_str(key) % m.cap as u64) as i64;
    let start = idx;
    loop {
        let e = unsafe { &*entry_at(m, idx) };
        if e.occupied == OCCUPIED_EMPTY {
            break;
        }
        if e.occupied == OCCUPIED_LIVE
            && e.is_str_key == 1
            && unsafe { strcmp(e.key as *const c_char, key) == 0 }
        {
            return 1;
        }
        idx = (idx + 1) % m.cap;
        if idx == start {
            break;
        }
    }
    0
}

/// `map_len(m)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_len(map: *mut c_void) -> i64 {
    if map.is_null() {
        return 0;
    }
    unsafe { (*(map as *mut LppMap)).len }
}

/// `map_remove(m, int_key)` — tombstone the entry if present.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_remove(map: *mut c_void, key: i64) {
    if map.is_null() {
        return;
    }
    let m = unsafe { &mut *(map as *mut LppMap) };
    if m.len == 0 {
        return;
    }
    let mut idx = (hash_int(key) % m.cap as u64) as i64;
    let start = idx;
    loop {
        let e = unsafe { &mut *entry_at(m, idx) };
        if e.occupied == OCCUPIED_EMPTY {
            break;
        }
        if e.occupied == OCCUPIED_LIVE && e.is_str_key == 0 && e.key == key {
            if m.arc_values != 0 {
                unsafe { lpp_arc_release(e.val as *mut c_void) };
            }
            e.occupied = OCCUPIED_TOMB;
            m.len -= 1;
            return;
        }
        idx = (idx + 1) % m.cap;
        if idx == start {
            break;
        }
    }
}

/// `map_remove(m, str_key)` — tombstone the entry if present.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_remove_str(map: *mut c_void, key: *const c_char) {
    if map.is_null() || key.is_null() {
        return;
    }
    let m = unsafe { &mut *(map as *mut LppMap) };
    if m.len == 0 {
        return;
    }
    let mut idx = (hash_str(key) % m.cap as u64) as i64;
    let start = idx;
    loop {
        let e = unsafe { &mut *entry_at(m, idx) };
        if e.occupied == OCCUPIED_EMPTY {
            break;
        }
        if e.occupied == OCCUPIED_LIVE
            && e.is_str_key == 1
            && unsafe { strcmp(e.key as *const c_char, key) == 0 }
        {
            if m.arc_values != 0 {
                unsafe { lpp_arc_release(e.val as *mut c_void) };
            }
            e.occupied = OCCUPIED_TOMB;
            m.len -= 1;
            return;
        }
        idx = (idx + 1) % m.cap;
        if idx == start {
            break;
        }
    }
}

/// `map_put(m, int_key, float_val)` — the float is bit-packed into the slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_put_float(map: *mut c_void, key: i64, val: f64) {
    unsafe { lpp_map_put(map, key, val.to_bits() as i64) };
}

/// `map_get(m, int_key)` reinterpreted as a float.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_get_float(map: *mut c_void, key: i64) -> f64 {
    f64::from_bits(unsafe { lpp_map_get(map, key) } as u64)
}

/// `map_put(m, str_key, float_val)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_put_str_float(map: *mut c_void, key: *const c_char, val: f64) {
    unsafe { lpp_map_put_str(map, key, val.to_bits() as i64) };
}

/// `map_get(m, str_key)` reinterpreted as a float.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_get_str_float(map: *mut c_void, key: *const c_char) -> f64 {
    f64::from_bits(unsafe { lpp_map_get_str(map, key) } as u64)
}

/// `map_keys(m)` → a `List` of the live keys (a fresh ARC-owning list of string
/// copies when any key is a string, else a plain integer list).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_keys(map: *mut c_void) -> *mut c_void {
    if map.is_null() {
        return unsafe { lpp_list_new() };
    }
    let m = unsafe { &*(map as *mut LppMap) };
    if m.len == 0 {
        return unsafe { lpp_list_new() };
    }
    let mut has_str_key = false;
    for i in 0..m.cap {
        let e = unsafe { &*entry_at(m, i) };
        if e.occupied == OCCUPIED_LIVE && e.is_str_key != 0 {
            has_str_key = true;
            break;
        }
    }
    let lst = if has_str_key {
        unsafe { lpp_list_new_arc() }
    } else {
        unsafe { lpp_list_new() }
    };
    for i in 0..m.cap {
        let e = unsafe { &*entry_at(m, i) };
        if e.occupied != OCCUPIED_LIVE {
            continue;
        }
        if e.is_str_key != 0 {
            let s = e.key as *const c_char;
            let len = if s.is_null() {
                0
            } else {
                unsafe { strlen(s) as i64 }
            };
            let copy = unsafe { lpp_arc_alloc(len + 1) } as *mut c_char;
            if !copy.is_null() {
                if !s.is_null() && len > 0 {
                    unsafe {
                        memcpy(
                            copy.cast::<c_void>(),
                            s.cast::<c_void>(),
                            len as libc::size_t,
                        );
                    }
                }
                unsafe { *copy.add(len as usize) = 0 };
            }
            unsafe { lpp_list_push_arc(lst, copy.cast::<c_void>()) };
        } else {
            unsafe { lpp_list_push(lst, e.key) };
        }
    }
    lst
}

/// `map_values(m)` → a `List` of the live values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_values(map: *mut c_void) -> *mut c_void {
    if map.is_null() {
        return unsafe { lpp_list_new() };
    }
    let m = unsafe { &*(map as *mut LppMap) };
    if m.len == 0 {
        return unsafe { lpp_list_new() };
    }
    let lst = if m.arc_values != 0 {
        unsafe { lpp_list_new_arc() }
    } else {
        unsafe { lpp_list_new() }
    };
    for i in 0..m.cap {
        let e = unsafe { &*entry_at(m, i) };
        if e.occupied != OCCUPIED_LIVE {
            continue;
        }
        if m.arc_values != 0 {
            unsafe { lpp_list_push_arc(lst, e.val as *mut c_void) };
        } else {
            unsafe { lpp_list_push(lst, e.val) };
        }
    }
    lst
}

/// `map_clear(m)` — release managed values and zero the table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_clear(map: *mut c_void) {
    if map.is_null() {
        return;
    }
    let m = unsafe { &mut *(map as *mut LppMap) };
    if m.entries.is_null() {
        return;
    }
    if m.arc_values != 0 {
        for i in 0..m.cap {
            let e = unsafe { &*entry_at(m, i) };
            if e.occupied == OCCUPIED_LIVE {
                unsafe { lpp_arc_release(e.val as *mut c_void) };
            }
        }
    }
    for i in 0..m.cap {
        let e = unsafe { &mut *entry_at(m, i) };
        e.key = 0;
        e.val = 0;
        e.is_str_key = 0;
        e.occupied = OCCUPIED_EMPTY;
    }
    m.len = 0;
}

/// `map_capacity(m)` — the current entry-table size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_map_capacity(map: *mut c_void) -> i64 {
    if map.is_null() {
        return 0;
    }
    unsafe { (*(map as *mut LppMap)).cap }
}
