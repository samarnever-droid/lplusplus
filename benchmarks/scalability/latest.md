# L++ Compiler Scalability

Generated: `2026-07-19T08:13:04.062922+00:00`

## System

- Platform: `Linux-6.1.158+-x86_64-with-glibc2.41`
- CPU: `Intel(R) Xeon(R) Processor @ 2.60GHz`
- Logical CPUs: `2`
- Memory: `1991.6 MiB`

## Phase scaling

Single-run development measurements. Link time is reported separately because it is dominated by the host linker.

| LOC | I/O ms | Lex ms | Parse ms | Semantic ms | Typecheck ms | Escape ms | MIR ms | AOT ms | Compiler total ms | Link ms |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10000 | 0.127 | 2.545 | 4.673 | 0.925 | 0.444 | 62.631 | 2.121 | 56.874 | 71.351 | 211.267 |
| 50000 | 0.529 | 12.129 | 23.813 | 5.076 | 2.948 | 345.913 | 10.083 | 317.934 | 390.416 | 200.618 |
| 100000 | 1.003 | 26.405 | 49.233 | 9.601 | 5.816 | 691.433 | 19.834 | 634.006 | 783.497 | 202.726 |
