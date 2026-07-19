# Cross-language comparison

Generated: `2026-07-19T08:17:40.395768+00:00`

| Workload | Language | Compile ms | Link ms | Total build ms | Runtime ms | Status |
|---|---|---:|---:|---:|---:|---|
| fib35 | lpp | 4.302 | 213.406 | 217.708 | 53.552 | PASS |
| fib35 | c | 76.593 |  | 76.593 | 19.755 | PASS |
| fib35 | cpp | 123.844 |  | 123.844 | 18.432 | PASS |
| fib35 | rust | 84.954 |  | 84.954 | 34.070 | PASS |
| fib35 | go |  |  |  |  | SKIP |
| fib35 | zig |  |  |  |  | SKIP |
| loop10m | lpp | 2.842 | 200.608 | 203.449 | 9.874 | PASS |
| loop10m | c | 39.585 |  | 39.585 | 1.301 | PASS |
| loop10m | cpp | 52.859 |  | 52.859 | 1.452 | PASS |
| loop10m | rust | 74.333 |  | 74.333 | 1.574 | PASS |
| loop10m | go |  |  |  |  | SKIP |
| loop10m | zig |  |  |  |  | SKIP |
