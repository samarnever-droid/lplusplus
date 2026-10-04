# Rewrite optimization strategy

Optimization is a first-class rewrite requirement, not a cleanup phase after
compatibility. Every phase must improve or preserve both compiler efficiency
and generated-program quality while retaining v1 behavior. Correctness remains
the hard constraint: no optimization may guess about types, effects, aliasing,
ownership, target support, or ABI properties.

This document is the cross-phase optimization contract.

## 1. Optimization goals

We optimize four different products and measure them separately:

1. **Frontend/compiler throughput** — wall time, CPU time, peak RSS, allocation
   count, and repeated-build latency.
2. **Generated programs** — runtime, allocation traffic, retained bytes, code
   size, startup time, and system calls.
3. **Build artifacts** — deterministic object/executable size, relocation
   count, duplicate data, and link time.
4. **Developer feedback** — incremental invalidation, diagnostics latency, and
   LSP query latency.

A runtime speedup that doubles compiler memory is not automatically accepted;
a compile-time shortcut that weakens ownership verification is never accepted.
Results are recorded by optimization profile and target.

## 2. Non-negotiable rules

- Preserve documented v1 source, runtime ABI, layout, and observable behavior.
- Run MIR/type/ownership verification before and after transformations.
- Derive optimization facts from typed IR and generated ABI effects; never
  match builtin behavior by unverified string names.
- Every optimization needs a positive test, a non-applicable test, and an
  ownership/effect regression where relevant.
- Observable iteration and output remain deterministic.
- Optimizer bugs fail compilation; they do not silently fall back to a
  different backend or reinterpret a program.
- Compile-time and code-size budgets prevent unbounded inlining,
  specialization, loop unrolling, or trait solving.
- Unsafe implementation techniques require a measured benefit and a separately
  reviewed invariant. The default implementation remains safe Rust.

## 3. Optimization profiles

The rewrite exposes one shared profile to HIR, MIR, ownership, codegen, and the
linker:

| Profile | Intent | Main transformations |
|---|---|---|
| `O0` | diagnostics/debug | canonical lowering only; no semantic rewrites |
| `O1` | fast development | local folding, CFG cleanup, copy/DCE, ARC pairs |
| `O2` | release default | interprocedural summaries, inlining, SCCP, GVN, escape, LICM, bounds elimination |
| `O3` | throughput | higher specialization/inlining/unroll budgets, vectorization candidates |
| `Os` | small binaries | size-costed inlining, dead stripping, pooling; no growth-heavy transforms |
| `Oz` | minimum size | aggressive outlining/pooling and minimal specialization |

Profile selection is represented by typed options, not environment checks
scattered through stages. Backends may refine machine-level choices but cannot
change language semantics.

## 4. Compiler data and memory optimization

These changes begin immediately in frontend/HIR phases:

- Keep one `Arc<str>` source buffer per file. A token is 16 bytes and contains
  only kind and byte span, not a separately allocated copy of token text.
- Use compact typed `u32` IDs and contiguous arenas for symbols, definitions,
  scopes, expressions, statements, types, blocks, values, and instances.
- Intern identifier text, module components, type shapes, signatures, and
  monomorphization keys.
- Arena-based HIR/MIR references replace recursive `Box` trees and repeated
  node clones. Traversal becomes cache-friendly and IDs remain stable.
- Reserve capacities from parser/module counts and reuse scratch vectors in hot
  passes.
- Use hash maps only for non-observable lookup; emitted order is always stable
  ID/order-map order.
- Parse every module once. Remap temporary file identities after deterministic
  graph ordering instead of lexing and parsing a second time.
- Share source `Arc`s between the frontend and `SourceMap`.
- Avoid quadratic path/name concatenation. Keep interned components and format
  only at diagnostics/output boundaries.
- Make recursive graph/type algorithms iterative or depth-bounded where an
  adversarial package can exhaust the process stack.

Phase 2 originally retained token-owned `String`s and reparsed graph modules.
The Phase 3 kickoff removes both costs before additional HIR volume is added.

## 5. Frontend optimization

- Linear UTF-8 scanning with byte offsets and no per-character collection.
- Compact tokens and zero-copy token text views into the shared source.
- Trivia remains lossless but semantic passes consume filtered index views.
- Replace per-line parser allocations with reusable or borrowed ranges.
- Incremental reparse boundaries are declarations and indentation blocks;
  fingerprints include source bytes, edition, and parser version.
- Cache literal decoding separately from lossless spelling.
- Keep parser recovery bounded to avoid quadratic malformed-input behavior.
- Fuzz and benchmark deeply nested delimiters, long identifiers, comment-heavy
  files, large generated modules, and invalid indentation.

## 6. Package graph and incremental build optimization

- Canonical, case-sensitive paths are graph identities.
- Ordered work queues and stable IDs make cache keys reproducible.
- Cache filesystem metadata and import resolution within one build.
- Hash source contents, compiler/edition/options, dependency interfaces, target,
  and runtime ABI version. Never key only by modification time.
- Split module fingerprints into public interface and implementation hashes so
  private body edits do not invalidate unrelated dependents.
- Parse independent modules in parallel, then merge by stable path order.
- Store reverse dependencies for targeted invalidation.
- Cache stage outputs independently: parse, resolved HIR, typed HIR,
  monomorphized instances, optimized MIR, and object code.
- Content-addressed cache writes are atomic and checksummed; cache corruption is
  a miss, not compiler input.

## 7. HIR, name resolution, types, generics, and traits

Phase 3 must implement efficiency with correctness from the start:

### HIR and resolution

- Intern every name once and resolve through compact `Symbol`, `DefId`, and
  `LocalId` values.
- Use module-local indexed lookup plus stable ordered definition storage; a
  hash lookup may be added only with a separately stable observable order.
- Maintain real module namespaces internally. Edition-1 flattening is a
  compatibility view, not destructive declaration copying.
- Index imports, exports, methods, fields, variants, and generic parameters;
  never scan every declaration for each lookup.
- Arena-based expressions/statements preserve source origins without recursive
  object allocation.

### Type inference

- Canonically intern types as `TypeId`; equal types have equal IDs.
- Use union-find with path compression/rank for inference variables.
- Keep occurs checks and level/generalization metadata explicit.
- Cache normalized substitutions and structural layout queries.
- Represent primitive/container heads compactly so common comparisons avoid
  recursive shape walking.

### Traits

- Index implementations by `(TraitId, self-type head)`.
- Memoize canonical trait goals with explicit in-progress cycle detection.
- Reject ambiguous/overflowing searches deterministically.
- Apply depth, candidate, and normalization budgets with diagnostics rather
  than exponential hangs.
- Derive devirtualization candidates only after coherence and exact receiver
  type are proven.

### Generics and specialization

- Monomorphize on demand from reachable roots.
- Intern stable `InstanceKey { DefId, substitutions }` values and compile each
  instance once.
- Process instance worklists in deterministic key order.
- Share polymorphic bodies where representation and ABI permit; specialize only
  when a measured runtime/code-size model benefits.
- Cap recursive instantiation depth and total specialization growth.

### Early semantic optimization

- Evaluate language constants and fold type-level expressions in typed HIR.
- Record precise effects (`pure`, allocation, IO, blocking, panic, mutation),
  aliasing, and ownership intent for MIR consumers.
- Resolve direct calls and trait targets early, but perform behavior-changing
  transforms only in verified MIR.

## 8. MIR representation and optimization pipeline

MIR uses typed CFG blocks, compact values/locals, explicit calls/effects, and
explicit ownership operations. A pass manager declares prerequisites,
preserved analyses, invalidation, and profile eligibility.

Recommended order (repeated to fixed point only where bounded):

1. **Canonicalization** — split critical edges, normalize calls/branches,
   remove unreachable blocks, and simplify block parameters.
2. **Constant folding and SCCP** — propagate constants through branches and
   eliminate unreachable paths without evaluating side effects.
3. **Algebraic simplification** — apply overflow-, float-, and trap-aware
   identities only under language rules.
4. **Copy propagation and DCE** — remove dead pure values/stores while retaining
   calls, panics, ownership, volatile, and IO effects.
5. **GVN/CSE** — merge equivalent pure computations using typed operands and
   memory/effect versions.
6. **Call graph and summaries** — compute purity, read/write, allocation,
   capture, throw/panic, blocking, and ownership summaries.
7. **Devirtualization** — replace trait/dynamic calls only for proven unique
   targets.
8. **Costed inlining** — profile-specific budgets, recursion guards, cold/hot
   weighting, and code-size accounting.
9. **Escape/SROA** — scalar-replace aggregates and stack-promote allocations
   that cannot escape or participate in required identity.
10. **Bounds/range analysis** — eliminate checks only when integer range and
    container length proofs dominate the access.
11. **Loop pipeline** — canonical loops, LICM for invariant pure operations,
    induction simplification, strength reduction, and small bounded unrolling.
12. **Closure optimization** — remove unused captures, choose by-value/by-ref
    captures from ownership facts, and direct-call non-escaping closures.
13. **Async optimization** — minimize state-machine fields, remove immediately
    ready suspension points, and reject unsafe blocking paths.
14. **Ownership planning** — choose frame/stack/arena/owned/shared placement,
    break owning cycles, and insert required ARC operations.
15. **ARC optimization** — move elision, retain/release pairing, borrow
    forwarding, local non-atomic ARC, release sinking, and loop traffic removal.
16. **Late CFG/DCE** — clean artifacts while preserving finalized ownership.
17. **Backend legalization** — lower only target-independent operations that the
    common MIR contract assigns to legalization.

Validation runs after every pass in debug/CI and at analysis boundaries in
release builds. Optimization equivalence tests execute pre/post MIR with the
same inputs where a MIR interpreter supports the feature.

## 9. Ownership-specific performance

Ownership is both a safety system and a primary performance area:

- Escape analysis is flow-sensitive enough to distinguish return, capture,
  thread handoff, FFI, container storage, and temporary borrows.
- Stack promotion removes allocation and ARC only when object identity/lifetime
  remain valid.
- Arena placement is region-proven; arenas cannot hide unbounded growth.
- Thread-local values use non-atomic reference counts until a proven handoff.
- Last-use analysis turns copies into moves and shortens live ranges.
- Borrowed parameters/results avoid retain traffic under explicit lifetime
  contracts.
- ARC pair elimination is CFG-aware and verified on every exit/unwind edge.
- Cycle breaking is deterministic and validated against the owning graph.

Metrics include allocations, retains, releases, atomic operations, promoted
objects, arena bytes, and peak live managed memory.

## 10. Backend optimization

### Common backend contract

- Consume only validated optimized MIR, target capabilities, layouts, and ABI
  descriptors.
- Carry proven attributes such as non-null, no-alias, readonly, no-unwind, and
  alignment. Never invent them backend-locally.
- Keep stable symbol mangling and deterministic function/data ordering.

### Cranelift

- Map rewrite profiles to explicit Cranelift optimization settings.
- Preserve direct calls, block structure, aliases, and stack slots so Cranelift
  can schedule/register-allocate effectively.
- Use target features only when requested/probed by `TargetSpec`.
- Verify generated objects and compare hot-function disassembly in benchmarks.

### LLVM

- Emit accurate attributes, TBAA/alias scopes, range metadata, calling
  conventions, and target features from proven facts.
- Keep the common MIR optimizer authoritative for semantics; LLVM performs
  machine/general IR optimization after correct lowering.
- Validate at `-O0/-O2/-Oz` and use sanitizers in differential tests.

### WebAssembly

- Minimize locals and stack spills, deduplicate imports/data, structure CFGs
  deterministically, and use bulk-memory/SIMD only when target capabilities
  permit.
- Run `wasm-validate`-equivalent checks and track encoded byte size.

## 11. Runtime and ABI optimization

ABI compatibility constrains representation changes but does not prohibit
internal improvement:

- Generate effect/ownership metadata with signatures and consume it in passes.
- Pool immutable strings and canonical constants per artifact.
- Use size-class/slab allocation where measured, with overflow and alignment
  checks.
- Specialize common list/map/string paths while retaining generic ABI adapters.
- Avoid repeated UTF-8 scans by caching only where lifetime and mutation rules
  make cache validity explicit.
- Buffer IO and batch syscalls without changing flushing/ordering semantics.
- Separate atomic and local ARC implementations behind generated ABI-safe
  wrappers.
- Keep platform calls in thin adapters and share optimized core algorithms.
- Benchmark host and freestanding runtimes separately.

Any layout change requires an ABI version or compatibility adapter.

## 12. Linker and artifact optimization

- Parse symbol/section tables once and index lookups.
- Batch relocations by section and use checked linear writes.
- Dead-strip unreachable functions/data from explicit roots.
- Merge identical constants/strings and optionally identical functions only
  when address identity permits.
- Emit function/data sections for external linker GC where appropriate.
- Order hot/cold sections deterministically when profile data is explicitly
  supplied.
- Parallelize independent object parsing but serialize deterministic layout.
- Track output bytes, padding, relocation counts, symbols, and link time.

## 13. Measurement and regression gates

The checked-in legacy baseline anchors are:

- `benchmarks/scalability/latest.{json,md}` for 10k/50k/100k-line compiler
  phase scaling;
- `benchmarks/king20/stable/v1/latest.{json,md}` for compile, AOT, link,
  runtime, object, and executable measurements over 20 programs;
- `benchmarks/aot_profiles/latest.{json,md}` for backend profile time/size;
- `benchmarks/workloads/latest.{json,md}` for workload-shape coverage;
- `benchmarks/comparison/latest.{json,md}` for cross-language context.

Historical results are behavior/performance anchors, not valid direct deltas
across different hosts. Rewrite before/after claims must use the same pinned
input, toolchain, target, host, warmup, and repetition protocol. The Phase 3A
structural baseline is enforced in tests: 16-byte tokens, four-byte IDs and
four-byte optional IDs, eight-byte operand ranges, bounded non-owning HIR
nodes, one interned name allocation, one source allocation shared by syntax and
`SourceMap`, and one parse per discovered file.

Every optimization change records a before/after result using pinned inputs.
The gate suite contains:

- microbenchmarks for lexer, parser, interning, resolution, type inference,
  trait lookup, MIR passes, codegen, and linker;
- compile-time scaling at 10k/50k/100k source lines;
- cold build, warm build, one-module body edit, and public-interface edit;
- runtime suites for scalar loops, allocations/ARC, strings, lists/maps,
  closures, traits/generics, async, IO, and real packages;
- code-size suites for minimal, representative, and generic-heavy programs;
- repeated-output hashes for HIR, MIR, objects, lockfiles, and diagnostics;
- pre/post optimization differential execution and property tests;
- sanitizer/leak tests for optimized ownership paths.

Initial budget policy:

- no unexplained compatibility or deterministic-output change;
- no statistically significant runtime regression above 3% on a stable suite;
- no compile-time or peak-RSS regression above 5% unless an approved tradeoff
  records a larger generated-code win;
- no release code-size regression above 3% without profile-specific evidence;
- specialization/inlining growth is always capped.

Small noisy changes are rerun; budgets are not bypassed with `|| true`.

## 14. Delivery by phase

| Rewrite phase | Optimization delivered with the phase |
|---|---|
| Phase 2/3 kickoff | shared source buffers, compact tokens/IDs, one-pass module parsing, interned names, typed arenas |
| Phase 3 | arena HIR and operand lists, origin-preserving lowering, indexed resolution, type interning, union-find inference, memoized traits, demand-driven stable monomorphization, interface fingerprints |
| Phase 4 | typed MIR, verifier/pass manager, SCCP/GVN/DCE/inlining/loops, escape/SROA, ownership and ARC optimization |
| Phase 5 | profile-aware Cranelift/LLVM/WASM lowering, proven attributes, deterministic optimized objects |
| Phase 6 | runtime allocation/data-path work, pooling/dead stripping, indexed and parallel linker internals |
| Phase 7 | incremental query/cache engine, parallel project builds, low-latency LSP reuse, profile CLI and benchmark reporting |

Optimization work therefore starts now and continues through every remaining
phase. Compatibility shadows remain active throughout.
