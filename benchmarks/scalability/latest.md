# L++ Compiler Scalability

Generated: `2026-07-20T03:08:28.298775+00:00`

## System

- Platform: `Linux-6.1.158+-x86_64-with-glibc2.41`
- CPU: `Intel(R) Xeon(R) Processor @ 2.60GHz`
- Logical CPUs: `2`
- Memory: `1991.6 MiB`

## Phase scaling

Single-run development measurements. Link time is reported separately because it is dominated by the host linker.

| LOC | I/O ms | Lex ms | Parse ms | Semantic ms | Typecheck ms | Escape ms | MIR ms | AOT ms | C codegen ms | Compiler total ms | Link ms |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10000 | 0.109 | 2.130 | 4.246 | 0.894 | 0.382 | 0.201 | 2.104 | 54.973 | 0.000 | 65.048 | 223.065 |
| 50000 | 0.412 | 10.881 | 21.618 | 4.622 | 2.720 | 2.074 | 10.155 | 299.537 | 0.000 | 352.034 | 196.992 |
| 100000 | 1.686 | 23.575 | 42.714 | 9.382 | 5.421 | 5.252 | 19.948 | 637.807 | 0.000 | 745.798 | 195.969 |
