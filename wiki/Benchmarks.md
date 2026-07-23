# Benchmarks (BPW v3)

## Cross-Platform Results

**Linux x86-64** (AMD EPYC / Intel Xeon, 15GB RAM, Ubuntu 24.04):

| Benchmark | L++ (avg) | Rust (avg) | Go (avg) | L++ Size | Rust Size | Go Size |
|-----------|----------|-----------|---------|---------|----------|--------|
| CPU-Heavy (fib40 + primes 50k) | 4ms | 3ms | 5ms | **47KB** | 435KB | 2345KB |
| RAM-Heavy (500k list) | 3ms | 2ms | 5ms | **47KB** | 436KB | 2345KB |
| File I/O (400KB write+read) | **1ms** | 6ms | 5ms | **47KB** | 445KB | 2470KB |
| Multi-file compile | 4ms | — | — | 47KB | — | — |
| Large project (5k LOC) | 10ms | — | — | 62KB | — | — |

**Windows x86-64** (PE direct via lpp-link, no MSVC):

| Benchmark | L++ | Rust | Go | L++ Size |
|-----------|-----|------|-----|---------|
| CPU-Heavy | 7ms | 7ms | 10ms | **15.5KB** |
| RAM-Heavy | 8ms | — | — | **15.5KB** |

## Key Takeaways

- **Smallest binaries**: 15.5KB (Windows PE) / 47KB (Linux) vs Rust 435KB / Go 2345KB
- **Fastest compile**: 4ms for multi-file project, 10ms for 5000 LOC
- **Competitive runtime**: Within 1-2ms of Rust on CPU/RAM benchmarks
- **File I/O champion**: 1ms vs Rust 6ms / Go 5ms (thanks to `str_repeat` O(1) allocation)

## Methodology

- 10 runs per benchmark, reporting avg/min/max/stddev
- System info recorded (CPU model, RAM, OS version, compiler versions)
- All languages use release/optimized builds
- All outputs verified identical across languages
- All source code included in artifacts for reproducibility

## Running BPW

The Benchmark Package Workflow runs automatically on push:

```
.github/workflows/bpw.yml
```

Three jobs: `bpw-linux` → `bpw-windows` → `bpw-merge` (downloads both, generates cross-platform report)
