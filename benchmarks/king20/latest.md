# L++ King 20 Benchmark Results

Generated: `2026-07-19T08:01:49.618585+00:00`

## System information

- Platform: `Linux-6.1.158+-x86_64-with-glibc2.41`
- CPU: `Intel(R) Xeon(R) Processor @ 2.60GHz`
- Logical CPUs: `2`
- Memory: `1991.6 MiB`
- Rust: `rustc 1.97.1 (8bab26f4f 2026-07-14)`
- C compiler: `cc (Debian 14.2.0-19) 14.2.0`

## Results

Single-run development measurements. A result is recorded only after stdout and process exit status match the manifest.

| # | Benchmark | Compiler ms | AOT ms | Link ms | Runtime ms | Object B | EXE B | Status |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | `recursive-fib-35` | 0.797 | 0.673 | 220.143 | 54.594 | 2424 | 24120 | PASS |
| 2 | `loop-10m` | 0.899 | 0.735 | 202.139 | 10.381 | 2320 | 24120 | PASS |
| 3 | `call-chain-1m` | 0.973 | 0.819 | 202.220 | 4.558 | 2512 | 24176 | PASS |
| 4 | `integer-arithmetic` | 0.827 | 0.703 | 202.509 | 1.704 | 2352 | 24096 | PASS |
| 5 | `conditional-branches` | 1.065 | 0.933 | 200.551 | 1.709 | 2464 | 24128 | PASS |
| 6 | `nested-direct-calls` | 0.912 | 0.776 | 200.063 | 1.463 | 2464 | 24152 | PASS |
| 7 | `immutable-closure` | 0.892 | 0.720 | 199.903 | 1.375 | 2616 | 24192 | PASS |
| 8 | `arc-list-int` | 0.912 | 0.713 | 199.569 | 1.493 | 2688 | 24096 | PASS |
| 9 | `owned-struct-return` | 0.763 | 0.638 | 201.929 | 1.300 | 2472 | 24168 | PASS |
| 10 | `branch-owned-return` | 0.962 | 0.820 | 199.597 | 1.470 | 2560 | 24160 | PASS |
| 11 | `nested-struct-destructor` | 0.827 | 0.707 | 202.221 | 1.429 | 2656 | 24176 | PASS |
| 12 | `direct-arc-alias` | 0.841 | 0.729 | 198.200 | 1.487 | 2472 | 24136 | PASS |
| 13 | `closure-arc-capture` | 1.016 | 0.876 | 200.128 | 1.557 | 2856 | 24232 | PASS |
| 14 | `borrowed-parameter-return` | 0.864 | 0.684 | 207.183 | 1.387 | 2576 | 24168 | PASS |
| 15 | `borrowed-field-return` | 0.897 | 0.725 | 203.778 | 1.404 | 2840 | 24208 | PASS |
| 16 | `field-alias` | 0.882 | 0.747 | 202.899 | 1.389 | 2752 | 24176 | PASS |
| 17 | `list-int-alias` | 0.712 | 0.589 | 200.339 | 1.336 | 2464 | 24096 | PASS |
| 18 | `list-custom-ownership` | 0.777 | 0.654 | 198.193 | 1.625 | 2648 | 24136 | PASS |
| 19 | `nested-branch-alias` | 0.928 | 0.790 | 200.477 | 1.629 | 2768 | 24176 | PASS |
| 20 | `closure-branch-capture` | 1.047 | 0.880 | 199.106 | 1.405 | 2880 | 24232 | PASS |

## Method

Each source file is compiled with `LPP_AOT=1` and `BENCHMARK=1`, linked with the host C compiler and `lpp_runtime.c`, then executed once. The external link step is reported separately because Cranelift currently emits an object file; a host linker is still required for a standalone executable.
