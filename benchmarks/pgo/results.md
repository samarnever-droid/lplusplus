# Profile-guided MIR inlining benchmark

**Run:** 2026-10-26, Linux x86-64, Cranelift `-O3`, `cc` host linker. Reproduce
from the repository root with:

```sh
cargo build --locked -p lpp-runtime
cargo build --locked --bin lpp
python3 benchmarks/pgo/run.py --samples 7
```

The fixture calls the tiny pure `plus_one` function inside a ten-million-iteration
loop. The harness builds an instrumented executable, runs it once to write a
fingerprinted profile, then compares seven fresh-process executions of the
uninstrumented baseline and the executable compiled with `--pgo-use`. Both
variants printed the same result, `50000005000000`; the generated profile had
three function-entry records.

| Variant | Median | Minimum |
|---|---:|---:|
| `-O3`, without PGO | 14.027 ms | 13.826 ms |
| `-O3 --pgo-use hot.pgo` | 7.704 ms | 7.591 ms |

**Median speedup: 1.82×.** The PGO run selects the hot `plus_one` callee for the
bounded MIR inliner; it does not guide backend machine-code optimization.

The single instrumented profile-generation run took **282.278 ms** on this
fixture (about 20× the baseline median). This is expected overhead from
counting every function entry and is not included in the steady-state timings.
The runtime lazily initializes a dense table of 1,048,576 relaxed atomic
counters (about 8 MiB); the table is scanned once when source `main` returns.
Profiles are written to `LPP_PGO_PROFILE`, or `lpp.pgo` by default, and are
accepted only when their pre-optimization MIR fingerprint matches the source.

This is an initial workload-specific result, not a claim about general
profile-guided optimization or arbitrary programs. See
[`../docs/CURRENT_CAPABILITIES.md`](../../docs/CURRENT_CAPABILITIES.md)
for supported flags and limits.
