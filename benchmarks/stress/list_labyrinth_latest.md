# List Labyrinth backend benchmark

Workload: **18,415 lines**, List-heavy game + file-write integration.

| Path | Emit / compile ms | Host link ms | Run ms | Output |
|---|---:|---:|---:|---|
| C backend | 188.234 | 9697.204 | 1.800 | `552` |
| Cranelift AOT | 432.783 | 232.298 | 1.409 | `552` |

## Direct native linker

Status: **rejected as unsupported**. This workload intentionally calls `write_file`; direct Linux ELF does not currently provide file I/O/writable-data support. Rejection is correct safety behavior, not a benchmark failure.

Raw data: `list_labyrinth_latest.json`.
