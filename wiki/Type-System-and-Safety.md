# Type system and safety

**Current as of 2026-07-30.**

## Mutability

Bindings are immutable by default:

```lpp
x := 1
mut y := 2
y = 3
```

Field mutation also requires a mutable binding. Parameters are caller-owned and
cannot be reassigned directly.

## Ownership model

| Value kind | Current strategy |
|---|---|
| Scalars | Copy/value |
| Non-escaping ordinary structs | Stack payload |
| Escaping single-thread values | ARC heap |
| Values reachable across a thread boundary | Atomic ARC |
| Strings | Immortal-header literals or ARC heap strings |
| Self-referential structs | Arena-backed nodes with ARC-compatible headers |
| Closure environments | ARC-managed |
| Non-escaping closure capsules | Stack-resident capsule with direct destructor |

## Escape analysis

The compiler solves escape/storage facts over MIR in
`src/mir/escape_solver.rs`. It does not use the old AST analyzer.

```text
Frame < Owned < Shared
```

The solver is conservative for direct calls, indirect calls, unknown builtins,
field stores, closure capture, lists, and thread boundaries. A missed fact costs
an optimization; it must not create a dangling pointer.

## Cycles and Arena

Recursive struct types are accepted. The static cycle breaker demotes one edge
of each type cycle to non-owning, so the owning subgraph is acyclic. A
self-referential allocation gets an Arena region. The region remains alive while
its nodes are referenced and is reclaimed after the final node dies.

```lpp
struct Node:
    value: Int
    next: Node
```

This is no longer a rejection contract. It is covered by recursive-structure
and Arena-return tests.

## Vectors

The current explicit vector API supports `VectorI64x2` construction, splat,
add/subtract/multiply/XOR, constant shift, lane extraction, and sum. It is
implemented in both Cranelift and LLVM. General automatic vectorization of
arbitrary list loops is not yet claimed.

## Generics and traits

Generics are monomorphized in the tested compiler pipeline. Traits support
static and dynamic dispatch, including generic trait implementations in the
verified corpus. Unsupported or unresolved types are rejected before backend
code generation.

## Safety boundaries

- FFI is inherently outside MIR ownership proofs and uses conservative runtime
  behavior.
- Windows LLVM object/runtime execution still needs a Windows CI runner.
- LLVM LTO/PGO is not implemented.
- Arena currently prioritizes correctness over bump-allocation performance.
- Sanitizer coverage is targeted and recorded; it is not a proof of all possible
  programs.
