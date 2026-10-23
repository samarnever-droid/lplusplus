# ARC cycle breaking — investigation, root cause, and architecture

Status: implemented and validated for the Cranelift/native path. §1 records
the investigation and root cause; §2 describes that shipped design exactly as
implemented. WASM allocation reuse and the remaining WASM/LLVM cycle-collector
gaps are called out explicitly below.

## 1. Investigation note

### Observed failure

1. **MIR oracle (the reported error).** Any program that builds a runtime
   reference cycle through *struct* fields fails the ARC interpreter's
   end-of-execution balance proof with `E4304 OwnershipLeak { nodes: N }`
   when executed via `lpp_mir::execute_mir_arc` with the ownership plan's
   pinned set. Minimal reproduction (`struct Node { id, next: Node }`,
   `a.next = b; b.next = a`): the plan reports `cycles=0`,
   `pinned=[]`, the aggregate node is `owned []` (no containment edges),
   and execution ends with `OwnershipLeak { nodes: 2 }`. Self-loops,
   3-cycles, and mutual `Child <-> Parent` cycles fail identically.
   Acyclic chains of the same types pass.
2. **Native leak.** The same programs compile and print correct results
   through the Cranelift backend, but cyclic garbage is never reclaimed:
   200 000 iterations of a two-node cycle grow peak RSS to 20.9 MB vs
   10.8 MB for the acyclic control (same allocation count). Nothing
   crashes; the objects simply leak.
3. **Immortal acyclic values in the oracle.** Recursive *enum* types are
   detected as cycles, but the interpreter then refuses to free **any**
   value of those types — an acyclic two-element recursive-enum list ends
   with `frees=0, pinned_live=3`. Type-level pinning leaks ordinary data.

### Expected behaviour

L++'s documented model (v1 engine, `README.md`, `ARCHITECTURE.md`,
`documentation/CURRENT_CAPABILITIES.md`) is that reference cycles are
*handled*: cyclic programs compile, run, and are reclaimed — not rejected
and not leaked. The v1 engine implemented this with static edge demotion
plus arenas (`src/analysis/cyclebreak.rs`, removed in the v0.1 rewrite
cutover `8d8f6da`). The rewrite kept the claims but not the mechanism.

### Root cause

Three compounding defects, none of them in the error message itself:

* **RC1 — the containment graph has no struct-field edges.**
  `crates/lpp-ownership/src/plan.rs` (`build_graph`, `Aggregate` arm) and
  the independent recomputation in `crates/lpp-ownership/src/verify.rs`
  walk only `aggregate_variants` + `variant_fields`. In MIR, struct fields
  live in `MirAggregate.fields` (`program.aggregate_fields()`); only enum
  payloads live under variants (`crates/lpp-mir/src/builder/aggregate.rs`).
  Every struct node therefore has an empty edge list, no struct SCC is
  ever found, and nothing struct-shaped is classified `Shared`. This was
  recorded as a known deferral in `docs/rewrite/PHASE_5.md` ("the 4D plan
  graph carries no struct-field edges … `pinned_types() = ∅` for a struct
  self-cycle; such a program fails the oracle's balance proof with
  `E4304`") and shipped in the cutover.

* **RC2 — no reclamation mechanism consumes `Shared`.** The plan is a
  classification only. The interpreter "handles" `Shared` by *pinning*:
  releases are absorbed, nodes are never freed, survivors are reported as
  `pinned_live` instead of reclaimed (`crates/lpp-mir/src/interpret/`).
  The native backends never see the plan at all: the driver computes and
  proves it, then discards it (`crates/lpp-driver/src/compile.rs`
  `prove_ownership` returns `()`), the `Backend` trait has no plan input,
  and the Cranelift backend emits uniform plain ARC — so runtime cycles
  leak silently. PHASE_5 states this plainly: the object "retains the
  cycle with plain ARC (no special-case code)".

* **RC3 — the plan verifier cannot catch RC1.** `verify.rs` duplicates
  the same variant-only walk, so the E4402 proof agrees with the planner
  on exactly the dimension that is wrong. The 4D exit gate only exercised
  closure/list cycles, never struct cycles.

### Modules and crates involved

| Component | Role in the failure |
|---|---|
| `crates/lpp-ownership` (`plan.rs`, `verify.rs`) | Static containment graph + SCC classification; missing struct edges (RC1, RC3). Correct home for the *analysis* fix. |
| `crates/lpp-mir` interpreter (`execute_mir_arc`) | ARC oracle; pins instead of reclaiming (RC2); source of the user-visible `E4304`. |
| `crates/lpp-driver` | Proves then discards the plan; backends recompute what they need. |
| `crates/lpp-codegen-cranelift` | Emits ARC traffic; knows nothing about `Shared` (RC2). Must emit collectible markings + trace functions. |
| `crates/lpp-runtime` (`arc.rs`, `list.rs`, `tuple.rs`, `task.rs`) | Native ARC; plain refcounting only. Correct home for the *collector*. |
| `lpp_runtime.c`, `runtime/*_min.c` | C reference + freestanding runtimes; same objects link against any of them, so the collector ABI must exist in all. |
| `crates/lpp-codegen-wasm` | WASM's ARC heap has size-class free bins and reuses released blocks; 100 and 100,000 same-shape churn iterations both stay at 1 page in `arc_heap_reuses_freed_blocks_under_repeated_managed_churn`. This bounds tested acyclic churn only; WASM trial-deletion is not implemented, so unreachable cycles still leak. |
| `crates/lpp-codegen-llvm` | LLVM cycle collection and managed struct/list/tuple parity are not implemented/verified; its supported subset retains plain-ARC limitations. |

### Proposed architectural correction (implemented as §2)

Keep each responsibility where the rewrite already put it; complete the
missing half of the design instead of replacing it:

1. **Analysis (`lpp-ownership`).** Add struct-field edges
   (`aggregate_fields`) to the containment graph in both the planner and
   the verifier's independent recomputation. `Shared` then means what
   the docs say: the type can participate in a value cycle.
2. **Reclamation (runtime + oracle).** Implement Bacon–Rajan
   *trial-deletion* cycle collection over `Shared` (collectible) objects
   only. All edges stay strong; plain refcounting still frees everything
   acyclic promptly (including acyclic values of recursive types); the
   collector reclaims exactly the unreachable cyclic garbage. No edge is
   demoted, so the v1 demotion hazards (dangling weak reads, the arena +
   generation machinery needed to make them safe, `head` not retaining
   `tail` across a return) do not return. Compile-time checks (E4403
   use-after-move, MIR verification) are untouched: the collector is a
   runtime mechanism and bypasses no static rule.
3. **Codegen (Cranelift).** Recompute the ownership plan, generate a
   trace function beside each destructor for `Shared` aggregates/closure
   environments, mark `Shared` allocations collectible, and run a final
   collection from the generated `main` wrapper. Threshold-triggered
   collection inside the runtime bounds memory during long runs.
4. **No new crate.** The collector is runtime state (refcounts, buffers,
   registries) and belongs with the ARC core in `lpp-runtime` (and its C
   mirrors); the analysis already has a crate. A separate crate would
   split the refcount invariants across a boundary for no benefit.


## 2. Final design (as implemented)

### 2.1 Division of responsibilities

| Layer | Responsibility |
|---|---|
| `lpp-ownership` (plan + verify) | Static classification only. Builds the type-containment graph (struct fields **and** enum variant fields, closure captures, list/task/tuple elements), finds SCCs, and labels every type that can sit on a value cycle `Shared` (exposed as `pinned_types()`; the name is historical). No code is generated here and no program is rejected for being cyclic. |
| `lpp-mir` interpreter | The ARC oracle. Models honest refcounts for all types; collectible objects are buffered as candidate roots and reclaimed by the same trial-deletion algorithm. `E4304 OwnershipLeak` now means a genuine accounting bug (a survivor with an unexplained reference), never "a cycle exists". |
| `lpp-codegen-cranelift` | Recomputes the plan during its scan. For each `Shared` aggregate/closure-environment it generates a trace function `lpp_trace_{s,e,c}{id}` mirroring the generated destructor's walk, inserts `lpp_arc_mark_collectible` calls at the allocation sites that can produce a `Shared` value, and ends the generated `main` wrapper with `lpp_arc_collect_cycles()`. Programs with no `Shared` types get **no** collector imports and byte-identical plain-ARC codegen. |
| Runtimes (`lpp-runtime`, `lpp_runtime.c`, `runtime/linux_x86_64_min.c`, `runtime/windows_x86_64_min.c`, gate shim `crates/lpp-codegen-cranelift/tests/c_shim.c`) | Own the collector: collectible flags in the ARC header, candidate buffering in the release path, the trace registry, and trial deletion. Runtime-owned shapes (list, task, closure capsule, managed tuple) supply their own traces (`lpp_list_trace`, `lpp_task_trace`, `lpp_closure_trace`, `lpp_tuple_trace`). |

`Owned` types (the complement of `Shared`) are provably unable to sit on
a value cycle — a value cycle implies a containment cycle through the
fields/elements that realize it — so plain refcounting is complete for
them and they never touch collector state.

**Leak diagnosis.** The oracle no longer stops at counting survivors.
`execute_mir_arc_diagnosed` additionally returns a `LeakReport` that
recomputes every live node's refcount from the owning heap graph (the
same edges deallocation releases, flattened through inline tuples) and
classifies each survivor: `PhantomRoot` (count unexplained by the
graph — missed or duplicated ARC traffic in the model), `HeldByPhantom`
(alive only through a phantom root), `ClosedGarbage` (counts fully
explained internally — unreachable garbage the collector could not
take, e.g. a cycle whose type the plan failed to mark `Shared`), and
`DanglingTarget` (a live edge into an already-dead node). The summary
rides in `OwnershipLeak { nodes, phantom, closed }` (code `E4304`
unchanged); the report carries each node's allocation type, kind,
refcount vs recomputed internal refs, and collectibility — so a leak
failure names its cause instead of just its size.

### 2.2 The algorithm: Bacon–Rajan trial deletion

Reference counting frees an object when its count reaches zero. A cycle
keeps every member's count positive forever, reachable or not. Trial
deletion (Bacon & Rajan 2001; the same algorithm as CPython's cyclic gc)
answers "could this subgraph be kept alive only by itself?" without
tracing the whole heap:

1. **Candidate buffering.** When a release leaves a *collectible* object
   alive (new count > 0), that object *may* head a cycle that just became
   unreachable, so it is buffered once (a `BUFFERED` header flag dedupes)
   as `(pointer, generation)`. The generation lets the collector reject
   an address that was freed and recycled in the meantime. Non-collectible
   releases pay one flags load and nothing else.
2. **Trigger.** A round runs when the buffer reaches 512 candidates, or
   when `lpp_arc_collect_cycles()` is called explicitly — the generated
   `main` wrapper does so before returning, and the interpreter does so
   in its end-of-execution proof. Rounds repeat while freeing garbage
   buffers new candidates (a cycle held only by another cycle).
3. **Mark gray.** From the still-valid candidates, walk collectible edges
   (via the trace functions) building the candidate subgraph. Each node
   gets a *scratch count* = refcount − (number of incoming edges from
   inside the subgraph). Multi-edges subtract multiply, matching the
   retains that created them. Children that are not collectible are not
   part of the subgraph: they are freed by plain refcounting if their
   owner dies.
4. **Scan.** A node with scratch count > 0 has a reference from *outside*
   the subgraph (a local, a global, an acyclic parent, another thread's
   counted reference) and is live; liveness propagates along its edges.
   This is the reachable/unreachable distinction: no root set is ever
   enumerated — external hold is *proved* arithmetically from refcounts.
5. **Collect.** The remaining nodes form unreachable garbage (cycles and
   everything hanging only off them). They are torn down in three
   phases — first *invalidate* every garbage header (generation bumped,
   validity cleared, trace deregistered, and in the freestanding
   runtimes the refcount parked far below zero), then run every
   destructor, then release the storage. Because invalidation precedes
   destruction, a destructor releasing a fellow garbage member is a
   no-op: every destructor runs exactly once, storage is freed exactly
   once, and acyclic children leaving the garbage set are released
   normally by the destructors. Garbage destructors run in a
   deterministic (address/slot) order; no ordering between cycle members
   is observable because members can no longer be reached from live code.

Fail-safe choices throughout: a node whose trace is unavailable is
treated as externally held (kept alive); a failed trace or allocation
failure mid-round aborts the round having freed nothing. The collector
can only ever err toward leaking, never toward freeing live memory.

All walks are iterative (explicit worklists), so deep graphs cannot
overflow the stack *in the collector*; ordinary destructor cascades of
deep acyclic structures remain the language's pre-existing behaviour.

### 2.3 Runtime mechanics

* **Header flags.** Two bits, `COLLECTIBLE` and `BUFFERED`. Rust runtime
  and `lpp_runtime.c` keep them in a dedicated word at header offset 12
  (formerly padding; the header stays 24 bytes, `_Static_assert`s pin
  the layout). The freestanding runtimes have no spare word:
  `linux_x86_64_min.c` steals the top two bits of `map_size` (size
  classes are 1..8, whole-region sizes page-rounded; `lpp_arc_free`
  masks them out, capping one whole-region object at 1 GiB), and
  `windows_x86_64_min.c` steals bits 62/61 of `allocation_size` (bit 63
  is its arena flag) and puts a generation in the padding after
  `refcount` (layout unchanged). Generated code never reads headers, so
  one object file still links against any runtime.
* **Trace registry.** Maps a collectible object to its trace function.
  The Rust runtime keys it by *destructor address* (one entry per type:
  each collectible type has one destructor and one trace walking the
  same fields), so the free path does no registry work. The C runtimes
  key it by object pointer with a small open-addressing map and
  deregister on free. Either way the registry holds no references and
  cannot keep objects alive.
* **Locking.** The hosted runtimes guard collector state with a mutex
  (Rust) / spinlock (C, same pattern as the arena lock); destructors run
  outside the lock so their releases can re-enter the candidate buffer,
  and a `collecting` flag defers nested triggers. Refcounts stay atomic.
  Collection assumes the collectible graph is not being mutated by
  another thread mid-round; acyclic ARC guarantees are unaffected. The
  freestanding runtimes take no lock (linux-min exposes no thread
  primitive; windows-min follows the same assumption for collectible
  graphs).
* **Counters.** `lpp__runtime_drop_count` (all normal frees) and
  `lpp__cycle_collected_count` (collector frees) are exported by every
  runtime for gates and diagnostics; the Cranelift gate asserts executed
  native drop counts equal the interpreter oracle's.

### 2.4 Complexity and measured performance

Buffering is O(1) amortized per surviving release of a collectible
object; a round is O(nodes + edges) over the *candidate subgraph* only —
never a heap walk. Non-collectible traffic costs one extra flags load
per release. Programs without `Shared` types contain no collector calls
at all.

Release-build measurements (200 000 iterations, best of 3, this
machine; "before" is the pre-fix compiler, whose cycle case leaked):

| Program | Before | After | Peak RSS before → after |
|---|---|---|---|
| 2-node cycle churn | 0.027 s (leaking) | 0.115 s | 20.7 MB → 11.0 MB |
| Acyclic values of a recursive type | 0.025 s | 0.049 s | 11.0 MB → 11.0 MB |
| Non-recursive struct churn (no `Shared`) | 0.011 s | 0.011 s | 11.0 MB → 11.0 MB |

The acyclic-recursive slowdown is the mark + candidate buffering on
collectible types (the standard cost of synchronous cycle detection,
cf. CPython); it shrinks as real work per object grows. Correctness was
not traded for speed anywhere: the collector's failure mode under OOM
is to collect nothing.

### 2.5 Covered patterns and limits

Covered wherever the language can construct them: self-loops, 2- and
3+-node cycles, multi-field/multi-edge cycles, diamonds, cycles inside
larger graphs, multiple independent cycles, cycles kept alive by
external roots (never collected early — pinned by test), cycles
becoming unreachable after root drop, cycles through lists, closure
environments/captures (interpreter-validated; see limits), tasks and
managed tuples as cycle members, repeated create/destroy churn, and
acyclic tails hanging off cycles (freed exactly once).

Known limits, none introduced by this work:

* **Cranelift closure-origin limit — FIXED (2026-10-20, follow-up).** A
  closure that captures a list it is then stored into (`list_push(xs, f)`
  where `f` captures `xs`) used to be rejected with `E5001 "function
  value of unknown origin"`: function-value provenance was tracked
  intraprocedurally in instruction order, so it died at the capture
  boundary. The pre-scan now resolves provenance with a whole-program
  constraint fixpoint (`resolve_value_classes`: constants + equality
  propagation across copies, list push/get/set, list literals, and
  closure captures). Validated by `tests/cycle_closure_env.lpp`
  (call-through-the-cycle) in AOT parity, plus flat-RSS native churn.
  Provenance through struct fields and direct call arguments/returns
  remains at the previous boundary (caller-supplied capsule default).
* **Cranelift structural tuples — FIXED (2026-10-20, tuple drop
  glue).** Tuple records used to carry a null destructor and were
  never retained/released, leaking the record and its managed
  elements. A structural tuple is now a first-class ARC value:
  `is_managed` includes `TypeKind::Tuple`, each constructed tuple
  type gets a generated destructor (`lpp_drop_t{n}`, releasing the
  managed element slots) and — when the tuple type is `Shared` — a
  generated trace (`lpp_trace_t{n}`) with collector marking at
  construction. Sound because tuples are immutable after
  construction (the frontend rejects element assignment), so sharing
  the record under refcounting matches the oracle's inline-aggregate
  value semantics; every ownership path (copies, overwrites, exit
  pass, aggregate/closure/task destructors) already keyed off
  `is_managed` and now covers tuples uniformly, including cycles
  routed through tuple types (`N -> H -> (N, Int) -> N`). Validated
  by `tests/tuple_{managed_churn,overwrite,cycle,async_closure}.lpp`
  (AOT parity + ASan/LSan-clean native binaries, flat RSS at 100x
  churn) and the oracle test `cycle_through_structural_tuple_is_collected`.
  Structural tuples are now accepted as Cranelift list elements and are
  retained/released through the managed-element path; `tuple_list_{churn,cycle,alias}.lpp`
  cover destruction, cycle tracing, and alias-safe replacement. WASM/LLVM tuple
  list-element parity remains unsupported/unverified, and tuple arity remains
  2–4.
* **WASM allocation reuse — PARTIAL.** The runtime uses size-class ARC free
  bins rather than relying only on bump-heap growth. The focused WASM heap
  test measured 1 page at both 100 and 100,000 same-shape churn iterations.
  This proves reuse for the tested acyclic allocations, not collection of
  unreachable cycles: WASM cycle collection remains unimplemented. **LLVM**
  managed aggregate/collector parity is also unimplemented and unverified.
* **windows_x86_64_min.c** received the same port (its header gained a
  generation in existing padding). *Update (2026-10-20, follow-up):*
  actually compiled with mingw-w64 GCC and Clang (mingw target) — which
  caught and fixed a real bug where two `&header->refcount` sites had
  been mangled by the port's `lpp__rc_load` rewrite — and executed
  under Wine: a C harness exercising the collector (2-cycle, live-root
  survival, acyclic promptness, 5000 self-loop churn) passes with exact
  destructor/collected/drop counts. The object was mingw-built rather
  than MSVC-built (CI's `cl.exe` job remains the MSVC authority), but
  the source is now proven C-clean and behaviourally correct.
* The ownership plan *verifier* (`verify.rs`) now walks the same
  complete graph as the planner; it remains a recomputation with shared
  assumptions rather than a fully independent proof (RC3 is mitigated —
  the graph is correct — not eliminated as a structural concern).

### 2.6 Extending

To make a new runtime-owned shape collectible: give it a trace that
reports exactly the owned ARC children its destructor releases, export
the trace from every runtime, add the name to
`tests/test_runtime_parity.rs`, and mark the shape's allocations at the
codegen sites where a `Shared` type can produce it. To support a new
container in the plan, add its containment edges in both `plan.rs` and
`verify.rs`. Anything not classified `Shared` must remain provably
acyclic — that invariant is what lets the collector ignore most of the
heap.

### 2.7 Validation map

* `crates/lpp-ownership/tests/cycle_breaking.rs` — 15 oracle-level
  regression tests (all patterns above, exact `collected`/`frees`
  accounting).
* `crates/lpp-runtime` `collect::tests` — hand-built graph unit tests
  incl. exactly-once destruction.
* `crates/lpp-codegen-cranelift` gates — linked executions against the C
  shim collector, drop counts equal to the oracle, export/import census
  including `lpp_trace_*`.
* `tests/cycle_{self_loop,ring,list_ring,churn}.lpp` +
  `tests/run_aot_parity.sh` — surface programs on hosted C and
  freestanding Linux runtimes.
* ASan/LSan clean and flat RSS (11 MB hosted Rust, 8.2 MB linux-min) over
  200 000 churned cycles with a live cycle held throughout.
