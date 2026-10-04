# L++ Compiler Scalability

Generated: `2026-07-20T17:00:05.284935+00:00`

## System

- Platform: `Windows-11-10.0.26200-SP0`
- CPU: `unknown`
- Logical CPUs: `8`
- Memory: `None MiB`

## Phase scaling

Single-run development measurements. Link time is reported separately because it is dominated by the host linker.

| LOC | I/O ms | Lex ms | Parse ms | Semantic ms | Typecheck ms | Escape ms | MIR ms | AOT ms | C codegen ms | Compiler total ms | Link ms |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10000 | 48.976 | 5.631 | 9.846 | 1.818 | 0.878 | 0.463 | 10.990 | 6.639 | 0.000 | 85.317 | 467.698 |
| 50000 | 41.744 | 65.071 | 52.364 | 6.814 | 4.397 | 3.138 | 39.592 | 16.264 | 0.000 | 229.435 | 426.189 |
| 100000 | 120.273 | 76.576 | 89.345 | 19.601 | 8.426 | 5.139 | 75.705 | 30.110 | 0.000 | 425.250 | 406.252 |
