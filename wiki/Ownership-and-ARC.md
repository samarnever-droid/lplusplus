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

## Safety mission

The project tracks evidence levels and does not claim full full language-wide safety guarantee today. See [`documentation/Safety_Mission.md`](../documentation/Safety_Mission.md).

## v0.1.3 current-status note

This page is maintained with the project, but current support claims are
centralized in [Current Capabilities](../documentation/CURRENT_CAPABILITIES.md).

```text
Use LppData/build/release and LppData/cache for package artifacts.
Use host-linked AOT for filesystem/networking work.
Do not assume direct ELF supports files, networking, JSON, or threads.
Do not claim language-wide Rust-equivalent safety outside the verified AOT subset.
```
