# L++ Compiler Scalability

Generated: `2026-07-20T02:39:36.929512+00:00`

## System

- Platform: `Linux-6.1.158+-x86_64-with-glibc2.41`
- CPU: `Intel(R) Xeon(R) Processor @ 2.60GHz`
- Logical CPUs: `2`
- Memory: `1991.6 MiB`

## Phase scaling

Single-run development measurements. Link time is reported separately because it is dominated by the host linker.

| LOC | I/O ms | Lex ms | Parse ms | Semantic ms | Typecheck ms | Escape ms | MIR ms | AOT ms | C codegen ms | Compiler total ms | Link ms |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10000 | 0.120 | 2.320 | 4.334 | 0.982 | 0.414 | 0.224 | 2.040 | 56.837 | 3.435 | 70.821 | 203.778 |
| 50000 | 0.600 | 11.374 | 21.783 | 5.610 | 2.934 | 2.984 | 9.756 | 326.131 | 16.656 | 398.261 | 201.417 |
| 100000 | 0.999 | 22.684 | 42.074 | 11.506 | 6.142 | 6.046 | 20.180 | 662.331 | 36.292 | 809.006 | 198.762 |
