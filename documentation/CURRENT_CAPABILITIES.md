# Current capabilities

**Last reviewed: 2026-10-26 (FFI, SIMD, MIR inlining, and PGO);** remaining gate counts below are the 2026-10-20 snapshot. This file is a compact index; the detailed
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
| FFI / `extern "C"` | Cranelift supports direct calls for the documented scalar/`Str` subset, including imported declarations across modules and host-library linking. The system SQLite and zlib binding smokes pass; LLVM/WASM still reject imported calls. No raw-pointer type is added. See [`EXTERNAL_C_ABI.md`](EXTERNAL_C_ABI.md). |
| Native optimization | `-O0` selects Cranelift `none`; `-O1`/`-O2`/`-O3` use Cranelift `speed`; `-Os`/`-Oz` use `speed_and_size`. `--lto` enables the bounded whole-program MIR inliner for tiny pure scalar callees; `--pgo-generate`/`--pgo-use` profile function entries and steer that same inliner. This is not object-file LTO or general profile-guided machine-code optimization. LLVM remains forced to `-O0`. See [`benchmarks/lto/results.md`](../benchmarks/lto/results.md) and [`benchmarks/pgo/results.md`](../benchmarks/pgo/results.md). |
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
`vec_i64_checksum` runtime intrinsic now auto-dispatches to a four-lane
256-bit AVX2 kernel on capable x86-64 CPUs and falls back to a scalar reference
elsewhere. This is a workload-specific SIMD intrinsic, not general loop
auto-vectorization or a source-level `VectorI64x4` type; see
[`benchmarks/simd/results.md`](../benchmarks/simd/results.md).

## MIR inlining and PGO

`--lto` opts into the whole-program MIR inliner: it only substitutes direct,
one-block, tiny, pure scalar callees and avoids effects, calls, stores, and
managed values. It operates after L++ modules have been merged into MIR; it is
not object-file, runtime, or foreign-library LTO.

PGO generation is available for Cranelift builds targeting the compiler host:

```sh
lpp app.lpp -O3 --pgo-generate -o app-instrumented
LPP_PGO_PROFILE=app.pgo ./app-instrumented
lpp app.lpp -O3 --pgo-use app.pgo -o app-pgo
```

If `LPP_PGO_PROFILE` is unset, the runtime writes `lpp.pgo` in the process
working directory. The versioned profile stores MIR function-entry counts and
a fingerprint of the pre-optimization program; `--pgo-use` rejects malformed or
mismatched profiles. PGO selects hot callees for the same bounded MIR inliner;
it does not collect edge/value profiles or guide backend machine-code passes.
The runtime uses relaxed atomic counters in a dense table (IDs below 1,048,576)
and flushes it when source `main` returns normally. Profile generation adds
per-entry instrumentation overhead; see the benchmark for a measured example.

## Verified gates (2026-10-26)

- `cargo fmt --all -- --check` and `cargo test --locked --workspace`: **PASS**;
  includes PGO profile generation/use/mismatch checks and the imported-extern
  cross-module regression.
- Hosted and Linux freestanding C runtimes compile with the project flags; the
  corrected recursive runtime-symbol gate and Kernel32 import-table gate pass.
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
- `LPP=/home/user/lplusplus/target/debug/lpp sh packages/lpp-bindgen/tests/run.sh`:
  **PASS (2026-10-26, 116 s)**, including the system SQLite header/native smoke
  (270 declarations) and system zlib smoke (73 declarations). These exercise
  imported `extern` declarations through the rewrite compiler, native linker,
  and runtime. The earlier `Unsupported(UnresolvedName)` report is stale.

## Not yet claimed

- Project-wide “100% feature freeze”; several language additions remain experimental.
- WASM cycle collection or bounded memory for cyclic WASM workloads.
- LLVM managed structs/lists/tuples or native cycle-collector parity.
- Async I/O readiness, backpressure, multi-thread work stealing, or C100K claims.
- General lifetime-parameterized slices, mutable slices, or slice thread send.
- Unsafe native C variadic ABI or tuples outside arity 2–4.
- Windows LLVM runtime execution.
- Object-file/LLVM-style LTO or general profile-guided machine-code optimization.
- General source-level automatic vectorization of arbitrary loops or a source-level wider SIMD type.
- Arena bump/chunk allocation performance optimization.
