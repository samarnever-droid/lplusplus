# Cross-language comparison

Generated: `2026-07-20T02:55:26.084667+00:00`; median of 3 runs after 1 warmups.

| Workload | Language | Build median ms | Runtime median ms | Runtime p95 ms | Status |
|---|---|---:|---:|---:|---|
| fib35 | lpp-aot-none | 358.612 | 66.027 | 66.488 | PASS |
| fib35 | lpp-aot-speed | 200.213 | 66.379 | 67.111 | PASS |
| fib35 | c | 67.869 | 19.016 | 19.216 | PASS |
| fib35 | cpp | 85.084 | 19.171 | 19.899 | PASS |
| fib35 | rust | 72.363 | 28.489 | 28.566 | PASS |
| fib35 | go | 0.000 |  |  | SKIP |
| fib35 | zig | 0.000 |  |  | SKIP |
| fib35 | java | 532.898 | 104.59 | 105.035 | PASS |
| fib35 | python | 0.000 | 1195.021 | 1211.35 | PASS |
| fib35 | node | 0.000 | 128.308 | 128.966 | PASS |
| fib35 | ruby | 0.000 |  |  | SKIP |
| loop10m | lpp-aot-none | 202.221 | 10.363 | 10.411 | PASS |
| loop10m | lpp-aot-speed | 201.084 | 7.291 | 7.453 | PASS |
| loop10m | c | 34.794 | 1.186 | 1.267 | PASS |
| loop10m | cpp | 47.909 | 1.164 | 1.194 | PASS |
| loop10m | rust | 70.664 | 1.428 | 1.435 | PASS |
| loop10m | go | 0.000 |  |  | SKIP |
| loop10m | zig | 0.000 |  |  | SKIP |
| loop10m | java | 498.939 | 63.76 | 64.166 | PASS |
| loop10m | python | 0.000 | 1038.542 | 1039.38 | PASS |
| loop10m | node | 0.000 | 348.065 | 349.455 | PASS |
| loop10m | ruby | 0.000 |  |  | SKIP |
