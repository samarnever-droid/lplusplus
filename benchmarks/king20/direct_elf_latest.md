# King 20 direct ELF subset

Generated: `2026-07-19T09:31:51.140188+00:00`

This report links without a host final linker. The freestanding runtime object is packaged before the run.

| # | Workload | Compile ms | Direct link ms | Runtime ms | Status |
|---:|---|---:|---:|---:|---|
| 1 | `recursive-fib-35` | 2.873 | 3.972 | 56.154 | PASS |
| 2 | `loop-10m` | 2.835 | 1.691 | 9.344 | PASS |
| 3 | `call-chain-1m` | 2.727 | 1.687 | 3.481 | PASS |
| 4 | `integer-arithmetic` | 2.374 | 1.578 | 0.344 | PASS |
| 5 | `conditional-branches` | 2.310 | 1.591 | 0.303 | PASS |
| 6 | `nested-direct-calls` | 2.353 | 1.883 | 0.323 | PASS |
| 7 | `immutable-closure` | 2.363 | 1.712 | 0.663 | PASS |
| 9 | `owned-struct-return` | 2.357 | 1.703 | 0.269 | PASS |
| 10 | `branch-owned-return` | 2.876 | 1.575 | 0.252 | PASS |
| 11 | `nested-struct-destructor` | 2.391 | 1.510 | 0.236 | PASS |
| 12 | `direct-arc-alias` | 2.287 | 1.599 | 0.259 | PASS |
| 13 | `closure-arc-capture` | 2.365 | 1.704 | 0.294 | PASS |
| 14 | `borrowed-parameter-return` | 2.196 | 1.578 | 0.207 | PASS |
| 15 | `borrowed-field-return` | 2.357 | 1.674 | 0.350 | PASS |
| 16 | `field-alias` | 2.521 | 1.614 | 0.196 | PASS |
| 19 | `nested-branch-alias` | 2.468 | 1.644 | 0.399 | PASS |
| 20 | `closure-branch-capture` | 2.488 | 1.597 | 0.381 | PASS |
