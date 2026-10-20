# L++ Roadmap — how complete is L++, and what "done" looks like

_Last audited: 2026-10-20, against the working tree (19 crates, ~96k LOC of
Rust, 524 workspace tests, 187/187 corpus, 53/53 AOT parity). This document
is the honest map: what exists, what is missing, in what order to build it,
and how bugs get caught fast along the way. Numbers are engineering
estimates, not marketing._

## 1. Completeness scorecard

| Area | ~Done | Evidence / gap |
|---|---|---|
| Core language | **85%** | Structs, enums+match, generics+turbofish, traits (static+dynamic), closures, `Result`+`?`, modules, FFI, tuples, slices, variadics, defaults. Missing: iterators/generators, macros, operator overloading. |
| Compiler & backends | **80%** | Cranelift production; LLVM optional; WASM works; direct ELF/PE/Mach-O linker. Cycle collector + tuple drop glue now cover Cranelift fully. WASM heap still reclaims nothing; LLVM collector parity unverified. |
| Memory-safety proof | **~55%** | Safety Mission level **S2 of S4** (verified Linux x86-64 Cranelift subset). S3 needs per-platform CI evidence; S4 needs a spec, formal argument, fuzzing, independent audit. |
| Standard library | **60%** | ~500 builtin entries + 35 stdlib modules. Broad but thin vs Go/Python. |
| Tooling | **65%** | Keel PM (SemVer, lockfiles), LSP (8 methods), bench, direct linker. No debugger story, no REPL, no `fmt` CLI, shallow LSP. |
| Async / concurrency | **40%** | Threads + atomics exist; async is single-thread run-to-completion, no I/O reactor, no written data-race model. |
| Platforms | **65%** | Linux proven. CI has windows-latest + macOS jobs; runtime execution evidence on Windows/MSVC is still thin. aarch64 hosted-glibc linker path has the tree's only TODO. |
| Ecosystem & stability | **15%** | v0.1.0, ~24 in-repo packages, no stability guarantee yet. |
| **Overall: usable language today** | **~75%** | |
| **Overall: mature 1.0 platform** | **~45%** | The gap is mostly proof, platforms, and ecosystem — not syntax. |

## 2. What is missing, in build order

### Tier 1 — correctness holes (bugs, not features)

1. ~~Cranelift structural-tuple drop glue~~ — **DONE 2026-10-20**
   (`lpp_drop_t{n}`/`lpp_trace_t{n}`; see `documentation/CYCLE_BREAKING.md`).
2. **WASM heap reclamation** — the wasm backend's bump heap never frees.
   Port the ARC + trial-deletion collector to the wasm runtime helpers so
   long-running wasm programs have bounded memory.
3. **LLVM parity for the cycle collector** — generate traces/marks in the
   LLVM backend and prove with the same churn corpus natively.
4. **Concurrency safety model on paper** — specify what may cross a
   `spawn` boundary (Send/Sync-style rules), then enforce in the ownership
   verifier. Until written, no data-race-freedom claim is possible.
5. **Panic/unwinding semantics spec** — what runs, what leaks-by-design,
   what is released, across FFI boundaries.

### Tier 2 — platform proof (S2 → S3)

6. Windows MSVC job that *executes* the full corpus + sanitizer-equivalent
   checks, not just compiles the min runtime.
7. macOS (arm64 + Intel) corpus execution in CI; aarch64 Linux execution
   (also closes the linker `__libc_start_main` TODO).

### Tier 3 — language completeness

8. **Real async**: I/O readiness reactor (epoll/kqueue/IOCP adapters),
   backpressure, general suspension points, optional multi-thread executor.
9. **General slice lifetimes**: mutable slices, escapable views with
   lifetime parameters, slices across threads.
10. **Tuples as list elements** and tuple arity beyond 4 (drop glue made
    this cheap: element class `Managed` + pre-scan acceptance).
11. **Iterator/generator protocol**; richer collection set (Set, deque).
12. **Auto-vectorization** of ordinary loops (LLVM first); wider SIMD
    types; LTO/PGO plumbing.
13. Macros/metaprogramming — *design decision required*, deliberately last:
    it multiplies the surface every other item must verify.

### Tier 4 — tooling & developer experience

14. **Debug info (DWARF/CodeView)** so gdb/lldb/WinDbg work on L++ binaries.
15. `lpp fmt` CLI + linter; LSP rename/references/code-actions; REPL on
    the MIR interpreter (the oracle makes this nearly free).
16. **In-repo fuzzing in CI** (started: `scripts/fuzz_ownership.py`) and a
    one-command fast gate (`scripts/fast_gate.sh`).

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

| Layer | Command | Speed | Catches |
|---|---|---|---|
| Format + lint | `cargo fmt --check`, `cargo clippy` | seconds | Style, logic smells. Exhaustive matches mean a new MIR/type variant *cannot compile* until every backend handles it. |
| Unit tests | `cargo test --workspace` (524) | ~1 min | Pure logic: layout, semver, leak classification, collector property tests. |
| **The ARC oracle** ⭐ | `execute_mir_arc` test suites | ~1 min | The MIR interpreter proves exact retain/release balance. **House rule: no feature lands in a backend until the oracle models it first** — backend bugs then show up as model-vs-native mismatches, which are mechanical to find. Leak failures are classified (`LeakReport`) so the error names the cause. |
| Corpus + AOT parity | `scripts/corpus_correctness.sh`, `tests/run_aot_parity.sh` | ~2–3 min | 187 programs compiled/linked/executed natively with exact-output checks; 13 negative programs must be rejected. |
| Sanitizers | ASan/LSan/UBSan link of emitted objects | minutes | UAF, double-free, leaks invisible to output tests. (`cc -fsanitize=address,undefined app.o c_shim.c`.) |
| Differential fuzzing | `scripts/fuzz_ownership.py --seed N --count M` | continuous | Seeded random ownership-graph programs (cycles, tuples, closures, churn), self-checking checksums, native + oracle agreement, RSS bound. Every failing seed becomes a permanent corpus test. |
| CI matrix | `.github/workflows/ci.yml` | per push | Platform lies — the Windows-min `&header->refcount` bug survived precisely because nothing compiled that file in CI. |

Two habits matter more than any tool:

1. **Every bug becomes a failing test before it is fixed** (the corpus
   already works this way — 187 files and counting).
2. **Claim discipline** (Safety Mission S-levels): never document beyond
   the evidence, so gaps stay visible instead of hiding — e.g. this file
   says "WASM reclaims nothing" until the day a test proves otherwise.

For day-to-day work: `scripts/fast_gate.sh` runs the sub-5-minute subset
(fmt, clippy, workspace tests, corpus smoke). Run it before every commit;
run the full battery + sanitizers + a fuzz batch before every release.
