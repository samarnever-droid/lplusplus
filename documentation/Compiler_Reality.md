# Compiler reality — 2026-07-30

This document replaces the old v0.1 notes. The current compiler is a native
Rust/MIR compiler; there is no C transpilation stage in the production pipeline.

## Pipeline

```text
lexer/parser
  -> semantic resolver
  -> type checker
  -> monomorphization
  -> MIR lowering
  -> scalar MIR passes
  -> static cycle breaking
  -> MIR escape solver
  -> stack/ARC/Arena cleanup
  -> Cranelift or optional LLVM object backend
  -> host linker or lpp-link
```

The relevant source files are:

- `src/frontend/lexer.rs`, `parser.rs`, `ast.rs`;
- `src/analysis/semantic.rs`, `typecheck.rs`, `cyclebreak.rs`, `monomorph.rs`;
- `src/analysis/types.rs`, `type_facts.rs`, and `layout.rs` define the shared
  resolved type model, ownership/ABI facts, and backend-neutral native layout;
- `src/mir/lower.rs`, `ir.rs`, `escape_solver.rs`, and `pass_*.rs`;
- `src/backend/cranelift/`;
- `src/backend/llvm.rs`;
- `src/bin/lpp-link.rs`.

## Shared type and layout contract

The resolved type model no longer lives inside the type-checker implementation.
`analysis/types.rs` is consumed by semantic analysis, MIR, cycle breaking, and
both backends. `analysis/type_facts.rs` is the canonical classification for
managed values, borrowed views, ABI categories, nested tasks, and list element
policies. `analysis/layout.rs` computes struct and tuple offsets once.

LLVM no longer imports layout helpers from the Cranelift backend. Cranelift and
LLVM independently map the same backend-neutral `AbiClass` and `FieldLayout` to
their own IR types. Adding a new `TypeRef` therefore produces exhaustive-match
compiler errors in the shared fact layer rather than requiring several partial
type lists.

## MIR escape solver

`src/mir/escape_solver.rs` computes one whole-program fact over MIR. The
lattice is:

```text
Frame < Owned < Shared
```

MIR matching is exhaustive over the value forms. Indirect calls and unknown
builtins remain conservative local sinks. The old `src/analysis/escape.rs` AST
analyzer is retired and no longer exists in the current tree.

## Ownership lowering

- `Ownership::Managed` identifies pointer-shaped values during lowering.
- The solver determines whether their storage can be Frame, Owned, or Shared.
- `pass_escape` changes provably frame-local struct/closure payloads to stack
  allocations.
- `pass_arc` inserts deterministic retain/release or direct stack destructors.
- `pass_moveout` removes balanced thread handoff reference traffic when the
  source is proven dead after the spawn.

## Arena regions

Self-referential structs receive a lazily created region. Arena nodes have
ARC-compatible headers and are registered with the region. The region survives
as long as nodes reference it and is reclaimed after the last node dies. The
cycle breaker ensures that one edge of every type cycle is non-owning, so the
owning subgraph is acyclic.

This is a correctness-first Arena implementation. It is not yet a bump/chunk
allocator benchmark claim.

## Tuples, rests, borrowed views, and tasks

These features are represented explicitly rather than hidden in scalar handles:

- `TypeRef::Tuple`, `AllocateTuple`, and `TupleField` use a shared native layout
  function. A tuple payload stores a managed-element mask and packed offsets so
  the runtime releases exactly the owned child fields.
- A typed variadic call allocates a normal `List[T]`, pushes each extra argument
  with the existing element ownership rules, and passes that rest handle as the
  final MIR argument. No C `...` call is emitted.
- `StrSlice` / `Slice[T]` use `MakeSlice`, `SliceLen`, `SliceGet`, and
  `SliceToStr`. The view itself is a 40-byte stack record and allocates no heap
  payload. `validate_borrows` enforces the first-tier escape contract.
- `Task[T]`, `MakeTask`, and `Await` create an ARC task plus an ARC tuple-shaped
  environment. Each backend emits an `(env) -> i64` task thunk. The runtime task
  state machine is pending/running/complete and executes on the caller thread.

The host, Linux freestanding, and Windows freestanding sources export tuple,
slice, and task symbols. Linux behavior is measured on host/direct paths;
Windows source presence is not a substitute for a Windows execution run.

## Cranelift backend

Cranelift is the default because its compile latency is low and it integrates
well with the existing custom object/linker path. It supports the full verified
MIR corpus, ARC, closures, lists, maps, and Arena nodes.

The explicit `VectorI64x2` API lowers to Cranelift SIMD values when the target
supports them. The long checksum intrinsic has an AVX2 runtime fast path and a
scalar fallback.

## LLVM backend

`src/backend/llvm.rs` is optional and selected with:

```sh
lpp file.lpp --backend llvm
```

It emits textual LLVM IR and invokes `clang` to create an object. It supports
the current tested aggregate/ARC/closure/list/map/Arena paths and uses the same
host or direct linker. LLVM is slower to compile but can produce stronger
optimized/vectorized code for large numeric loops.

LLVM does not silently replace Cranelift. If it cannot lower a MIR form, it
returns a diagnostic.

## Linkers and runtimes

- Host linking uses `cc`/MSVC and the host runtime.
- `lpp-link` emits direct ELF, PE/COFF, or Mach-O according to target support.
- Linux freestanding and Windows freestanding runtime sources contain ARC,
  closure, list, string, and Arena symbols.
- Windows LLVM execution still needs a real Windows CI runner; Linux host/direct
  behavior is the current measured environment.

## Verification

The current recorded gates are:

- cargo tests: 70/70;
- Cranelift parity: 44/44;
- LLVM corpus: 86/86 in the validation clone;
- lppsqlite: 118/118 differential;
- compresslpp: all cross-verification pass;
- ASan/UBSan and TSan targeted ownership/vector tests: clean;
- four-feature parity: 17 positive programs on Cranelift/LLVM × host/direct,
  9 rejection contracts, 5 ASan/UBSan runs, and combined TSan: pass.

The four-feature tier is experimental. In particular, async functions are
run-to-completion tasks rather than general resumable coroutines, and borrowed
slices use a restrictive compile-time non-escape policy rather than general
lifetime parameters.

Old reports that mention C backend stubs, `EscapeAnalyzer`, rejected recursive
structs, or unimplemented closure lifting are historical and should not be read
as current status.
