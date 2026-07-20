# Cross-language comparison

This folder compares equivalent canonical workloads across:

```text
L++ AOT
C
C++
Rust
Go
Zig
```

Run:

```sh
python3 benchmarks/comparison/run.py
```

The runner detects installed toolchains. Missing languages are marked `SKIP`, never treated as a performance result.

## Workloads

1. `fib35` — recursive Fibonacci
2. `loop10m` — integer accumulation loop
3. `calls1m` — function-call chain inside a loop

The runner records compiler version, build latency, runtime latency, stdout correctness, and process status. It is intentionally a single-run development tool; use repeated pinned-hardware runs for publishable performance claims.

## v0.1.3 documentation status

For the current supported subset and explicit feature boundaries, see
[`documentation/CURRENT_CAPABILITIES.md`](../../documentation/CURRENT_CAPABILITIES.md).

Do not use historical benchmark numbers or roadmap text as current guarantees.
