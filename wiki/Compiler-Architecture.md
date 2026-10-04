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
- `src/analysis/typecheck.rs` checks compatibility and inference.
- `src/analysis/types.rs` owns the resolved type model and type table.
- `src/analysis/type_facts.rs` owns canonical lifetime, ABI, task-containment,
  and container-element classifications.
- `src/analysis/layout.rs` owns backend-neutral struct and tuple layout.
- `src/analysis/monomorph.rs` specializes generic functions, structs, enums,
  methods, and trait implementations.
- `src/analysis/cyclebreak.rs` classifies one edge of each ownership cycle as
  non-owning.
- Tuple types/expressions, destructuring, typed rest parameters, borrowed slice
  types, `async def`, and postfix `.await` are all first-class AST/type forms;
  they are not parser-only desugarings.

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

The new aggregate/borrow/task layer is explicit in MIR:

```text
AllocateTuple / TupleField
AllocateList + typed rest pushes
MakeSlice / SliceLen / SliceGet / SliceToStr
MakeTask / Await
```

`validate_borrows` runs immediately after lowering and rejects first-tier slice
escapes before scalar optimization or ownership insertion. Task environments
reuse tuple layout metadata, while each backend emits a typed task thunk.

## Shared ABI boundary

Backends do not own language layout policy. The analysis layer produces an
`AbiClass` and aligned `FieldLayout`; Cranelift maps that to Cranelift types and
LLVM maps it to LLVM textual types. LLVM has no dependency on the Cranelift
module. Ownership-sensitive passes consume `TypeRef::lifetime_class()` rather
than maintaining private lists of managed types.

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

## Runtime state and views

- Tuples are ARC payloads with a managed-child mask and field-offset metadata.
- Rest arguments are ordinary typed ARC lists.
- Slice views are stack records: base, start, length, generation, and kind.
- Tasks are ARC records with code, environment, result, ownership flag, and
  pending/running/complete state.
- The executor polls on the caller thread with deterministic run-to-completion
  policy; no hidden thread is created.

The full host runtime and Linux/Windows freestanding sources expose matching
symbols. Actual Windows execution remains a CI requirement.

## Link stage

`src/bin/lpp-link.rs` supports the direct native object path for the verified
ELF/PE/Mach-O targets. Most language features are implemented in the backend or
runtime; the linker resolves objects and platform runtime symbols.

## Current non-goals

- No Turbo mode is in the current repository.
- No LLVM LTO/PGO integration.
- No measured Arena bump/chunk allocator yet.
- Windows LLVM execution still needs Windows CI validation.
