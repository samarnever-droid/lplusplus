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
