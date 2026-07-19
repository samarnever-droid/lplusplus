# L++ (L Plus Plus)

Status on July 19, 2026: active experimental compiler prototype. The core frontend, ownership-aware MIR, ARC runtime, Cranelift AOT backend, C compatibility backend, and cross-platform build helpers are usable for the tested subset. Advanced language semantics and full C/AOT ownership parity remain incomplete.

L++ is a modern, experimental programming language that aims to combine the best aspects of four major paradigms:

1. **Easy like Python** — high readability, low cognitive overhead, no manual memory bookkeeping, and significant whitespace.
2. **Safe like Rust** — no segfaults, no use-after-free, memory-safe by design.
3. **Compile speed like Go** — blazing fast builds for quick iteration, enabled by explicit public type signatures.
4. **Latency/Speed like C++** — zero-cost abstractions where possible, predictable performance without GC pauses.

## Core Design: The Hybrid Memory Model

L++ employs a **Value-by-Default with Auto-ARC** memory model. 
Data is stack-allocated by default. The compiler uses a sophisticated semantic pass (Escape Analysis) to automatically promote data to the heap (via Automatic Reference Counting or Arenas) only when necessary:
- **Rule 1:** Returned by Reference
- **Rule 2:** Closure Capture
- **Rule 3:** Unbounded/Ambiguous Lifetime Container
- **Rule 4:** Concurrency Boundary
- **Rule 5:** Self-Referential Data (Arenas)
- **Rule 6:** Algorithmic Aliasing

Developers write clean, Python-like code, and the compiler handles the lifetime optimizations under the hood.

## Implementation Status

Currently implemented with meaningful coverage:

- lexer, parser, semantic analysis, type checking
- escape analysis and MIR lowering
- C code generation
- Cranelift-based native object emission for the supported subset
- cross-platform package/install/build workflow with host C compiler fallback

Currently experimental or partial:

- closure lowering and captured environments
- list behavior parity across backends
- full ARC/runtime ownership behavior
- package ecosystem and dependency resolution robustness

Current builtin runtime coverage includes console/file I/O, ARC-managed `List[Int]` and `List[Custom]`, JSON helpers, threads, and TCP networking primitives (`net_connect`, `net_listen`, `net_accept`, `net_send`, `net_recv`, `net_close`).

## Benchmark Snapshot

**Measured July 19, 2026** on the Arena Linux sandbox (`Linux 6.1.158+`, Debian `cc` 14.2.0). These are **single-run development measurements**, not cross-machine comparisons or statistical performance claims. The compiler was pre-built in release mode; compile timing below is the compiler pipeline and excludes rebuilding the compiler itself.

| Workload | L++ compiler total | Cranelift AOT phase | C link step | Native runtime | Result | Executable size |
|---|---:|---:|---:|---:|---|---:|
| `fib(35)` | 0.974 ms | 0.828 ms | 339.118 ms | 71.029 ms | `9227465` | 24,120 B |
| loop (10M) | 0.866 ms | 0.734 ms | 205.309 ms | 8.786 ms | `49999995000000` | 24,120 B |
| call chain (1M) | 1.038 ms | 0.893 ms | 200.665 ms | 5.035 ms | `500000500000` | 24,176 B |

Notes:

- Native-link time is dominated by the host C toolchain and runtime link, not the L++ frontend or Cranelift object emission.
- The AOT benchmark path links Cranelift PIC objects with `lpp_runtime.c` and verifies each expected program result.
- Use the parity suite (`sh tests/run_aot_parity.sh`) to check supported C/AOT behavior rather than comparing these measurements to the older Windows/MSVC benchmark record.

## King 20 Standards

The **King 20** benchmark family has two tracks.

| Track | Purpose | Change policy |
|---|---|---|
| **King 20 Stable v1** | Historical comparison baseline | Frozen forever; create `v2` rather than editing it |
| **King 20 Experimental** | New ownership features, regressions, and optimization work | May evolve with the language |

Each suite combines three runtime workloads with ownership, ARC, closure, list, and branch regressions. A case must match expected stdout and exit with status zero before timing is recorded.

```bash
python3 benchmarks/king20/run.py --suite stable
python3 benchmarks/king20/run.py --suite experimental
```

The latest checked-in Stable v1 sandbox run is recorded in [`benchmarks/king20/stable/v1/latest.md`](benchmarks/king20/stable/v1/latest.md): **20 / 20 passed** on a 2-core Intel Xeon Linux sandbox. The runner captures platform, CPU model, logical CPU count, memory, Python, Rust, and host C compiler information in suite-specific `latest.json` files.

The direct-ELF King 20 subset currently passes **17 / 20** workloads with no host final linker. Its latest report is [`benchmarks/king20/direct_elf_latest.md`](benchmarks/king20/direct_elf_latest.md). In the current sandbox, direct linking takes about **1.5–2.1 ms** per supported workload, compared with roughly **200 ms** for the host-link path.

> Standalone AOT executables normally require a host linker because Cranelift emits native object files. Phase 2 now includes an experimental Linux x86-64 `lpp-link` ELF MVP that merges internal `.text`, `.rodata`, GOT runtime imports, and a freestanding syscall ARC runtime without a host final-link step. It is tested with runtime-free programs, scalar workloads, ARC structs, aliases, destructors, closures, and **17 / 20** King 20 workloads. The remaining direct-link gap is `List[Int]` / `List[Custom]`; networking, files, threads, and JSON still use the host-link fallback.

## Scalability phase analysis

The scalability suite generates deterministic programs at **10,000**, **50,000**, and **100,000** LOC and records the individual compiler phases:

```text
I/O → lexing → parsing → semantic analysis → type checking → escape analysis → MIR → Cranelift AOT → host linking
```

```bash
python3 benchmarks/scalability/run.py
```

The current 100k LOC sandbox run completed the compiler pipeline in **783.497 ms**. Escape analysis (**691.433 ms**) and Cranelift AOT (**634.006 ms**) are currently the dominant scalable passes; host linking stayed near **203 ms** and did not scale with source size. See [`benchmarks/scalability/latest.md`](benchmarks/scalability/latest.md) for the full phase table.

## Cross-language comparison

Equivalent `fib(35)` and loop workloads can be compared against C, C++, Rust, Go, and Zig:

```bash
python3 benchmarks/comparison/run.py
```

Missing toolchains are recorded as `SKIP`; they are never reported as benchmark values. See [`benchmarks/comparison/README.md`](benchmarks/comparison/README.md) for methodology.

## GitHub language recognition

L++ source uses the `.lpp` extension and the maintained VS Code TextMate scope `source.lpp`. Generated benchmark reports are marked with `linguist-generated=true` in `.gitattributes`, so they do not distort repository language statistics.

Global GitHub language-bar recognition requires an upstream GitHub Linguist contribution; it cannot be forced by repository metadata alone. The maintained submission package is in [`linguist/UPSTREAM_LINGUIST_PR.md`](linguist/UPSTREAM_LINGUIST_PR.md), with a representative sample at [`linguist/samples/lpp/ownership_and_closures.lpp`](linguist/samples/lpp/ownership_and_closures.lpp).

## Getting Started

Check out [Doc.md](Doc.md) for a comprehensive guide on the syntax and semantics of L++.

Installer helpers:

- Windows: `.\install.ps1`
- Unix-like shells: `./install.sh`
- Repo-local wrappers: `lpp.bat` on Windows, `./lpp` on Unix-like shells

## Architecture

L++ is currently implemented as a Rust-based compiler frontend featuring:
- A custom lexer managing Python-style significant whitespace.
- A recursive descent parser.
- A semantic resolver mapping a persistent Arena-based Scope and Binding tree.
- A multi-pass Type Table for resolving self-referential structs and local types.
