# Ownership and ARC

L++ AOT lowering tracks three ownership modes:

```text
Copy      — plain value copied freely
Owned     — ARC-managed value whose lifetime is transferred or retained
Borrowed  — non-owning alias that must not outlive its owner
```

The compiler represents allocation, moves, borrows, retains, releases, and owned returns in MIR. It inserts cleanup through control-flow paths rather than relying on a C backend convention.

## Supported ownership work

- ARC struct allocation and generated destructor chains
- owned and borrowed return paths
- direct aliases and field aliases
- ARC closures/capture capsules
- `List[Int]` and `List[Custom]`
- recursive destruction of owned children
- definite-live cleanup across branches

## Cycles

Strong ownership cycles are rejected in AOT. ARC cannot reclaim a cycle safely. Restructure the ownership graph until L++ provides an explicitly designed weak-reference or arena facility.

## Rule for contributors

Never add a type/backend shortcut that silently bypasses ownership cleanup. Reject unsupported semantics with a diagnostic until the MIR, ARC pass, runtime ABI, and tests support them together.
