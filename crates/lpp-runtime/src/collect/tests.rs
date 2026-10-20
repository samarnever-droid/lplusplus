//! Direct collector tests: hand-built ARC graphs with test trace
//! functions. All tests share the process-global registry/buffer, so
//! they serialize on `SERIAL` and assert on counter deltas.

use super::*;
use crate::arc::{lpp_arc_alloc_with_destructor, lpp_arc_release, lpp_arc_retain};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());
static DTOR_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Payload: two owned child slots (offsets 0 and 8), released by the
/// destructor and reported by the trace.
unsafe extern "C" fn test_destructor(payload: *mut c_void) {
    DTOR_COUNT.fetch_add(1, Ordering::Relaxed);
    let slots = payload as *mut *mut c_void;
    unsafe {
        lpp_arc_release(*slots);
        lpp_arc_release(*slots.add(1));
    }
}

unsafe extern "C" fn test_trace(payload: *mut c_void, ctx: *mut c_void) {
    let slots = payload as *mut *mut c_void;
    unsafe {
        lpp_trace_visit(ctx, *slots);
        lpp_trace_visit(ctx, *slots.add(1));
    }
}

unsafe fn node() -> *mut c_void {
    let p = unsafe { lpp_arc_alloc_with_destructor(16, Some(test_destructor)) };
    assert!(!p.is_null());
    unsafe { lpp_arc_mark_collectible(p, Some(test_trace)) };
    p
}

/// Store `child` into `parent`'s slot `slot`, retaining like generated
/// code does for a new owning edge.
unsafe fn link(parent: *mut c_void, slot: usize, child: *mut c_void) {
    unsafe {
        lpp_arc_retain(child);
        *(parent as *mut *mut c_void).add(slot) = child;
    }
}

fn collected() -> usize {
    lpp__cycle_collected_count.load(Ordering::Relaxed)
}

fn dtors() -> usize {
    DTOR_COUNT.load(Ordering::Relaxed)
}

#[test]
fn two_node_cycle_is_reclaimed_after_roots_drop() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (c0, d0) = (collected(), dtors());
    unsafe {
        let a = node();
        let b = node();
        link(a, 0, b);
        link(b, 0, a);
        // External owners drop: plain ARC stalls at 1 each (the cycle).
        lpp_arc_release(a);
        lpp_arc_release(b);
        assert_eq!(dtors() - d0, 0, "nothing freed before collection");
        lpp_arc_collect_cycles();
        assert_eq!(collected() - c0, 2, "both cycle members reclaimed");
        assert_eq!(dtors() - d0, 2, "each destructor ran exactly once");
    }
}

#[test]
fn cycle_with_live_external_root_survives_until_root_drops() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (c0, d0) = (collected(), dtors());
    unsafe {
        let a = node();
        let b = node();
        link(a, 0, b);
        link(b, 0, a);
        lpp_arc_release(b); // b's external owner drops; a's remains
        lpp_arc_collect_cycles();
        assert_eq!(collected() - c0, 0, "reachable cycle must survive");
        assert_eq!(dtors() - d0, 0);
        lpp_arc_release(a);
        lpp_arc_collect_cycles();
        assert_eq!(collected() - c0, 2, "unreachable now: reclaimed");
        assert_eq!(dtors() - d0, 2);
    }
}

#[test]
fn self_loop_is_reclaimed() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (c0, d0) = (collected(), dtors());
    unsafe {
        let a = node();
        link(a, 0, a);
        lpp_arc_release(a);
        lpp_arc_collect_cycles();
        assert_eq!(collected() - c0, 1);
        assert_eq!(dtors() - d0, 1);
    }
}

#[test]
fn acyclic_collectible_graph_frees_by_refcount_without_collector() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (c0, d0) = (collected(), dtors());
    unsafe {
        let a = node();
        let b = node();
        link(a, 0, b);
        lpp_arc_release(b);
        lpp_arc_release(a); // frees a, whose destructor frees b
        assert_eq!(dtors() - d0, 2, "plain ARC freed both promptly");
        lpp_arc_collect_cycles();
        assert_eq!(collected() - c0, 0, "collector had nothing to do");
    }
}

#[test]
fn garbage_cycle_releases_its_live_child_without_freeing_it() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (c0, d0) = (collected(), dtors());
    unsafe {
        let a = node();
        let b = node();
        let kid = node();
        link(a, 0, b);
        link(b, 0, a);
        link(a, 1, kid); // the cycle also owns `kid`
        lpp_arc_release(a);
        lpp_arc_release(b);
        // kid's external reference is still held by this test.
        lpp_arc_collect_cycles();
        assert_eq!(collected() - c0, 2, "only the cycle is reclaimed");
        assert_eq!(dtors() - d0, 2, "kid not destroyed with the cycle");
        lpp_arc_release(kid);
        assert_eq!(dtors() - d0, 3, "kid frees with its last owner");
    }
}

#[test]
fn three_cycle_and_duplicate_edges_are_reclaimed() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (c0, d0) = (collected(), dtors());
    unsafe {
        let a = node();
        let b = node();
        let c = node();
        link(a, 0, b);
        link(a, 1, b); // duplicate edge a -> b
        link(b, 0, c);
        link(c, 0, a);
        lpp_arc_release(a);
        lpp_arc_release(b);
        lpp_arc_release(c);
        lpp_arc_collect_cycles();
        assert_eq!(collected() - c0, 3);
        assert_eq!(dtors() - d0, 3);
    }
}

#[test]
fn threshold_triggers_collection_without_explicit_call() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (c0, d0) = (collected(), dtors());
    unsafe {
        // COLLECT_THRESHOLD is 512; 600 self-loops must start freeing
        // through the release path alone.
        for _ in 0..600 {
            let a = node();
            link(a, 0, a);
            lpp_arc_release(a);
        }
        assert!(
            collected() - c0 >= 512,
            "threshold collection ran ({} reclaimed so far)",
            collected() - c0
        );
        lpp_arc_collect_cycles();
        assert_eq!(collected() - c0, 600);
        assert_eq!(dtors() - d0, 600);
    }
}

#[test]
fn mark_and_collect_tolerate_null_and_foreign_pointers() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        lpp_arc_mark_collectible(std::ptr::null_mut(), Some(test_trace));
        let mut buf = [0u8; 64];
        lpp_arc_mark_collectible(buf.as_mut_ptr().cast::<c_void>(), Some(test_trace));
        lpp_arc_collect_cycles();
        lpp_trace_visit(std::ptr::null_mut(), std::ptr::null_mut());
    }
}

/// Deterministic property test (fixed-seed xorshift, no external fuzz
/// infrastructure): random owned graphs over 48 nodes, half of them
/// kept reachable from live roots. Properties asserted:
///   * collection NEVER frees a node reachable from a live root
///     (destructor count stays put while roots are held, and the
///     rooted nodes still read back),
///   * once every root drops, every node is destroyed exactly once
///     (destructor count == node count: no leak, no double free),
///   * collection is idempotent (a second explicit round frees 0
///     additional nodes — implied by the exact destructor count).
#[test]
fn random_graphs_never_free_live_and_always_reclaim_dead() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _trial in 0..24 {
        let (c0, d0) = (collected(), dtors());
        const N: usize = 48;
        unsafe {
            let nodes: Vec<*mut c_void> = (0..N).map(|_| node()).collect();
            // Tag each node by writing its index into a spare byte the
            // destructor/trace ignore (payload slot area is only the
            // two child pointers; use refcount-independent identity:
            // pointer equality after the fact instead).
            for i in 0..N {
                for slot in 0..2 {
                    if next() % 2 == 0 {
                        let j = (next() as usize) % N;
                        link(nodes[i], slot, nodes[j]);
                    }
                }
            }
            // Model reachability from the first half (the roots we
            // keep) by walking child slots test-side.
            let mut reachable: std::collections::HashSet<usize> =
                nodes[..N / 2].iter().map(|&p| p as usize).collect();
            let mut work: Vec<*mut c_void> = nodes[..N / 2].to_vec();
            while let Some(p) = work.pop() {
                for slot in 0..2 {
                    let child = *(p as *mut *mut c_void).add(slot);
                    if !child.is_null() && reachable.insert(child as usize) {
                        work.push(child);
                    }
                }
            }
            // Drop the local reference of the second half and collect.
            for &p in &nodes[N / 2..] {
                lpp_arc_release(p);
            }
            lpp_arc_collect_cycles();
            // Exactly the unreachable nodes are gone (by plain
            // refcounting and/or the collector); exactly the reachable
            // ones survive with a live refcount.
            assert_eq!(
                dtors() - d0,
                N - reachable.len(),
                "trial {_trial}: freed set must equal the unreachable set"
            );
            for &addr in &reachable {
                let count =
                    (*crate::arc::refcount_cell(addr as *const c_void)).load(Ordering::Acquire);
                assert!(count >= 1, "trial {_trial}: reachable node was collected");
            }
            // Drop the roots: now EVERYTHING must go, exactly once.
            for &p in &nodes[..N / 2] {
                lpp_arc_release(p);
            }
            lpp_arc_collect_cycles();
            assert_eq!(
                dtors() - d0,
                N,
                "trial {_trial}: every node destroyed exactly once"
            );
            // At least the cycle members (if the random graph had
            // unreachable cycles) went through the collector or plain
            // refcounting; the collective accounting is the exact
            // destructor count above.
            let _ = collected() - c0;
        }
    }
}
