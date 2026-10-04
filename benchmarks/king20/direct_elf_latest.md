# King 20 direct ELF subset

Generated: `2026-07-20T18:17:47.026494+00:00`

This report links without a host final linker. The freestanding runtime object is packaged before the run.

| # | Workload | Compile ms | Direct link ms | Runtime ms | Status |
|---:|---|---:|---:|---:|---|
| 1 | `recursive-fib-35` | 3.735 | 1.663 | 70.016 | PASS |
| 2 | `loop-10m` | 2.742 | 1.626 | 9.944 | PASS |
| 3 | `call-chain-1m` | 2.849 | 1.560 | 2.590 | PASS |
| 4 | `integer-arithmetic` | 2.271 | 1.530 | 0.305 | PASS |
| 5 | `conditional-branches` | 2.676 | 1.709 | 0.334 | PASS |
| 6 | `nested-direct-calls` | 2.488 | 1.498 | 0.268 | PASS |
| 7 | `immutable-closure` | 2.324 | 1.620 | 0.238 | PASS |
| 8 | `arc-list-int` | 2.265 | 1.630 | 0.629 | PASS |
| 9 | `owned-struct-return` | 3.509 | 1.861 | 0.324 | PASS |
| 10 | `branch-owned-return` | 2.446 | 1.508 | 0.220 | PASS |
| 11 | `nested-struct-destructor` | 2.714 | 1.669 | 0.331 | PASS |
| 12 | `direct-arc-alias` | 2.363 | 1.500 | 0.269 | PASS |
| 13 | `closure-arc-capture` | 2.289 | 1.464 | 0.345 | PASS |
| 14 | `borrowed-parameter-return` | 2.156 | 1.513 | 0.314 | PASS |
| 15 | `borrowed-field-return` | 2.676 | 1.630 | 0.347 | PASS |
| 16 | `field-alias` | 2.311 | 1.540 | 0.273 | PASS |
| 17 | `list-int-alias` | 2.072 | 1.623 | 0.292 | PASS |
| 18 | `list-custom-ownership` | 2.189 | 1.555 | 0.240 | PASS |
| 19 | `nested-branch-alias` | 2.555 | 1.414 | 0.272 | PASS |
| 20 | `closure-branch-capture` | 2.355 | 1.400 | 0.268 | PASS |
