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
| Structural tuples | ARC aggregate; managed children released by metadata-driven destructor |
| Typed variadic rest | Normal `List[T]` object assembled at the call site |
| `StrSlice` / `Slice[T]` | Borrowed stack view, no view heap allocation/destructor |
| `Task[T]` | ARC task + ARC environment/result ownership |

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

## Structural tuples and typed rests

Tuple compatibility is structural and element-by-element. Arity is restricted
to 2–4. Construction transfers owned temporaries or retains borrowed managed
elements; destructuring reads each element as a borrow and uses the normal
assignment ownership operation to create any new owner.

A variadic declaration such as `def log(level: Str, ...items: Str)` has a fixed
prefix plus one typed rest element type. Calls allocate `List[Str]`, push extras
with list ownership rules, and pass the list handle. No unsafe native varargs ABI
is inferred, and extern declarations reject `...`.

## Borrowed slice boundary

A view records its source, range, generation, and source kind. Construction and
reads check bounds. It owns no source buffer and has no destructor. The current
borrow validator rejects:

- return from the creating function;
- closure capture or owning aggregate/container storage;
- thread handoff;
- unknown/retaining calls;
- source reassignment while the view is live.

A view may be consumed by explicit slice operations or a statically known
function whose corresponding parameter is slice-typed and whose body passes the
same validator. `str_slice_to_str` is the explicit owned escape.

## Async task boundary

An async call captures arguments in an owned environment and returns `Task[T]`.
`.await` is restricted to async functions (with async `main` entered by the
executor). Managed results are retained for each await, so double-await is
defined; task destruction releases its environment and held result exactly
once. Polling a completed task is idempotent. Task capture by closures is
rejected in this tier so a task cannot leave its executor boundary.

The first executor has one caller thread and run-to-completion policy. A
transitive call-graph check rejects blocking operations without adapters. This
is not yet general coroutine suspension, nonblocking socket readiness,
backpressure, or work stealing.

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

- The tuple/rest/slice/task tier is experimental and is not a project-wide
  feature-freeze claim.
- Windows source-level runtime coverage exists, but execution still requires a
  real Windows gate.
- FFI is inherently outside MIR ownership proofs and uses conservative runtime
  behavior.
- Windows LLVM object/runtime execution still needs a Windows CI runner.
- LLVM LTO/PGO is not implemented.
- Arena currently prioritizes correctness over bump-allocation performance.
- Sanitizer coverage is targeted and recorded; it is not a proof of all possible
  programs.
