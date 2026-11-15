# L++ Roadmap — how complete is L++, and what "done" looks like

_Updated for **v0.3.0** (`v0.3-dev`). These figures and capabilities reflect
observed implementation and test suites in this tree; open items remain tracked
transparently below._

## 1. Completeness scorecard

| Area | ~Done | Evidence / gap |
|---|---|---|
| Core language | **92%** | Structs, enums+exhaustive match, generics+turbofish, traits (static+dynamic), closures, `Result`+`?`, modules, FFI, tuples up to 16, f-strings, ranges, slices, variadics, defaults, principled `unsafe` (`Ptr[T]`, `RawPtr`, casts, MMIO/port/control-reg intrinsics, `--deny-unsafe`). Missing: iterators/generators, macros, operator overloading. |
| Compiler & backends | **92%** | Cranelift (default native `x86_64`/`aarch64` + `*-unknown-none` bare-metal kernels); LLVM (`--backend llvm -O0`–`-O3`) now at full MIR/ARC/kernel/SIMD parity; direct ELF/PE/Mach-O + static kernel ELF linker; stack-placed `Frame` POD structs/tuples & destructor-free POD `Owned` ARC. WASM works with size-class free-list reuse for acyclic churn, while WASM cycle collection remains open. |
| Memory-safety proof | **~65%** | Safety Mission level **S2 → S3**. Ownership proofs (`Frame`/`Owned`/`Shared`), Bacon-Rajan cycle collector across Cranelift & LLVM, bindgen ASan/LSan/UBSan checks, and 22,000+ LOC `v03_*` production suites across all workspace crates. Still open: TSan stress & concurrent `Send`/`Sync` enforcement, S4 formal mechanization. |
| Standard library | **75%** | 500+ builtin entries, `Set[T]` & `Deque[T]`, strings, lists, maps, buffers, files, networking, JSON, bare-metal kernel/Multiboot2 runtime. Still thin relative to mature Go/Python ecosystems. |
| Tooling | **85%** | Keel PM (SemVer, lockfiles, `lpp/keel migrate`), DWARF `-g` debug info (`lpp debug` CLI + backtraces), `lpp fmt`, `lpp repl`, `lpp test`, `lpp-lsp`, `lpp-link`, `lpp-bench`. LSP semantic rename/references still minimal. |
| Async / concurrency | **45%** | Threads + atomics exist; async is single-thread run-to-completion, no I/O reactor yet. `Send`/`Sync`-style static enforcement and TSan validation remain open. |
| Platforms | **75%** | Linux, Windows MSVC/PE, and macOS (arm64 + Intel) native CI, WASI WebAssembly CI, plus `x86_64-unknown-none` & `aarch64-unknown-none` bare-metal targets. Complete aarch64 Linux hosted execution and Windows Sanitizer coverage remain. |
| Ecosystem & stability | **25%** | **v0.3.0**, modern `Keel.toml` across all in-repo packages, git registry (`llppregistry`), no 1.0 stability guarantee yet. |
| **Overall: usable language today** | **~88%** | |
| **Overall: mature 1.0 platform** | **~62%** | The remaining gap is WASM cycle collection, async I/O reactor, `Send`/`Sync` proof, and ecosystem scale. |

## 2. Status since v0.2.0 & remaining build order

### Tier 1 — correctness & parity

1. ~~Cranelift structural-tuple drop glue~~ — **DONE**
   (`lpp_drop_t{n}`/`lpp_trace_t{n}`; see `documentation/CYCLE_BREAKING.md`).
2. **WASM cycle collection — PARTIAL.** The WASM runtime reuses freed ARC
   blocks in size-class bins (`1` page under acyclic churn), but unreachable
   reference cycles still leak in WASM. Next, port the ownership-plan
   trace/mark/trial-deletion protocol to `lpp-codegen-wasm`.
3. ~~LLVM managed-value, kernel, SIMD, and -O3 parity~~ — **DONE in v0.3.0.**
   `lpp-codegen-llvm` lowers full MIR, managed structs/enums/lists/tuples,
   cycle-collector ARC destructors, DWARF `-g`, SIMD intrinsics, kernel primitives,
   and `--backend llvm -O0`..`-O3`.
4. **Concurrency safety enforcement — PARTIAL.** `documentation/CONCURRENCY.md`
   records the contract and open gaps. Add a concurrent-sharing fact to the
   ownership plan, enforce `Send`/`Sync` rules, add a collector-exclusion
   protocol, and validate under TSan.
5. ~~Stack-placed `Frame` POD & zero-overhead POD `Owned` codegen~~ — **DONE in v0.3.0.**
   `lpp-codegen-api::ownership_lowering` identifies non-escaping `Frame` POD
   structs/tuples for stack allocation (`StackSlot` / `alloca`) with zero ARC
   traffic, and passes null destructors for escaping POD `Owned` payloads across
   Cranelift and LLVM.

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
10. ~~Tuple parity and tuple arity beyond 4~~ — **DONE in v0.3.0**
    for Cranelift and LLVM (tuples up to arity 16 with managed elements).
11. **Iterator/generator protocol** — `Set[T]` and `Deque[T]` collections are **DONE in v0.3.0**; general lazy iterator/generator traits remain open.
12. **General auto-vectorization** of ordinary loops (`-O3` LLVM unlocks loop vectorization when eligible) and
    source-level wider SIMD types remain open. `vec_i64_checksum` dispatches to
    a 4-lane 256-bit AVX2 kernel with scalar fallback; this is workload-specific,
    not a general source-level `VectorI64x4` type (`benchmarks/simd/results.md`).
13. Macros/metaprogramming — *design decision required*, deliberately last:
    it multiplies the surface every other item must verify.

### Tier 4 — tooling & developer experience

14. ~~Debug info (DWARF `-g`) & runtime backtraces~~ — **DONE in v0.3.0**
    for Cranelift & LLVM, along with `lpp debug` CLI support. CodeView/WinDbg PDB emission remains open on Windows.
15. ~~`lpp fmt`, `lpp repl`, `lpp test`, `lpp/keel migrate`~~ — **DONE in v0.3.0.**
    LSP rename/references/code-actions remain open.
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
| Unit/integration tests | `cargo test --locked -j2 --workspace` | **872 passed** | Layout, ABI, semver, leak classification, collector properties, diagnostics, backend contracts, `v03_*` production suites. |
| **The ARC oracle** ⭐ | `execute_mir_arc` test suites | Included in workspace tests | Exact retain/release balance and typed leak reports. **House rule: model semantic behavior in the oracle before backend codegen**; then compare oracle and native results. |
| Bindgen source compile | `python3 scripts/check_bindgen_sources.py` | **37 modules / 4 entry graphs** | Package-local L++ type/MIR/ownership regressions that Cargo does not compile. Roots cover `main.lpp` imports plus unreachable source modules. |
| Focused memory sanitizer | `bash scripts/check_bindgen_lsan.sh` | **PASS** | Compiles clean and stale-context CMemory cases with ASan/LSan/UBSan. Confirms the tombstone still traps after destroy and is reclaimed at shutdown; no sanitizer suppression. |
| Native corpus | `bash scripts/corpus_correctness.sh run` | **193/193 scored** (180 run + 13 compiler rejects; 18 skipped) | Exact run/exit/assert checks plus correct negative-test rejection. |
| AOT parity | `sh tests/run_aot_parity.sh` | **58 passed, 0 failed** | Oracle/native stdout parity through host C and direct link paths; includes the short-circuit side-effect regression. |
| Sanitizers | CI `sanitizers-and-fuzz` job; bindgen focus above | per PR | UAF, double-free, leaks, and UB that output-only tests miss. |
| Differential fuzzing | `scripts/fuzz_ownership.py --seed N --count M` | CI smoke: seed 1, count 60 | Seeded ownership graphs (cycles, tuples, closures, churn), oracle/native checks and bounded-memory properties. Persist each failing seed as a regression. |
| CI platform matrix | `.github/workflows/ci.yml` | per push | Cross-platform/runtime mismatches; Linux, Windows, macOS Intel/arm64, and all-arch checks. |

**Previously reported red test — revalidated green (2026-10-26):**
`LPP=/home/user/lplusplus/target/debug/lpp sh
packages/lpp-bindgen/run-tests.sh` passes end to end in 116 seconds on this
host, including the explicit C-memory ASan/LSan/UBSan gate, system SQLite
header/native smoke (270 declarations), and system zlib smoke (73 declarations).
This confirms the documented scalar/`Str` imported-`extern` path through rewrite
MIR, linking, and execution. The old `Unsupported(UnresolvedName)` note was
stale; do not treat it as an active FFI failure. This does not expand the ABI to
raw pointers, aggregates, callbacks, or C varargs.

Two habits matter more than any tool:

1. **Every bug becomes a failing regression before it is fixed.** The
   scored corpus currently has 191 cases; sanitizer-only and backend-specific
   cases remain explicit tests even when not part of that corpus.
2. **Claim discipline:** keep passing evidence separate from known failures.
   For example, WASM now reuses freed blocks for tested acyclic churn, but
   that is not cycle collection; and native FFI passes the documented scalar/
   `Str` system-library smokes, but that is not general pointer or aggregate ABI support.
3. **Keep feedback loops cheap and layered:** run the changed crate/test
   first, then `scripts/fast_gate.sh` before every commit, then the authored
   corpus/platform/sanitizer/fuzz matrix before merge or release. On a failure,
   preserve the seed, minimized source, backend, and exact command in a new
   regression; do not hide it with a skip or relax the assertion.

`bash scripts/fast_gate.sh` is the one-command local gate: fmt, workspace
Clippy, all workspace tests (including the 22,500+ LOC `v03_*` production
suites), driver build, bindgen source compilation, the focused bindgen
ASan/LSan/UBSan regression, and 58-case AOT parity. The larger corpus,
cross-platform jobs, full ownership sanitizer set, and seeded fuzz batch run in
CI.
