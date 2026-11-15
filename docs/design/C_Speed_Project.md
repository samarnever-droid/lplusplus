# Project Kick — C-Speed L++

## Mission

Make the verified L++ Cranelift AOT subset competitive with optimized C while preserving the four pillars:

| Pillar | Non-negotiable rule |
|---|---|
| Readable | No user-visible unsafe annotations, pointer arithmetic, or optimizer-only syntax required for ordinary code. |
| Safe | No optimization may erase ARC retain/release semantics, borrow boundaries, checked rejection, or destructor behavior. |
| Fast iteration | Compile-latency regressions are measured separately from runtime improvements. |
| Native | Benchmarks use real native executables, not interpreter or synthetic IR-only numbers. |

## Baseline

The current canonical workload is `loop10m`:

```text
C -O3:                 1.186 ms median
L++ Cranelift speed:   7.291 ms median
L++ / C ratio:         6.15x
```

This is a baseline, not a claim that all workloads have the same ratio. The cross-language harness records toolchain versions, correctness, warmups, repetitions, median, and p95.

## Order of attack

1. **Measure generated native code** — inspect disassembly and distinguish loop code, call ABI cost, ARC overhead, and runtime I/O from compiler time.
2. **Scalar MIR optimizations** — constant propagation, dead temporary elimination, branch simplification, and safe direct-call inlining for proven scalar-only functions.
3. **Cranelift lowering quality** — reduce redundant variables/instructions, preserve SSA opportunities, use appropriate optimization profile per build mode.
4. **ARC optimization** — eliminate only provably paired retain/release operations; every change needs ownership regression tests.
5. **Runtime ABI** — reduce external-call and allocation overhead without allocator/FFI ambiguity.
6. **Profile-guided decisions** — retain a change only when it improves canonical benchmarks without violating compile-latency or safety gates.

## Hard gates

Every C-Speed change must pass:

```sh
cargo test --locked
sh tests/run_aot_parity.sh
sh scripts/run_s1_rejection.sh
python3 benchmarks/comparison/run.py --repetitions 5 --warmups 1
```

When native C/Rust runtime or ABI code changes, it must also pass S0 tools:

```sh
sh scripts/run_s0_safety.sh
```

## Explicit non-goals

- Faking speed by removing ARC or changing language semantics.
- Comparing different algorithms or unchecked arithmetic.
- Publishing a single lucky benchmark run as a performance claim.
- Regressing source readability to match C syntax.
