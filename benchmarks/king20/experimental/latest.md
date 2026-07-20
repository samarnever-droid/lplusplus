# L++ King 20 Benchmark Results

Generated: `2026-07-20T18:17:44.863727+00:00`

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
| 1 | `recursive-fib-35` | 1.000 | 0.832 | 236.608 | 70.830 | 2624 | 24424 | PASS |
| 2 | `loop-10m` | 0.813 | 0.675 | 213.829 | 7.776 | 2520 | 24424 | PASS |
| 3 | `call-chain-1m` | 0.897 | 0.730 | 215.492 | 3.586 | 2712 | 24480 | PASS |
| 4 | `integer-arithmetic` | 0.696 | 0.553 | 217.119 | 1.350 | 2544 | 24392 | PASS |
| 5 | `conditional-branches` | 0.763 | 0.639 | 215.461 | 1.336 | 2664 | 24424 | PASS |
| 6 | `nested-direct-calls` | 0.788 | 0.658 | 215.513 | 1.742 | 2664 | 24456 | PASS |
| 7 | `immutable-closure` | 1.340 | 1.113 | 213.956 | 1.307 | 2816 | 24496 | PASS |
| 8 | `arc-list-int` | 0.737 | 0.602 | 215.354 | 1.396 | 2888 | 24392 | PASS |
| 9 | `owned-struct-return` | 0.692 | 0.571 | 215.673 | 1.406 | 2672 | 24464 | PASS |
| 10 | `branch-owned-return` | 0.824 | 0.653 | 222.046 | 1.311 | 2760 | 24464 | PASS |
| 11 | `nested-struct-destructor` | 0.782 | 0.626 | 215.347 | 1.395 | 2856 | 24480 | PASS |
| 12 | `direct-arc-alias` | 0.765 | 0.627 | 245.281 | 1.273 | 2664 | 24432 | PASS |
| 13 | `closure-arc-capture` | 0.880 | 0.741 | 208.631 | 1.294 | 3056 | 24528 | PASS |
| 14 | `borrowed-parameter-return` | 0.842 | 0.702 | 210.762 | 1.297 | 2776 | 24464 | PASS |
| 15 | `borrowed-field-return` | 0.865 | 0.731 | 212.936 | 1.281 | 3040 | 24512 | PASS |
| 16 | `field-alias` | 0.866 | 0.727 | 212.194 | 1.454 | 2952 | 24480 | PASS |
| 17 | `list-int-alias` | 0.663 | 0.537 | 213.131 | 1.326 | 2664 | 24392 | PASS |
| 18 | `list-custom-ownership` | 0.862 | 0.717 | 213.885 | 1.313 | 2840 | 24432 | PASS |
| 19 | `nested-branch-alias` | 0.912 | 0.763 | 213.694 | 1.335 | 2968 | 24480 | PASS |
| 20 | `closure-branch-capture` | 1.084 | 0.918 | 212.970 | 1.311 | 3080 | 24528 | PASS |

## Method

Each source file is compiled with `LPP_AOT=1` and `BENCHMARK=1`, linked with the host C compiler and `lpp_runtime.c`, then executed once. The external link step is reported separately because Cranelift currently emits an object file; a host linker is still required for a standalone executable.
