# Benchmark notes

This directory contains historical benchmark reports and reproducible workload
scripts. Numbers in older reports are not current compiler guarantees.

## Current backend comparison

The current implementation has three relevant configurations:

| Configuration | Role |
|---|---|
| Cranelift | Default, lowest compile latency, complete current backend |
| LLVM | Optional, slower compilation, stronger optimization/vectorization |
| Explicit vectors | `VectorI64x2` API supported by both backends |

The long workload is `tests/vector_stress.lpp`. Re-run it instead of copying
old timing claims:

```sh
# Cranelift default
lpp tests/vector_stress.lpp --emit-object

# LLVM optimized object
LPP_LLVM_MARCH=native lpp tests/vector_stress.lpp \
    --backend llvm --emit-object
```

The repository does not claim a universal speedup. Compiler and runtime speed
depend on workload, CPU features, linker, and whether LLVM is available.

## Ownership/arena benchmarks

Recursive structures and Arena lifetime tests live in `tests/recursive_structures.lpp`
and `tests/arena_return.lpp`. They are correctness workloads first. Arena uses
correctness-first region lifetime handling; no bump allocator throughput claim
is made yet.

## Validation rather than marketing numbers

Use these gates for current results:

```sh
cargo test --release -j1
sh tests/run_aot_parity.sh
sh scripts/check_safety_mission.sh
```

Package workloads are validated separately by `packages/lppsqlite/run-tests.sh`
and `packages/compresslpp/run-tests.sh`.
