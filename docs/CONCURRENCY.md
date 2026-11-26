# L++ concurrency — the current contract

_Last audited: 2026-10-20 against the tree (Rust runtime `concur.rs`,
Cranelift spawn lowering, MIR ownership/move-out passes). This is the
honest contract: what is guaranteed today (with the test/evidence
level), what is explicitly not yet guaranteed, and the rules left to
build before any data-race-freedom claim (Safety Mission S4)._

## 1. What exists

| Facility | Surface | Runtime |
|---|---|---|
| OS threads | `spawn fn() ...:`, `spawn callback`, `lpp_thread_join` | `std::thread` (host Rust runtime); pthread in `lpp_runtime.c`. **Freestanding min runtimes have no thread primitive** — `spawn` fails to link there by design. |
| Async tasks | `async def`, `.await` | Multi-threaded worker-pool executor (`2..=16` worker threads) with cooperative caller stealing on `.await` and atomic task completion synchronization across both Rust and C hosted runtimes. Static `OwnershipBalancePass` rejects unsynchronized concurrent writes across aliased `async def` parameters; OS I/O readiness reactor (`epoll`/`kqueue`/`IOCP`) integration remains future work. |
| Atomics | `atomic_new/load/store/add/sub/and/or/xor/swap/cas` (+ acq/rel/relaxed variants, 32- and 64-bit) | Opaque i64 handles over host atomics (`Ordering::SeqCst` unless the relaxed variants are chosen). |
| Locks | mutex / rwlock builtins | Host-backed opaque handles (`concur.rs`). |

## 2. Guaranteed today

1. **Refcounts and ARC header magic are always atomic.** `lpp_arc_retain` /
   `lpp_arc_release` use AcqRel atomic fetch-add/sub on the header and
   Acquire/Release atomic loads/stores on `magic_cell` in every Rust runtime
   path, eliminating non-atomic header races under multi-threaded churn.
2. **Ownership transfer at spawn.** A spawned closure's captures are
   either *transferred* (the move-out pass proves the spawning thread
   never touches the value again — one owner before, one after) or
   *shared by retain* (both threads hold counted references; either
   may drop last safely). A value is never freed while a thread
   still holds a counted reference.
3. **Static `Send` / `Sync` & concurrent-write enforcement (`E4403`).**
   `OwnershipPlan` marks values and types reaching `spawn`/`async` boundaries
   as `concurrent` (distinct from cycle-`Shared`), and `OwnershipBalancePass`
   rejects both:
   - Unsynchronized mutable writes to concurrently-shared references
     (`UnsynchronizedConcurrentWrite`, unless mutex/rwlock-guarded or uniquely
     transferred), and
   - Non-`Send` transfers (`NonSendTransfer`: `Ptr[T]`, `RawPtr`, `Slice[T]`,
     `StrSlice`, or aggregates/containers transitively containing them) across
     `spawn`, `thread_spawn`, and `async def` boundaries on Cranelift, LLVM,
     and WASM.
4. **Collector-exclusion protocol.** `lpp-runtime`'s Bacon-Rajan cycle
   collector (`collect.rs` + `arc.rs`) uses a global `COLLECTOR_LOCK: RwLock<()>`
   with thread-local reentrancy tracking: mutator releases on `COLLECTIBLE`
   (`Shared`) objects take a shared read guard while `lpp_arc_collect_cycles`
   holds the exclusive write guard, preventing concurrent trial deletion from
   racing with collectible frees. Acyclic `Owned` objects never touch the lock.
   Validated with 8-thread concurrent cycle-collection stress tests.
5. **Task results are transferred, not shared-mutably.** `.await`
   produces a fresh counted reference to the result; task envs are
   released exactly once whether polled to completion or dropped.

## 3. Explicitly NOT yet guaranteed

1. **No data-race freedom for shared mutable fields.** If two threads
   hold references to the same struct and both store to a field, the
   language today provides refcount safety (no UAF/double-free) but
   the field stores race like C. There is no static
   Send/Sync-style rule preventing this yet — §4 is the plan. Until
   then: treat any value reachable from two threads as either
   immutable, atomic-only, or lock-protected. **Do not write "L++ is
   data-race-free" anywhere.**
2. **Collector vs concurrent graph mutation.** Trial deletion reads
   refcounts and walks child pointers; another thread mutating the
   *same* graph mid-collection can make the scratch accounting
   stale. The fail direction is toward *leaking* (a candidate whose
   generation changed is skipped), and refcounts themselves stay
   atomic — but "never freed early under concurrent mutation" is an
   engineering argument, not a proven one. Multi-threaded programs
   that churn shared cyclic graphs from several threads are outside
   the S2 verified subset.
3. **No fairness/starvation/scheduling guarantees** for `spawn` or
   the executor; async tasks are run-to-completion on the calling
   thread and a long task blocks its executor.
4. **Panics kill the process** (exit 101), on any thread — there is
   no per-thread panic isolation or join-error channel. See
   `PANIC_AND_ERRORS.md`.

## 4. Rules to build (the path to a Send/Sync-style model)

In order; each lands with oracle tests first (ROADMAP §3 house rule):

1. **Classify sharing in the ownership plan**: values that reach a
   `spawn`/task boundary get a `Concurrent` mark (the plan already
   distinguishes `Frame < Owned < Shared`; cycle-`Shared` and
   thread-`Shared` must become separate facts — today they are
   conflated in the lattice history).
2. **Reject mutable-field access through a concurrently-shared
   reference** unless the access is inside an atomic or lock
   operation, or the value is proven uniquely owned (count provably
   1 via the ownership verifier).
3. **Collector exclusion**: define whether collection is
   stop-the-world, per-thread (thread-local candidate buffers with
   handshakes), or restricted to the main thread — then prove the
   chosen protocol against concurrent mutation with TSan stress
   tests.
4. Only after 1–3: revisit the wording rules in
   `Safety_Mission.md` for any data-race claim (S3/S4 evidence).
