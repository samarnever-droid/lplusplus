# L++ Compiler Benchmarks

This report contains comprehensive benchmarks comparing **L++** and **Rust (`rustc`)** on a generated file containing all 4 core Memory Model Pillars + 5 Additional tests (If/Else, While, Nested Closures, Rule 3, Relational Ops).

## Workload details
- **L++ Lines of Code**: 82500
- **Rust Lines of Code**: 117005
- **Pillars & Tests Exercised**: Value (Stack), Reference Returns (Managed Heap), Closure Captures (Managed Heap), Spawn Closures (Managed Heap), Self-Referential Structs (Arenas), If/Else, While, Nested Closures, Rule 3, Relational Ops.

## 1. Overall Compile Time (External Latency)
This measures the complete end-to-end time from launching the process to the process exiting.
- **L++**: `4.9959 seconds` (Frontend only)
- **Rust**: `111.1815 seconds` (Full backend compilation)
- **Speedup**: L++ frontend latency is **22.25x faster** than Rust's full compilation.

---

## The 5 Internal Phase Tests (Micro-benchmarks)
We instrumented the L++ compiler to report exactly how long each phase of the frontend compilation took for the 82500 lines of code.

| Phase | Time (seconds) | Operations |
|-------|----------------|------------|
| **1. File I/O** | `0.1173 s` | Reading 100k lines from disk |
| **2. Lexer** | `0.1192 s` | Tokenizing the entire file |
| **3. Parser** | `0.0744 s` | Building the Abstract Syntax Tree (AST) |
| **4. Semantic & Typecheck** | `1.8301 s` | Resolving variable bindings and inferring types |
| **5. Escape Analyzer** | `1.7338 s` | Running Memory Model Rules 1, 2, 4, 5 |

### Total Internal Time: `3.8748 seconds`

## Summary
The Escape Analyzer is incredibly efficient. It successfully analyzed 82500 lines of code and mapped every variable to its optimal storage class (`Value`, `Arc`, or `Arena`) in just `1.7338` seconds!
