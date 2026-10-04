# Benchmarks and CI

## Main CI

The primary workflow is `.github/workflows/ci.yml`.

It validates:

- king20 smoke benchmarks
- scalability phase analysis
- ownership and parity tests
- Windows COFF / PE direct-link path
- macOS host-link path
- module and stdlib checks

## BPW benchmark workflow

The Benchmark Package Workflow is `.github/workflows/bpw.yml`.

It runs a multi-platform comparison across:

- L++
- Rust
- Go

Benchmarks include:

1. CPU-heavy: `fib(40)` plus prime counting
2. RAM-heavy: large list allocation/fill
3. File I/O: write/read workload
4. Multi-file import project
5. Generated large-project compile benchmark

Each benchmark records:

- average time
- min/max
- standard deviation
- binary size
- system information
- compiler versions
- output correctness

## Known benchmark interpretation

Very small millisecond timings are noisy. BPW runs multiple iterations and reports aggregate data, but benchmark numbers should still be read as directional rather than absolute.

The strongest observed L++ advantages are:

- very small direct-linked binaries
- fast compile times
- competitive native runtime on simple workloads


## BPW artifacts

The BPW workflow has separate platform jobs and a merge job:

```text
bpw-linux
bpw-windows
bpw-merge
```

The merge job downloads platform artifacts, combines JSON reports, prints a cross-platform table, and uploads a merged report artifact.

Typical artifact contents:

- source benchmark files
- per-platform `bpw_report.json`
- merged `bpw_merged.json`
- selected binaries where workflow configuration preserves them
