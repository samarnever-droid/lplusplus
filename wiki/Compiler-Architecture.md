# Compiler architecture

**Current as of 2026-07-30.** For verified status and known boundaries, see
[Current Capabilities](../documentation/CURRENT_CAPABILITIES.md) and
[Compiler Reality](../documentation/Compiler_Reality.md).

## Pipeline

```text
.lpp source
  -> lexer/parser
  -> semantic resolver
  -> type checker
  -> monomorphization
  -> MIR lowering
  -> MIR scalar passes
  -> cycle breaker
  -> MIR escape solver
  -> stack/ARC/Arena cleanup
  -> Cranelift (default) or LLVM (optional)
  -> host linker or lpp-link
  -> executable
```

## Frontend

- `src/frontend/lexer.rs` handles indentation, literals, keywords, comments,
  and operators.
- `src/frontend/parser.rs` builds the AST.
- `src/analysis/semantic.rs` assigns binding IDs and checks scopes/mutability.
- `src/analysis/typecheck.rs` checks types and struct/enum layouts.
- `src/analysis/monomorph.rs` specializes generic functions, structs, enums,
  methods, and trait implementations.
- `src/analysis/cyclebreak.rs` classifies one edge of each ownership cycle as
  non-owning.

## MIR and ownership

MIR is the ownership boundary. The old AST escape analyzer was removed.
`src/mir/escape_solver.rs` computes the single reachability fact:

```text
Frame < Owned < Shared
```

`pass_escape` performs stack promotion for frame-local structs and closure
capsules. `pass_arc` inserts cleanup. Stack payload cleanup calls generated
destructors directly; ARC payload cleanup calls the runtime. `pass_moveout`
removes balanced handoff retains/releases only after a liveness proof.

Arena regions are selected for self-referential struct allocations. Arena nodes
retain ARC-compatible headers and a region handle; cycle breaking ensures that
owning edges remain acyclic.

## Backends

### Cranelift

`src/backend/cranelift/` is the default production backend. It lowers MIR to
Cranelift IR and emits native objects. It has the lowest compile latency and
supports the full verified language/runtime subset.

### LLVM

`src/backend/llvm.rs` is an explicit optional backend:

```sh
lpp program.lpp --backend llvm --linker direct
```

It emits textual LLVM IR and invokes `clang`. It supports the current corpus,
including aggregate ownership, closures, lists/maps, Arena nodes, and explicit
vectors. Unsupported future MIR forms must produce an error rather than a
fallback or placeholder.

## Explicit vector layer

Both backends support `VectorI64x2` builtins for construction, splat, arithmetic,
XOR, constant shift, lane extraction, and sum. LLVM also has a four-lane
checksum IR path. The repository does not claim automatic vectorization of every
arbitrary list loop.

## Link stage

`src/bin/lpp-link.rs` supports the direct native object path for the verified
ELF/PE/Mach-O targets. Most language features are implemented in the backend or
runtime; the linker resolves objects and platform runtime symbols.

## Current non-goals

- No Turbo mode is in the current repository.
- No LLVM LTO/PGO integration.
- No measured Arena bump/chunk allocator yet.
- Windows LLVM execution still needs Windows CI validation.
