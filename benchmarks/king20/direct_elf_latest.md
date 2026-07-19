# King 20 direct ELF subset

Generated: `2026-07-19T09:08:14.201174+00:00`

This report links without a host final linker. The freestanding runtime object is packaged before the run.

| # | Workload | Compile ms | Direct link ms | Runtime ms | Status |
|---:|---|---:|---:|---:|---|
| 1 | `recursive-fib-35` | 3.126 | 1.844 | 54.105 | PASS |
| 2 | `loop-10m` | 2.890 | 1.671 | 11.756 | PASS |
| 3 | `call-chain-1m` | 2.646 | 1.726 | 3.794 | PASS |
| 4 | `integer-arithmetic` | 2.260 | 1.666 | 0.460 | PASS |
| 5 | `conditional-branches` | 2.645 | 1.656 | 0.345 | PASS |
| 6 | `nested-direct-calls` | 2.657 | 1.652 | 0.451 | PASS |
