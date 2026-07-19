# L++ King 20 Benchmark Results

Generated: `2026-07-19T08:08:12.244795+00:00`

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
| 1 | `recursive-fib-35` | 0.916 | 0.786 | 200.249 | 71.903 | 2424 | 24120 | PASS |
| 2 | `loop-10m` | 0.867 | 0.707 | 199.418 | 11.790 | 2320 | 24120 | PASS |
| 3 | `call-chain-1m` | 1.122 | 0.964 | 219.669 | 5.402 | 2512 | 24176 | PASS |
| 4 | `integer-arithmetic` | 0.843 | 0.652 | 200.652 | 1.459 | 2352 | 24096 | PASS |
| 5 | `conditional-branches` | 0.815 | 0.686 | 203.692 | 1.332 | 2464 | 24128 | PASS |
| 6 | `nested-direct-calls` | 0.937 | 0.794 | 202.769 | 1.420 | 2464 | 24152 | PASS |
| 7 | `immutable-closure` | 0.868 | 0.730 | 202.507 | 1.307 | 2616 | 24192 | PASS |
| 8 | `arc-list-int` | 0.868 | 0.735 | 202.134 | 1.628 | 2688 | 24096 | PASS |
| 9 | `owned-struct-return` | 0.931 | 0.802 | 197.728 | 1.445 | 2472 | 24168 | PASS |
| 10 | `branch-owned-return` | 0.927 | 0.779 | 203.968 | 1.434 | 2560 | 24160 | PASS |
| 11 | `nested-struct-destructor` | 0.835 | 0.685 | 208.863 | 1.607 | 2656 | 24176 | PASS |
| 12 | `direct-arc-alias` | 0.772 | 0.656 | 199.411 | 1.435 | 2472 | 24136 | PASS |
| 13 | `closure-arc-capture` | 1.160 | 0.945 | 199.814 | 1.383 | 2856 | 24232 | PASS |
| 14 | `borrowed-parameter-return` | 0.911 | 0.786 | 200.228 | 1.385 | 2576 | 24168 | PASS |
| 15 | `borrowed-field-return` | 0.915 | 0.779 | 199.673 | 1.408 | 2840 | 24208 | PASS |
| 16 | `field-alias` | 1.029 | 0.892 | 199.570 | 1.321 | 2752 | 24176 | PASS |
| 17 | `list-int-alias` | 0.719 | 0.593 | 199.893 | 1.598 | 2464 | 24096 | PASS |
| 18 | `list-custom-ownership` | 0.824 | 0.690 | 198.678 | 1.432 | 2648 | 24136 | PASS |
| 19 | `nested-branch-alias` | 1.028 | 0.827 | 203.932 | 1.302 | 2768 | 24176 | PASS |
| 20 | `closure-branch-capture` | 1.025 | 0.878 | 204.010 | 1.430 | 2880 | 24232 | PASS |

## Method

Each source file is compiled with `LPP_AOT=1` and `BENCHMARK=1`, linked with the host C compiler and `lpp_runtime.c`, then executed once. The external link step is reported separately because Cranelift currently emits an object file; a host linker is still required for a standalone executable.
