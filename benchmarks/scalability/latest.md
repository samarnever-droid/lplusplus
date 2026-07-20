# L++ Compiler Scalability

Generated: `2026-07-20T12:26:44.419655+00:00`

## System

- Platform: `Linux-6.1.158+-x86_64-with-glibc2.41`
- CPU: `Intel(R) Xeon(R) Processor @ 2.60GHz`
- Logical CPUs: `2`
- Memory: `1991.6 MiB`

## Phase scaling

Single-run development measurements. Link time is reported separately because it is dominated by the host linker.

| LOC | I/O ms | Lex ms | Parse ms | Semantic ms | Typecheck ms | Escape ms | MIR ms | AOT ms | C codegen ms | Compiler total ms | Link ms |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10000 | 0.122 | 2.495 | 4.753 | 0.850 | 0.413 | 0.231 | 4.692 | 2.164 | 0.000 | 15.728 | 210.175 |
| 50000 | 0.464 | 12.053 | 24.284 | 4.046 | 2.704 | 1.008 | 25.108 | 8.887 | 0.000 | 78.565 | 243.908 |
| 100000 | 1.020 | 25.479 | 48.950 | 8.342 | 5.681 | 2.240 | 53.820 | 18.753 | 0.000 | 164.299 | 240.153 |
