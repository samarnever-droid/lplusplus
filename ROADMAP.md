# L++ Roadmap — how complete is L++, and what "done" looks like

_Last audited: 2026-10-20, against the working tree (21 crates, 100,361 Rust
LOC, 531 workspace tests, 191/191 scored corpus cases, 58/58 AOT parity).
These are observed counts from the commands below; percentages are engineering
estimates, not marketing. This is the honest map of what exists, what remains,
and how to catch regressions quickly in a ~100k-LOC compiler._

## 1. Completeness scorecard

| Area | ~Done | Evidence / gap |
|---|---|---|
| Core language | **85%** | Structs, enums+match, generics+turbofish, traits (static+dynamic), closures, `Result`+`?`, modules, FFI, tuples, slices, variadics, defaults. Missing: iterators/generators, macros, operator overloading. Rewrite MIR still cannot resolve/call imported `extern` declarations; the bindgen system-SQLite smoke exposes this gap. |
| Compiler & backends | **80%** | Cranelift production; LLVM optional; WASM works with size-class ARC free-list reuse for acyclic churn; direct ELF/PE/Mach-O linker. Cycle collector + tuple drop glue cover Cranelift's currently supported managed shapes. WASM cycle collection is missing; LLVM managed structs/lists/tuples and collector parity remain unsupported/unverified. |
| Memory-safety proof | **~55%** | Safety Mission level **S2 of S4** (verified Linux x86-64 Cranelift subset). S3 needs per-platform CI evidence; S4 needs a spec, formal argument, fuzzing, independent audit. TSan has not been run. |
| Standard library | **60%** | 500+ builtin entries + 35 stdlib modules. Broad but thin vs Go/Python. |
| Tooling | **65%** | Keel PM (SemVer, lockfiles), LSP (8 methods), bench, direct linker, source-compile gate for bindgen, focused ASan/LSan/UBSan regression. No debugger story, no REPL, no `fmt` CLI, shallow LSP. |
| Async / concurrency | **40%** | Threads + atomics exist; async is single-thread run-to-completion, no I/O reactor. The concurrency contract is now documented; Send/Sync-style enforcement is not. |
| Platforms | **65%** | Linux gates are strong. CI executes authored corpus on Windows and macOS (arm64 + Intel); Windows sanitizer-equivalent coverage and aarch64 Linux runtime/linker proof remain. |
| Ecosystem & stability | **15%** | v0.1.0, ~24 in-repo packages, no stability guarantee yet. |
| **Overall: usable language today** | **~75%** | |
| **Overall: mature 1.0 platform** | **~45%** | The gap is mostly proof, platforms, and ecosystem — not syntax. |

## 2. What is missing, in build order

### Tier 1 — correctness holes (bugs, not features)

1. ~~Cranelift structural-tuple drop glue~~ — **DONE 2026-10-20**
   (`lpp_drop_t{n}`/`lpp_trace_t{n}`; see `documentation/CYCLE_BREAKING.md`).
2. **WASM cycle collection — PARTIAL (2026-10-20).** The WASM runtime now
   reuses freed ARC blocks in size-class bins, including list backing stores;
   `arc_heap_reuses_freed_blocks_under_repeated_managed_churn` measured **1
   page at both 100 and 100,000 iterations**. This is bounded acyclic churn,
   not cycle-collector parity: unreachable reference cycles still leak. Next,
   port the ownership-plan trace/mark/trial-deletion protocol and add cycle,
   live-root, acyclic-tail, and churn tests before claiming bounded cyclic
   workloads.
3. **LLVM managed-value and cycle-collector parity — NOT DONE.** The LLVM
   backend still lacks the Cranelift managed struct/list/tuple path and
   trial-deletion collector. First establish ARC drop/trace codegen for
   managed aggregates, then run the same oracle/native churn fixtures; do not
   claim parity before those pass.
4. **Concurrency safety enforcement — PARTIAL.** `documentation/CONCURRENCY.md`
   now records the actual contract and the gaps. Add a distinct concurrent
   sharing fact to the ownership plan, reject unsynchronized mutable-field
   access, choose a collector-exclusion protocol, and validate it with TSan
   stress tests. No data-race-freedom claim is currently warranted.
5. **Integer divide/mod panic parity — PARTIAL.** Guards are emitted for
   Cranelift, LLVM, and WASM; Cranelift and WASM runtime cases (division,
   modulo, and `INT_MIN / -1` wrapping) passed locally. The LLVM guard compiles,
   but this machine has no `clang`, so its conditional execution test did not
   run. Close with an LLVM-enabled CI/runtime run; the shared no-unwind/`Result`
   contract is in `documentation/PANIC_AND_ERRORS.md`.

### Tier 2 — platform proof (S2 → S3)

6. **Windows MSVC proof — PARTIAL.** CI now executes the authored corpus,
   native compile/link/run, and PE smoke cases on `windows-latest`; it is no
   longer compile-only. Still add sanitizer-equivalent stale-handle/heap
   diagnostics for this toolchain and pin their results.
7. **macOS / aarch64 Linux proof — PARTIAL.** CI runs the corpus on macOS 14
   arm64 and macOS 15 Intel. aarch64 Linux runtime execution is still missing
   (and would close the linker `__libc_start_main` TODO).

### Tier 3 — language completeness

8. **Real async**: I/O readiness reactor (epoll/kqueue/IOCP adapters),
   backpressure, general suspension points, optional multi-thread executor.
9. **General slice lifetimes**: mutable slices, escapable views with
   lifetime parameters, slices across threads.
10. **Tuple parity and tuple arity beyond 4 — PARTIAL.** Cranelift now
    supports managed tuples as list elements (drop glue + `Managed` element
    path; churn/alias/cycle tests). WASM/LLVM tuple-list support and arity
    beyond 2–4 remain.
11. **Iterator/generator protocol**; richer collection set (Set, deque).
12. **Auto-vectorization** of ordinary loops (LLVM first); wider SIMD
    types; LTO/PGO plumbing.
13. Macros/metaprogramming — *design decision required*, deliberately last:
    it multiplies the surface every other item must verify.

### Tier 4 — tooling & developer experience

14. **Debug info (DWARF/CodeView)** so gdb/lldb/WinDbg work on L++ binaries.
15. `lpp fmt` CLI + linter; LSP rename/references/code-actions; REPL on
    the MIR interpreter (the oracle makes this nearly free).
16. **In-repo fuzzing in CI** (seeded smoke: `scripts/fuzz_ownership.py
    --seed 1 --count 60`) and a one-command fast gate
    (`scripts/fast_gate.sh`). The fast gate now includes bindgen source
    compilation and a focused context-tombstone ASan/LSan/UBSan regression.

### Tier 5 — maturity (the long tail to 1.0)

17. Complete language specification (lifetimes, aliasing, FFI, unsafe
    boundary) — Safety Mission S4 prerequisite.
18. A **1.0 stability policy**: what may break, deprecation process, editions.
19. Independent security audit; formal/machine-checked soundness argument
    for the ownership core.
20. Ecosystem: hosted registry growth, docs hosting, more packages,
    production users. (Measured in years, not sprints — start early,
    finish last.)

**Sequencing rule:** features are *not* the bottleneck. Close Tier 1–2
before adding Tier-3 surface — every new syntax feature multiplies the
unverified area. A smaller language at S3 beats a bigger one at S2.

## 3. How we catch bugs fast in ~100k LOC

Layered gates — each layer faster and dumber than the next, so most bugs
die in seconds:

| Layer | Command | Observed result | Catches |
|---|---|---|---|
| Format + lint | `cargo fmt --check`; `cargo clippy --locked --workspace --all-targets -j2` | seconds | Formatting, suspicious code, compile-time exhaustiveness. A new MIR/type variant must be handled throughout the compiler before it can build. |
| Unit/integration tests | `cargo test --locked -j2 --workspace` | **531 passed** | Layout, ABI, semver, leak classification, collector properties, diagnostics, backend contracts. |
| **The ARC oracle** ⭐ | `execute_mir_arc` test suites | Included in workspace tests | Exact retain/release balance and typed leak reports. **House rule: model semantic behavior in the oracle before backend codegen**; then compare oracle and native results. |
| Bindgen source compile | `python3 scripts/check_bindgen_sources.py` | **30 modules / 4 entry graphs** | Package-local L++ type/MIR/ownership regressions that Cargo does not compile. Roots cover `main.lpp` imports plus unreachable source modules. |
| Focused memory sanitizer | `bash scripts/check_bindgen_lsan.sh` | **PASS** | Compiles clean and stale-context CMemory cases with ASan/LSan/UBSan. Confirms the tombstone still traps after destroy and is reclaimed at shutdown; no sanitizer suppression. |
| Native corpus | `bash scripts/corpus_correctness.sh run` | **191/191 scored** (178 run + 13 compiler rejects; 18 skipped) | Exact run/exit/assert checks plus correct negative-test rejection. |
| AOT parity | `sh tests/run_aot_parity.sh` | **58 passed, 0 failed** | Oracle/native stdout parity through host C and direct link paths; includes the short-circuit side-effect regression. |
| Sanitizers | CI `sanitizers-and-fuzz` job; bindgen focus above | per PR | UAF, double-free, leaks, and UB that output-only tests miss. |
| Differential fuzzing | `scripts/fuzz_ownership.py --seed N --count M` | CI smoke: seed 1, count 60 | Seeded ownership graphs (cycles, tuples, closures, churn), oracle/native checks and bounded-memory properties. Persist each failing seed as a regression. |
| CI platform matrix | `.github/workflows/ci.yml` | per push | Cross-platform/runtime mismatches; Linux, Windows, macOS Intel/arm64, and all-arch checks. |

**Current red test, reported honestly:** `LPP="$PWD/target/debug/lpp" sh
packages/lpp-bindgen/run-tests.sh` now gets past the previous 64-byte
LeakSanitizer tombstone failure; its explicit C-memory ASan/LSan/UBSan gate
passes. The full runner still exits 1 later in the optional system-SQLite
native smoke because the rewrite MIR does not yet resolve/call imported
`extern` functions (`Unsupported(UnresolvedName)`). This is a separate FFI
compiler gap. Do not mark the bindgen end-to-end suite green, skip the smoke,
or weaken the stale-context assertion.

Two habits matter more than any tool:

1. **Every bug becomes a failing regression before it is fixed.** The
   scored corpus currently has 191 cases; sanitizer-only and backend-specific
   cases remain explicit tests even when not part of that corpus.
2. **Claim discipline:** keep passing evidence separate from known failures.
   For example, WASM now reuses freed blocks for tested acyclic churn, but
   that is not cycle collection; and bindgen's focused LSan gate passes while
   its full system-SQLite runner still fails on imported-extern resolution.
3. **Keep feedback loops cheap and layered:** run the changed crate/test
   first, then `scripts/fast_gate.sh` before every commit, then the authored
   corpus/platform/sanitizer/fuzz matrix before merge or release. On a failure,
   preserve the seed, minimized source, backend, and exact command in a new
   regression; do not hide it with a skip or relax the assertion.

`bash scripts/fast_gate.sh` is the one-command local gate: fmt, workspace
Clippy, all 531 workspace tests, driver build, bindgen source compilation,
the focused bindgen ASan/LSan/UBSan regression, and 58-case AOT parity. The
latest run passed in 105 seconds; the earlier dependency-cold run took
209 seconds. The larger corpus, cross-platform jobs, full ownership
sanitizer set, and seeded fuzz batch run in CI.
