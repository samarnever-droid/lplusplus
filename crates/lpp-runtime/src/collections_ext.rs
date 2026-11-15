//! Extended runtime collections: `Set` (int + UTF-8 string keys) and `Deque` (ring-buffer).
//!
//! Backed by a mutex-guarded handle table (`i64` handles >= 1) so user programs
//! get O(1) amortized operations, deterministic handle safety on invalid/freed
//! handles, optional explicit cleanup (`set_free`/`deque_free`), and zero false
//! LeakSanitizer positives for short-lived programs.

use std::collections::{HashSet, VecDeque};
use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::{Mutex, OnceLock};

#[derive(Hash, PartialEq, Eq, Clone)]
enum SetKey {
    Int(i64),
    Bytes(Vec<u8>),
}

#[derive(Default)]
struct Store<T> {
    slots: Vec<Option<T>>,
    free_ids: Vec<usize>,
}

impl<T: Default> Store<T> {
    fn alloc(&mut self, value: T) -> i64 {
        if let Some(idx) = self.free_ids.pop() {
            self.slots[idx] = Some(value);
            (idx + 1) as i64
        } else {
            self.slots.push(Some(value));
            self.slots.len() as i64
        }
    }

    fn get(&self, handle: i64) -> Option<&T> {
        let idx = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.slots.get(idx)?.as_ref()
    }

    fn get_mut(&mut self, handle: i64) -> Option<&mut T> {
        let idx = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.slots.get_mut(idx)?.as_mut()
    }

    fn free(&mut self, handle: i64) {
        if let Ok(idx) = usize::try_from(handle.saturating_sub(1))
            && idx < self.slots.len()
            && self.slots[idx].take().is_some()
        {
            self.free_ids.push(idx);
        }
    }
}

fn sets() -> &'static Mutex<Store<HashSet<SetKey>>> {
    static SETS: OnceLock<Mutex<Store<HashSet<SetKey>>>> = OnceLock::new();
    SETS.get_or_init(|| Mutex::new(Store::default()))
}

fn deques() -> &'static Mutex<Store<VecDeque<i64>>> {
    static DEQUES: OnceLock<Mutex<Store<VecDeque<i64>>>> = OnceLock::new();
    DEQUES.get_or_init(|| Mutex::new(Store::default()))
}

unsafe fn read_c_bytes(ptr: *const c_char) -> Option<Vec<u8>> {
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec())
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_new() -> i64 {
    sets()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .alloc(HashSet::new())
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_add(handle: i64, value: i64) -> i8 {
    let mut guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_mut(handle)
        .is_some_and(|set| set.insert(SetKey::Int(value))) as i8
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_has(handle: i64, value: i64) -> i8 {
    let guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get(handle)
        .is_some_and(|set| set.contains(&SetKey::Int(value))) as i8
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_remove(handle: i64, value: i64) -> i8 {
    let mut guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_mut(handle)
        .is_some_and(|set| set.remove(&SetKey::Int(value))) as i8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_set_add_str(handle: i64, key: *const c_char) -> i8 {
    let Some(bytes) = (unsafe { read_c_bytes(key) }) else {
        return 0;
    };
    let mut guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_mut(handle)
        .is_some_and(|set| set.insert(SetKey::Bytes(bytes))) as i8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_set_has_str(handle: i64, key: *const c_char) -> i8 {
    let Some(bytes) = (unsafe { read_c_bytes(key) }) else {
        return 0;
    };
    let guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get(handle)
        .is_some_and(|set| set.contains(&SetKey::Bytes(bytes))) as i8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_set_remove_str(handle: i64, key: *const c_char) -> i8 {
    let Some(bytes) = (unsafe { read_c_bytes(key) }) else {
        return 0;
    };
    let mut guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_mut(handle)
        .is_some_and(|set| set.remove(&SetKey::Bytes(bytes))) as i8
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_len(handle: i64) -> i64 {
    let guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    guard.get(handle).map_or(0, |set| set.len() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_clear(handle: i64) {
    let mut guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(set) = guard.get_mut(handle) {
        set.clear();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_free(handle: i64) {
    sets()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .free(handle);
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_union(a: i64, b: i64) -> i64 {
    let mut guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    let mut out = guard.get(a).cloned().unwrap_or_default();
    if let Some(sb) = guard.get(b) {
        out.extend(sb.iter().cloned());
    }
    guard.alloc(out)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_set_intersect(a: i64, b: i64) -> i64 {
    let mut guard = sets().lock().unwrap_or_else(|e| e.into_inner());
    let out = match (guard.get(a), guard.get(b)) {
        (Some(sa), Some(sb)) => sa.intersection(sb).cloned().collect(),
        _ => HashSet::new(),
    };
    guard.alloc(out)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_new() -> i64 {
    deques()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .alloc(VecDeque::new())
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_push_back(handle: i64, value: i64) {
    let mut guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dq) = guard.get_mut(handle) {
        dq.push_back(value);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_push_front(handle: i64, value: i64) {
    let mut guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dq) = guard.get_mut(handle) {
        dq.push_front(value);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_pop_back(handle: i64) -> i64 {
    let mut guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_mut(handle)
        .and_then(VecDeque::pop_back)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_pop_front(handle: i64) -> i64 {
    let mut guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_mut(handle)
        .and_then(VecDeque::pop_front)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_peek_front(handle: i64) -> i64 {
    let guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get(handle)
        .and_then(VecDeque::front)
        .copied()
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_peek_back(handle: i64) -> i64 {
    let guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get(handle)
        .and_then(VecDeque::back)
        .copied()
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_get(handle: i64, idx: i64) -> i64 {
    let Ok(idx) = usize::try_from(idx) else {
        return 0;
    };
    let guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get(handle)
        .and_then(|dq| dq.get(idx).copied())
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_len(handle: i64) -> i64 {
    let guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    guard.get(handle).map_or(0, |dq| dq.len() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_clear(handle: i64) {
    let mut guard = deques().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dq) = guard.get_mut(handle) {
        dq.clear();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_deque_free(handle: i64) {
    deques()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .free(handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_deque_smoke() {
        let s1 = lpp_set_new();
        assert_eq!(lpp_set_add(s1, 10), 1);
        let s2 = lpp_set_new();
        assert_eq!(lpp_set_add(s2, 20), 1);
        let su = lpp_set_union(s1, s2);
        let si = lpp_set_intersect(s1, s2);
        assert_eq!(lpp_set_len(su), 2);
        assert_eq!(lpp_set_len(si), 0);
        lpp_set_free(s2);
        lpp_set_free(su);
        lpp_set_free(si);
        lpp_set_free(s1);

        let dq = lpp_deque_new();
        lpp_deque_push_back(dq, 2);
        lpp_deque_push_front(dq, 1);
        assert_eq!(lpp_deque_pop_front(dq), 1);
        assert_eq!(lpp_deque_pop_back(dq), 2);
        lpp_deque_free(dq);
    }
}
