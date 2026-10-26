# Whole-program MIR inlining benchmark

**Run:** 2026-10-26, Linux x86-64. Both native programs were compiled with
Cranelift `-O3` and linked with `cc`; seven fresh-process timing samples were
taken. The fixture calls a one-expression `plus_one(Int) -> Int` function ten
million times. Output matched exactly: `50000005000000`.

| Build | Median | Minimum |
|---|---:|---:|
| `-O3` | 13.977 ms | 13.709 ms |
| `-O3 --lto` | 7.677 ms | 7.551 ms |

**Median speedup: 1.82×.** The current `--lto` is deliberately a safe first
slice: all L++ modules are already merged into one MIR program, and the pass
inlines only direct one-block callees with at most one pure scalar expression.
It never duplicates ownership effects, calls, stores, or managed values. It is
not object-file LTO and does not optimize the runtime or foreign libraries.
`benchmarks/lto/run.py` reproduces the test and verifies output equality.
