# Runtime-dispatched SIMD checksum benchmark

**Run:** 2026-10-26, Linux x86-64, AVX2 detected. Both L++ executables were
compiled with Cranelift `-O3`, linked with `cc`, and run against the optimized
Rust runtime at `target/release/liblpp_runtime.so`. Nine fresh-process samples
were taken per workload. The program output was identical in both cases:
`3840710418224960`.

| Workload | Median | Minimum |
|---|---:|---:|
| `vec_i64_checksum(50_000_000)` (runtime dispatch; 4 × i64 AVX2 on this host) | 13.933 ms | 13.881 ms |
| Pure L++ scalar loop, same arithmetic and iteration count | 32.650 ms | 32.276 ms |

**Median speedup: 2.34×.** This is one checked-in runtime intrinsic, not a claim
that arbitrary L++ loops are automatically vectorized. On hosts without AVX2,
the intrinsic takes its scalar fallback. `benchmarks/simd/run.py` reproduces
the measurement and refuses to report a result if the outputs differ.

Command:

```sh
cargo build --locked --release -p lpp-runtime
python3 benchmarks/simd/run.py \
  --lpp target/debug/lpp \
  --runtime target/release/liblpp_runtime.so \
  --samples 9
```
