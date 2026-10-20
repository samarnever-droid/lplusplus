//! Trial-deletion cycle collection for ARC (Bacon & Rajan), the
//! runtime half of L++'s static cycle breaking.
//!
//! The ownership plan (`lpp-ownership`) classifies every type that can
//! participate in a reference cycle as `Shared`. Generated code marks
//! each `Shared` allocation *collectible* here, together with a trace
//! function that enumerates its owned ARC children. Everything else
//! keeps plain refcounting and frees deterministically at zero.
//!
//! Collectible objects are also refcounted normally — an acyclic value
//! of a recursive type frees promptly at zero and never touches this
//! module. Only when a release leaves a collectible object alive can
//! it be part of a garbage cycle, so that release buffers it as a
//! possible cycle root. A collection (threshold-triggered, or explicit
//! via `lpp_arc_collect_cycles`) then proves which buffered subgraphs
//! are referenced solely from themselves and reclaims exactly those.
//!
//! The algorithm never scans roots (no stack walk): liveness is proven
//! from refcounts alone. Trial deletion subtracts one scratch count
//! per edge inside the candidate subgraph; a node whose scratch count
//! stays positive is referenced from outside and keeps its whole
//! reachable subgraph alive. The remainder is freed together, with the
//! magic cleared first so the members' destructors release only
//! references that leave the garbage set.
//!
//! Concurrency: refcounts stay atomic; the registry, the root buffer,
//! and collections are serialized by mutexes, and at most one
//! collection runs at a time. The S2 safety boundary
//! (`documentation/Safety_Mission.md`) already excludes unsynchronized
//! cross-thread transfer/mutation of the same graph.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use libc::{c_void, free};

use crate::arc::{
    destructor_cell, flags_cell, generation_cell, header_of, is_immortal, is_valid_arc_ptr,
    lpp__runtime_drop_count, magic_cell, refcount_cell,
};
use crate::layout::{ARC_FLAG_BUFFERED, ARC_FLAG_COLLECTIBLE};

/// Enumerates the owned ARC children of one collectible payload by
/// calling `lpp_trace_visit(ctx, child)` once per owning edge
/// (duplicates included). Generated per aggregate/closure-env by the
/// compiler; the runtime provides `lpp_list_trace`, `lpp_tuple_trace`,
/// `lpp_task_trace`, and `lpp_closure_trace` for its own types.
pub type LppTraceFn = unsafe extern "C" fn(*mut c_void, *mut c_void);

/// Buffered possible roots that trigger an opportunistic collection.
const COLLECT_THRESHOLD: usize = 512;

/// Registered trace function per collectible payload address.
static REGISTRY: Mutex<Option<HashMap<usize, usize>>> = Mutex::new(None);
/// Possible cycle roots: (payload address, generation when buffered).
/// The generation defeats address reuse: a stale entry whose object
/// was freed and recycled cannot match the new occupant.
static ROOT_BUFFER: Mutex<Vec<(usize, i32)>> = Mutex::new(Vec::new());
/// At most one collection at a time; nested triggers (releases issued
/// by destructors during collection) defer to the next round.
static COLLECTING: AtomicBool = AtomicBool::new(false);

/// Observability for gates and tests: nodes reclaimed by the collector.
#[unsafe(no_mangle)]
pub static lpp__cycle_collected_count: AtomicUsize = AtomicUsize::new(0);

fn with_registry<R>(f: impl FnOnce(&mut HashMap<usize, usize>) -> R) -> R {
    let mut guard = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// The registry is keyed by *destructor address*, not by object: each
/// collectible type has exactly one generated destructor and one
/// trace walking the same fields, so one entry serves every instance.
/// That keeps the registry proportional to the type count, makes
/// marking idempotent, and — crucially — removes all registry traffic
/// from the free path (see `deregister`). Objects without a destructor
/// (not producible by current codegen for collectible types) fall
/// back to their own address as key; a heap address can never equal
/// a destructor address because the text page holding the destructor
/// is itself mapped.
fn registry_key(ptr: *mut c_void) -> usize {
    match unsafe { *destructor_cell(ptr) } {
        Some(destructor) => destructor as usize,
        None => ptr as usize,
    }
}

/// Mark a fresh ARC allocation collectible and register its trace
/// function. Called by generated code for every allocation whose type
/// the ownership plan classified `Shared`. Tolerates null, foreign,
/// and immortal pointers exactly like the retain/release entry points.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_arc_mark_collectible(ptr: *mut c_void, trace: Option<LppTraceFn>) {
    let Some(trace) = trace else { return };
    if !is_valid_arc_ptr(ptr) || is_immortal(ptr) {
        return;
    }
    let key = registry_key(ptr);
    with_registry(|registry| {
        registry.entry(key).or_insert(trace as usize);
    });
    unsafe {
        (*flags_cell(ptr)).fetch_or(ARC_FLAG_COLLECTIBLE, Ordering::AcqRel);
    }
}

/// Remove a collectible object from the registry (normal free path).
/// Destructor-keyed entries are per-type and stay for the process
/// lifetime, so the common case is a no-op that never touches the
/// lock; only destructor-less fallback entries are removed per object.
pub(crate) fn deregister(ptr: *mut c_void) {
    if unsafe { *destructor_cell(ptr) }.is_some() {
        return;
    }
    with_registry(|registry| {
        registry.remove(&(ptr as usize));
    });
}

/// Buffer a collectible object as a possible cycle root after a
/// release that did not free it. No-op for unmarked objects, so plain
/// ARC traffic on acyclic types pays only one flags load.
pub(crate) fn note_possible_root(ptr: *mut c_void) {
    let flags = unsafe { (*flags_cell(ptr)).load(Ordering::Acquire) };
    if flags & ARC_FLAG_COLLECTIBLE == 0 || flags & ARC_FLAG_BUFFERED != 0 {
        return;
    }
    let previous = unsafe { (*flags_cell(ptr)).fetch_or(ARC_FLAG_BUFFERED, Ordering::AcqRel) };
    if previous & ARC_FLAG_BUFFERED != 0 {
        return; // another thread buffered it first
    }
    let generation = unsafe { (*generation_cell(ptr)).load(Ordering::Acquire) };
    let due = {
        let mut buffer = ROOT_BUFFER.lock().unwrap_or_else(|e| e.into_inner());
        buffer.push((ptr as usize, generation));
        buffer.len() >= COLLECT_THRESHOLD
    };
    if due {
        unsafe { lpp_arc_collect_cycles() };
    }
}

/// The trace sink: trace functions report each owned child here.
/// `ctx` is collector-internal state (a `*mut Vec<usize>`); trace
/// functions pass it through untouched. Validity filtering happens in
/// the collector core, so reporting a null or foreign child is safe.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_trace_visit(ctx: *mut c_void, child: *mut c_void) {
    if ctx.is_null() || child.is_null() {
        return;
    }
    let sink = unsafe { &mut *(ctx as *mut Vec<usize>) };
    sink.push(child as usize);
}

/// Run trial deletion until the root buffer drains (releases issued by
/// garbage destructors can expose further garbage, e.g. a cycle held
/// only by another cycle). Called explicitly from the generated `main`
/// wrapper for programs with `Shared` types, and automatically when
/// the buffer reaches `COLLECT_THRESHOLD`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_arc_collect_cycles() {
    if COLLECTING.swap(true, Ordering::AcqRel) {
        return; // a collection is already running (possibly ours)
    }
    loop {
        let freed = unsafe { collect_round() };
        let pending = ROOT_BUFFER.lock().map(|b| b.len()).unwrap_or(0);
        if freed == 0 || pending == 0 {
            break;
        }
    }
    COLLECTING.store(false, Ordering::Release);
}

/// One trial-deletion round over the currently buffered roots.
/// Returns how many nodes were reclaimed.
unsafe fn collect_round() -> usize {
    let candidates: Vec<(usize, i32)> = {
        let mut buffer = ROOT_BUFFER.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *buffer)
    };
    if candidates.is_empty() {
        return 0;
    }

    // Validate roots and seed the gray set. An entry is usable only if
    // it still names the same live collectible object.
    let mut scratch: HashMap<usize, i64> = HashMap::new();
    let mut stack: Vec<usize> = Vec::new();
    for (addr, generation) in candidates {
        let ptr = addr as *mut c_void;
        if !is_valid_arc_ptr(ptr) || is_immortal(ptr) {
            continue;
        }
        let flags = unsafe { (*flags_cell(ptr)).fetch_and(!ARC_FLAG_BUFFERED, Ordering::AcqRel) };
        if flags & ARC_FLAG_COLLECTIBLE == 0 {
            continue;
        }
        if unsafe { (*generation_cell(ptr)).load(Ordering::Acquire) } != generation {
            continue; // stale entry: the address was recycled
        }
        let count = unsafe { (*refcount_cell(ptr)).load(Ordering::Acquire) };
        if count <= 0 || scratch.contains_key(&addr) {
            continue;
        }
        if with_registry(|registry| !registry.contains_key(&registry_key(ptr))) {
            continue; // no trace: can never be proven garbage
        }
        scratch.insert(addr, count as i64);
        stack.push(addr);
    }

    // Mark-gray: subtract one scratch count per internal edge.
    let mut children: Vec<usize> = Vec::new();
    while let Some(addr) = stack.pop() {
        children.clear();
        trace_children(addr, &mut children);
        for child in children.drain(..) {
            let child_ptr = child as *mut c_void;
            if !is_valid_arc_ptr(child_ptr) || is_immortal(child_ptr) {
                continue;
            }
            let flags = unsafe { (*flags_cell(child_ptr)).load(Ordering::Acquire) };
            if flags & ARC_FLAG_COLLECTIBLE == 0 {
                continue; // acyclic-typed child: freed by refcount, not by us
            }
            let count = unsafe { (*refcount_cell(child_ptr)).load(Ordering::Acquire) };
            if count <= 0 {
                continue;
            }
            match scratch.entry(child) {
                std::collections::hash_map::Entry::Occupied(mut entry) => *entry.get_mut() -= 1,
                std::collections::hash_map::Entry::Vacant(entry) => {
                    if with_registry(|registry| !registry.contains_key(&registry_key(child_ptr))) {
                        // Untraceable collectible: treat as externally
                        // held so nothing behind it is reclaimed.
                        entry.insert(1);
                    } else {
                        entry.insert(count as i64 - 1);
                        stack.push(child);
                    }
                }
            }
        }
    }
    if scratch.is_empty() {
        return 0;
    }

    // Scan: externally referenced nodes (scratch > 0) are live roots;
    // everything reachable from them (through collectible edges) lives.
    let mut garbage: HashSet<usize> = scratch.keys().copied().collect();
    let mut live_stack: Vec<usize> = scratch
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(addr, _)| *addr)
        .collect();
    while let Some(addr) = live_stack.pop() {
        if !garbage.remove(&addr) {
            continue;
        }
        children.clear();
        trace_children(addr, &mut children);
        for child in children.drain(..) {
            if garbage.contains(&child) {
                live_stack.push(child);
            }
        }
    }
    if garbage.is_empty() {
        return 0;
    }

    // Collect: invalidate every garbage header and deregister first —
    // a member's destructor releasing another member must be a no-op —
    // then run destructors (which release references leaving the set)
    // and free the blocks with the same accounting as a normal drop.
    let mut doomed: Vec<(usize, Option<unsafe extern "C" fn(*mut c_void)>)> =
        Vec::with_capacity(garbage.len());
    for addr in &garbage {
        let ptr = *addr as *mut c_void;
        let destructor = unsafe { *destructor_cell(ptr) };
        deregister(ptr);
        unsafe {
            (*generation_cell(ptr)).fetch_add(1, Ordering::Release);
            magic_cell(ptr).write_volatile(0);
            (*flags_cell(ptr)).store(0, Ordering::Release);
        }
        doomed.push((*addr, destructor));
    }
    // Deterministic teardown order (address order) across runs.
    // Destructors first, storage second: a member destructor that
    // releases another member must land on that member's intact
    // (invalidated) header, never on recycled memory.
    doomed.sort_unstable_by_key(|(addr, _)| *addr);
    for (addr, destructor) in &doomed {
        let ptr = *addr as *mut c_void;
        lpp__runtime_drop_count.fetch_add(1, Ordering::Relaxed);
        lpp__cycle_collected_count.fetch_add(1, Ordering::Relaxed);
        if let Some(destructor) = destructor {
            unsafe { destructor(ptr) };
        }
    }
    for (addr, _) in doomed {
        let ptr = addr as *mut c_void;
        unsafe { free(header_of(ptr)) };
    }
    garbage.len()
}

/// Run one node's registered trace into `out`. A node without a trace
/// reports no children (callers treat it conservatively).
fn trace_children(addr: usize, out: &mut Vec<usize>) {
    let trace = with_registry(|registry| registry.get(&registry_key(addr as *mut c_void)).copied());
    let Some(trace) = trace else { return };
    let trace: LppTraceFn = unsafe { std::mem::transmute(trace) };
    let ctx = out as *mut Vec<usize> as *mut c_void;
    unsafe { trace(addr as *mut c_void, ctx) };
}

#[cfg(test)]
mod tests;
