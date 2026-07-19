# L++ King 20 Benchmark Results

Generated: `2026-07-19T08:08:07.881412+00:00`

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
| 1 | `recursive-fib-35` | 0.739 | 0.628 | 214.386 | 53.843 | 2424 | 24120 | PASS |
| 2 | `loop-10m` | 0.875 | 0.742 | 202.575 | 12.786 | 2320 | 24120 | PASS |
| 3 | `call-chain-1m` | 1.039 | 0.889 | 196.705 | 5.516 | 2512 | 24176 | PASS |
| 4 | `integer-arithmetic` | 0.723 | 0.612 | 198.631 | 1.391 | 2352 | 24096 | PASS |
| 5 | `conditional-branches` | 0.828 | 0.699 | 198.760 | 1.471 | 2464 | 24128 | PASS |
| 6 | `nested-direct-calls` | 0.920 | 0.789 | 198.259 | 1.347 | 2464 | 24152 | PASS |
| 7 | `immutable-closure` | 0.910 | 0.763 | 201.348 | 1.281 | 2616 | 24192 | PASS |
| 8 | `arc-list-int` | 0.842 | 0.686 | 199.505 | 1.445 | 2688 | 24096 | PASS |
| 9 | `owned-struct-return` | 0.981 | 0.756 | 198.424 | 1.527 | 2472 | 24168 | PASS |
| 10 | `branch-owned-return` | 0.949 | 0.805 | 213.320 | 1.849 | 2560 | 24160 | PASS |
| 11 | `nested-struct-destructor` | 1.082 | 0.876 | 205.325 | 1.327 | 2656 | 24176 | PASS |
| 12 | `direct-arc-alias` | 0.825 | 0.715 | 216.071 | 1.646 | 2472 | 24136 | PASS |
| 13 | `closure-arc-capture` | 1.387 | 1.162 | 202.995 | 1.397 | 2856 | 24232 | PASS |
| 14 | `borrowed-parameter-return` | 0.863 | 0.733 | 207.846 | 1.508 | 2576 | 24168 | PASS |
| 15 | `borrowed-field-return` | 1.102 | 0.957 | 201.630 | 1.389 | 2840 | 24208 | PASS |
| 16 | `field-alias` | 0.854 | 0.727 | 201.605 | 1.579 | 2752 | 24176 | PASS |
| 17 | `list-int-alias` | 0.768 | 0.615 | 201.646 | 1.381 | 2464 | 24096 | PASS |
| 18 | `list-custom-ownership` | 0.774 | 0.651 | 199.372 | 1.736 | 2648 | 24136 | PASS |
| 19 | `nested-branch-alias` | 0.953 | 0.813 | 202.499 | 1.331 | 2768 | 24176 | PASS |
| 20 | `closure-branch-capture` | 1.030 | 0.869 | 198.630 | 1.458 | 2880 | 24232 | PASS |

## Method

Each source file is compiled with `LPP_AOT=1` and `BENCHMARK=1`, linked with the host C compiler and `lpp_runtime.c`, then executed once. The external link step is reported separately because Cranelift currently emits an object file; a host linker is still required for a standalone executable.
