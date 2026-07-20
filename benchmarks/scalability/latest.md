# L++ Compiler Scalability

Generated: `2026-07-20T03:13:37.225375+00:00`

## System

- Platform: `Linux-6.1.158+-x86_64-with-glibc2.41`
- CPU: `Intel(R) Xeon(R) Processor @ 2.60GHz`
- Logical CPUs: `2`
- Memory: `1991.6 MiB`

## Phase scaling

Single-run development measurements. Link time is reported separately because it is dominated by the host linker.

| LOC | I/O ms | Lex ms | Parse ms | Semantic ms | Typecheck ms | Escape ms | MIR ms | AOT ms | C codegen ms | Compiler total ms | Link ms |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10000 | 0.099 | 2.210 | 4.570 | 0.838 | 0.390 | 0.215 | 3.950 | 15.968 | 0.000 | 28.248 | 636.102 |
| 50000 | 0.445 | 11.434 | 30.431 | 6.061 | 3.055 | 2.208 | 20.721 | 93.791 | 0.000 | 168.159 | 203.267 |
| 100000 | 0.971 | 22.410 | 48.883 | 16.333 | 6.020 | 5.398 | 55.998 | 208.010 | 0.000 | 364.038 | 196.817 |
