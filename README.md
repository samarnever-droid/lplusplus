# L++ (L Plus Plus)

Status on July 17, 2026: active experimental compiler prototype. The core frontend, analysis passes, C backend, and cross-platform CLI/build helpers are usable for development, but advanced runtime semantics and some AOT backend features are still incomplete.

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

Current builtin runtime coverage includes console/file I/O, lists, JSON helpers, threads, and TCP networking primitives (`net_connect`, `net_listen`, `net_accept`, `net_send`, `net_recv`, `net_close`).

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
