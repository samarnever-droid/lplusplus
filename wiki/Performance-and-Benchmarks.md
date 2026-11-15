# Performance & Benchmarks

L++ tracks compiler scalability, AOT/optimization gains, and runtime microbenchmarks transparently using reproducible repository harnesses under [`benchmarks/`](../benchmarks/Benchmarks.md).

> Per [`docs/Safety_Mission.md`](../docs/Safety_Mission.md), unverified external comparisons against C, C++, Rust, Go, or Python are omitted. Only measured in-repo suites are reported here.

---

## 1. Benchmark Suites in This Repository

| Suite | Focus | Location |
|---|---|---|
| **King20 Stable** | 20 canonical native L++ workloads across Cranelift & host/direct linkers | [`benchmarks/king20/stable/v1/latest.md`](../benchmarks/king20/stable/v1/latest.md) |
| **MIR Inlining (`--lto`)** | Whole-program bounded pure-scalar MIR inlining across modules | [`benchmarks/lto/results.md`](../benchmarks/lto/results.md) |
| **Profile-Guided Inlining (`--pgo-*`)** | Cranelift host-target entry counter profiling (`LPP-PGO-1`) | [`benchmarks/pgo/results.md`](../benchmarks/pgo/results.md) |
| **SIMD Kernel (`vec_i64_checksum`)** | 4-lane 256-bit AVX2 checksum dispatch vs scalar fallback | [`benchmarks/simd/results.md`](../benchmarks/simd/results.md) |
| **Compiler Scalability** | Multi-thousand-line parse, check, MIR, and codegen scaling | [`benchmarks/scalability/latest.md`](../benchmarks/scalability/latest.md) |

---

## 2. Optimization Controls

- **Cranelift Optimization Levels**:
  - `-O0` (default): skips MIR optimization passes while still running MIR verification and ownership balance proofs; maps to Cranelift `none`.
  - `-O1` / `-O2` / `-O3`: enables bounded MIR optimization budgets and Cranelift `speed` optimization.
  - `-Os` / `-Oz`: selects size-oriented MIR budgets and Cranelift `speed_and_size`.
- **Whole-Program Scalar Inliner (`--lto`)**:
  - Inlines small, single-block, pure scalar callees across merged L++ MIR modules (not object-file or C-library LTO).
- **Profile-Guided MIR Inlining (`--pgo-generate` / `--pgo-use`)**:
  ```bash
  lpp app.lpp -O3 --pgo-generate -o app-inst
  LPP_PGO_PROFILE=app.pgo ./app-inst
  lpp app.lpp -O3 --pgo-use app.pgo -o app-pgo
  ```
  Records MIR function-entry counts in a fingerprint-checked `LPP-PGO-1` file and prioritizes hot callees in the bounded MIR inliner.

---

## 3. Running Local Benchmarks

Use `lpp bench` (or inspect `benchmarks/Benchmarks.md` for harness scripts):
```bash
lpp app.lpp -O2 -o app
```
Setting `BENCHMARK=1` in the environment prints a `TIMING_JSON` line reporting total compilation wall-clock duration.
