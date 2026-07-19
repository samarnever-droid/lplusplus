# King 20 direct ELF subset

Generated: `2026-07-19T09:21:28.540019+00:00`

This report links without a host final linker. The freestanding runtime object is packaged before the run.

| # | Workload | Compile ms | Direct link ms | Runtime ms | Status |
|---:|---|---:|---:|---:|---|
| 1 | `recursive-fib-35` | 3.132 | 1.786 | 59.090 | PASS |
| 2 | `loop-10m` | 2.959 | 1.834 | 10.854 | PASS |
| 3 | `call-chain-1m` | 3.418 | 2.110 | 3.538 | PASS |
| 4 | `integer-arithmetic` | 2.438 | 1.650 | 0.314 | PASS |
| 5 | `conditional-branches` | 2.528 | 2.064 | 0.346 | PASS |
| 6 | `nested-direct-calls` | 2.810 | 1.773 | 0.361 | PASS |
| 7 | `immutable-closure` | 2.574 | 1.586 | 0.280 | PASS |
| 9 | `owned-struct-return` | 2.243 | 1.544 | 0.230 | PASS |
| 10 | `branch-owned-return` | 2.328 | 1.504 | 0.224 | PASS |
| 11 | `nested-struct-destructor` | 2.277 | 1.549 | 0.226 | PASS |
| 12 | `direct-arc-alias` | 2.267 | 1.551 | 0.214 | PASS |
| 13 | `closure-arc-capture` | 2.309 | 1.635 | 0.254 | PASS |
| 14 | `borrowed-parameter-return` | 2.536 | 1.632 | 0.227 | PASS |
| 15 | `borrowed-field-return` | 2.339 | 1.660 | 0.236 | PASS |
| 16 | `field-alias` | 2.454 | 1.549 | 0.249 | PASS |
| 19 | `nested-branch-alias` | 2.836 | 2.073 | 0.242 | PASS |
| 20 | `closure-branch-capture` | 2.460 | 1.662 | 0.234 | PASS |
