# L++ King 20 Standard

The King 20 suite is a numbered AOT standard for both native performance and ownership correctness.

## What it checks

1. recursive fib(35)
2. loop throughput
3. call-chain throughput
4. integer arithmetic
5. branches
6. nested direct calls
7. immutable closure invocation
8. ARC `List[Int]`
9. owned struct return
10. branch-specific owned return
11. nested struct destructor chain
12. direct ARC aliases
13. closure ARC capture
14. borrowed parameter returned owned
15. borrowed field returned owned
16. field aliases
17. `List[Int]` aliases
18. `List[Custom]` ownership
19. nested branch aliases
20. closure capture under branch flow

Every item is compiled with Cranelift AOT, linked into a native executable, run once, and checked for exact stdout and exit status `0`.

## Running

```sh
python3 benchmarks/king20/run.py
```

Requirements:

```text
cargo
rustc
cc/gcc/clang
Python 3
```

The runner writes:

```text
latest.json  machine-readable metrics and system information
latest.md    human-readable report
```

`latest.*` is a single-run development snapshot. It is not a replacement for repeated, pinned-hardware performance analysis.

## v0.1.3 documentation status

For the current supported subset and explicit feature boundaries, see
[`documentation/CURRENT_CAPABILITIES.md`](../../documentation/CURRENT_CAPABILITIES.md).

Do not use historical benchmark numbers or roadmap text as current guarantees.
