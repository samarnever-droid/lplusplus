# Current capabilities

**Last reviewed: 2026-10-20.** This file is a compact index; the detailed
historical snapshot is in [STATUS-2026-07-30.md](STATUS-2026-07-30.md), while
current gate results and open work are tracked in [ROADMAP.md](../ROADMAP.md).

## Compiler

| Capability | Status |
|---|---|
| Python-readable indentation syntax | Available |
| Functions, defaults, loops, break/continue | Available |
| Structs, enums, match, `?` | Available |
| Generics and turbofish | Available and monomorphized |
| Traits with static/dynamic dispatch | Available |
| FFI / `extern "C"` | Cranelift supports direct scalar calls with explicit `Str` borrow/copy rules and host-library linking; LLVM/WASM reject imported calls for now. See [`EXTERNAL_C_ABI.md`](EXTERNAL_C_ABI.md). |
| Closures and thread spawn | Available |
| Fixed structural tuples | Experimental first tier: arity 2–4, managed elements, destructuring; Cranelift supports tuple list elements, LLVM/WASM parity incomplete |
| Typed variadics | Experimental first tier: final owned rest `List[T]`; extern C varargs rejected |
| Borrowed slices | Experimental first tier: zero-copy `StrSlice` / `Slice[T]`, conservative non-escape rules |
| Async/await | Experimental first tier: task state records and single-thread run-to-completion executor |
| Lists, maps, strings, buffers, files, networking | Available in the supported runtime paths |
| LSP server and diagnostics | Available |
| Package tooling | Rust PM is production default; pure-L++ PM is experimental opt-in (`LPP_SELF_HOSTED_PM=1`) |
| `c2lpp` package | Phase 1 C-header/native-package generation complete; Phase 2 audited scalar translation started |

## Ownership

- MIR is the single source of escape/storage truth.
- `Frame < Owned < Shared` is the escape lattice.
- Non-escaping ordinary structs use stack payloads.
- Escaping values use ARC.
- Self-referential structs are ordinary ARC allocations whose types the
  ownership plan classifies `Shared` (collectible). Cranelift's native runtime
  reclaims unreachable cycles by trial-deletion; acyclic values free promptly
  by refcount, and no edge is demoted to non-owning. This reclamation claim is
  backend-specific: WASM recycles freed acyclic blocks but does not yet collect
  unreachable cycles; LLVM managed-value/collector parity is incomplete. (The
  legacy arena API is not the cycle mechanism. See
  `documentation/CYCLE_BREAKING.md` and `ROADMAP.md`.)
- Closures use ARC environments; non-escaping closure capsules can be stack-resident.
- Tuples and tasks are ARC aggregates with runtime child/environment teardown.
- Slice view records are stack-resident and borrowed; the MIR validator prevents
  first-tier escapes and the runtime validates bounds/generation at each access.

## Backends and linkers

| Combination | Status |
|---|---|
| Cranelift + host linker | Production/default; managed aggregate drop glue and native cycle collector |
| Cranelift + direct ELF/PE/Mach-O linker | Production for the verified targets |
| WASM | Experimental; ARC size-class free-list reuse passes bounded acyclic churn; cycle collection missing |
| LLVM + host linker | Optional; managed structs/lists/tuples and cycle-collector parity incomplete |
| LLVM + direct linker | Optional, tested for the supported subset |
| Windows LLVM execution | Needs Windows CI validation |

Select the optional backend explicitly:

```sh
lpp app.lpp --backend llvm --linker direct
```

There is no Turbo mode in the current repository. Cranelift remains the fast
compile-time default; LLVM is the optional optimization backend.

## Explicit vectors

The common API supports `VectorI64x2` construction, splat, add, subtract,
multiply, XOR, constant right shift, lane extraction, and horizontal sum. The
long checksum workload also has a four-lane LLVM IR path and an AVX2 runtime
path for Cranelift.

## Verified gates (2026-10-20)

- `bash scripts/fast_gate.sh`: **PASS** — 531 workspace tests; driver build;
  30 bindgen source modules across 4 entry graphs; focused bindgen
  ASan/LSan/UBSan regression; 58 AOT parity cases. Latest run: 105 seconds
  (the earlier dependency-cold run took 209 seconds).
- `bash scripts/corpus_correctness.sh run`: **191/191 scored** (178 run cases
  and 13 expected compiler rejections; 18 skipped by corpus policy).
- `cargo test --locked -p lpp-codegen-wasm --test wasm_gate
  arc_heap_reuses_freed_blocks_under_repeated_managed_churn -- --nocapture`:
  **PASS**; 100 and 100,000 same-shape churn iterations both use 1 WASM page.
- `bash scripts/check_bindgen_lsan.sh`: **PASS** with leak detection enabled;
  normal destroy and stale-handle panic both run under ASan/LSan/UBSan.
- Historical backend/differential/safety evidence is recorded in
  [STATUS-2026-07-30.md](STATUS-2026-07-30.md); it is not a replacement for
  the current gates above.
- `packages/lpp-bindgen/run-tests.sh`: **not fully passing** yet. Its C-memory
  sanitizer gate passes, then the optional system-SQLite smoke compile fails
  because rewrite MIR does not resolve imported `extern` functions. See
  `ROADMAP.md`; do not report this end-to-end suite as green.

## Not yet claimed

- Project-wide “100% feature freeze”; several language additions remain experimental.
- WASM cycle collection or bounded memory for cyclic WASM workloads.
- LLVM managed structs/lists/tuples or native cycle-collector parity.
- Successful rewrite-compiler calls to imported `extern` functions (the bindgen
  system-SQLite smoke is still a known failure).
- Async I/O readiness, backpressure, multi-thread work stealing, or C100K claims.
- General lifetime-parameterized slices, mutable slices, or slice thread send.
- Unsafe native C variadic ABI or tuples outside arity 2–4.
- General source-level automatic vectorization of arbitrary loops.
- Windows LLVM runtime execution.
- LLVM LTO/PGO.
- Arena bump/chunk allocation performance optimization.
