# King 20 direct ELF subset

Generated: `2026-07-19T09:43:38.017661+00:00`

This report links without a host final linker. The freestanding runtime object is packaged before the run.

| # | Workload | Compile ms | Direct link ms | Runtime ms | Status |
|---:|---|---:|---:|---:|---|
| 1 | `recursive-fib-35` | 2.971 | 1.797 | 56.520 | PASS |
| 2 | `loop-10m` | 2.853 | 1.779 | 11.018 | PASS |
| 3 | `call-chain-1m` | 2.630 | 1.667 | 4.601 | PASS |
| 4 | `integer-arithmetic` | 2.291 | 1.717 | 0.493 | PASS |
| 5 | `conditional-branches` | 2.491 | 1.690 | 0.353 | PASS |
| 6 | `nested-direct-calls` | 2.571 | 1.797 | 0.533 | PASS |
| 7 | `immutable-closure` | 2.931 | 1.771 | 0.487 | PASS |
| 8 | `arc-list-int` | 2.411 | 2.123 | 0.439 | PASS |
| 9 | `owned-struct-return` | 2.404 | 1.640 | 0.302 | PASS |
| 10 | `branch-owned-return` | 2.503 | 1.757 | 0.314 | PASS |
| 11 | `nested-struct-destructor` | 2.530 | 1.774 | 0.263 | PASS |
| 12 | `direct-arc-alias` | 2.207 | 1.814 | 0.369 | PASS |
| 13 | `closure-arc-capture` | 2.567 | 1.930 | 0.300 | PASS |
| 14 | `borrowed-parameter-return` | 2.343 | 1.572 | 0.285 | PASS |
| 15 | `borrowed-field-return` | 2.307 | 1.586 | 0.248 | PASS |
| 16 | `field-alias` | 2.302 | 1.574 | 0.277 | PASS |
| 17 | `list-int-alias` | 2.327 | 1.656 | 0.382 | PASS |
| 18 | `list-custom-ownership` | 2.396 | 1.848 | 0.356 | PASS |
| 19 | `nested-branch-alias` | 2.367 | 2.058 | 0.340 | PASS |
| 20 | `closure-branch-capture` | 2.502 | 1.630 | 0.370 | PASS |
