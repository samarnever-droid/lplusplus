# L++ (L Plus Plus)

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

## Getting Started

Check out [Doc.md](Doc.md) for a comprehensive guide on the syntax and semantics of L++.

## Architecture

L++ is currently implemented as a Rust-based compiler frontend featuring:
- A custom lexer managing Python-style significant whitespace.
- A recursive descent parser.
- A semantic resolver mapping a persistent Arena-based Scope and Binding tree.
- A multi-pass Type Table for resolving self-referential structs and local types.
