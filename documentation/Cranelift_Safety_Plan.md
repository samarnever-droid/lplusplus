# Cranelift AOT Safety Contract and Roadmap

L++ aims for Python-like syntax, Go-like iteration speed, and Rust-inspired memory safety. That is a long-term contract, not a claim the current prototype can make for every feature yet.

## Current AOT contract

The Cranelift backend is **experimental**. Its supported native subset is:

- `Int`, `Float`, `Bool`, and simple comparisons
- functions, branches, and loops
- custom structs with `Int`, `Float`, `Bool`, or pointer-like fields
- `List[Int]` only
- basic direct calls and supported runtime builtins

The compiler now rejects AOT float modulo and non-`Int` lists rather than generating incorrect native code. This is deliberate: rejecting an unsupported program is safer than silently compiling wrong behavior.

## Ownership invariants

1. A value passed to `lpp_arc_retain` or `lpp_arc_release` must have been allocated by `lpp_arc_alloc`.
2. Cranelift custom-struct allocation uses `lpp_arc_alloc`, so ARC-managed struct values have a valid runtime header.
3. Scalars (`Int`, `Float`, `Bool`) are values, never ARC pointers.
4. Strings and lists do **not** currently have an ARC object representation. Escape analysis must not emit ARC operations for them.
5. AOT struct allocation, field store, and field load use the same native layout function. Field offsets must never be inferred from `field_index * 8`.
6. Any feature without a verified AOT lowering must return a compiler error, not a placeholder value.

## Features not safe to promise yet

- automatic reclamation of cyclic object graphs
- generic lists (`List[String]`, `List[Struct]`, `List[Float]`, etc.)
- capture-by-reference semantics for mutable scalar closure captures
- complete alias analysis (Rule 6)
- backend parity with the C backend
- arbitrary thread-safe sharing

## Completed portability milestone: PIC AOT objects

Cranelift AOT now enables `is_pic=true`. On Linux, the generated object links
into the platform-default PIE executable without a `-no-pie` workaround or
text-relocation warning. The parity harness links AOT objects using the normal
host C compiler command.

## Required next milestones

### Milestone A — Verify the AOT core

Add a CI matrix that runs every supported program through:

1. frontend/typecheck only;
2. C emission + host C compiler;
3. Cranelift object emission + `lpp_runtime.c` link;
4. executable stdout/exit-code comparison.

Include regression cases for mixed `Bool`/`Float` struct fields, comparison values, escaped structs, closures, and unsupported-feature diagnostics.

A Unix harness now exists at `tests/run_aot_parity.sh`. It compares stdout from the C and Cranelift paths for the supported subset and verifies that `List[Float]` is rejected by AOT. Run it with:

```sh
./tests/run_aot_parity.sh
```

### Milestone B — Make ownership explicit in MIR

**Foundation implemented:** MIR locals now record `Copy`, `Owned`, or `Borrowed` ownership. Owned identifier reads lower to `Operand::Borrowed`, making a non-transferring read explicit. Custom-struct construction lowers to `AllocateArcStruct`; assignment of an owned temporary lowers to `Move`; and `ReturnOwned` records an explicit transfer of the returned local's ARC reference. ARC retains only when a borrow becomes another ARC-managed owner, and when a borrowed custom object is stored in a struct field. Fresh allocations and owned call results are not blindly retained. The ARC pass does not release a `ReturnOwned` value in the callee.

**Control-flow cleanup implemented:** ARC now runs a definite-live dataflow analysis across MIR basic blocks. At branch joins it intersects predecessor live-owner sets, so it releases only values known to be initialized on every path. Reassignment releases the previous owned reference first; `Move` removes its source from the live set; and `ReturnOwned` transfers rather than releases its owner. Regression programs cover ordinary and branch-specific owned returns.

**Recursive struct destruction implemented:** every AOT custom struct receives a generated local destructor. `AllocateArcStruct` registers its destructor in the ARC header; when the final release occurs, the runtime invokes it before freeing the payload. The destructor releases each custom-struct field, recursively activating child destructors only when their reference counts reach zero.

**Rule 6 baseline implemented:** direct aliases of custom objects (`b := a` and `b = a`) now promote both bindings to ARC ownership. Ownership MIR lowers the right-hand read as `borrow(a)`, retains it for the second owner, and releases both owners at scope exit.

**Ownership-capsule closures implemented:** closures are now ARC-managed capsules containing a code pointer and an owned environment pointer. The capsule destructor releases the environment; the generated environment destructor releases captured custom fields. Captured custom-field reads are explicitly borrowed, so a closure invocation never consumes the environment's ownership edge.

**Borrowed-return contract implemented:** custom structs and closure capsules are passed to ordinary functions as borrowed parameters. Returning such a borrowed value automatically retains it and emits `ReturnOwned`, so the caller always receives a valid owned reference. Fresh local allocations and call-result temporaries transfer ownership with `Move` instead of retaining again.

**ARC-managed lists implemented:** `List[Int]` and `List[Custom]` are ARC objects. `List[Custom]` retains each pushed element, drops each element through the list destructor, and returns borrowed elements from `list_get`. AOT automatically releases owned list locals at scope exit; manual `list_free` is rejected in AOT to prevent double release. Lists can be moved, borrowed for operations, returned as owned values, and stored in ARC-owning struct fields.

**Field alias baseline implemented:** reading a custom/list field produces a borrowed MIR value. Assigning that borrowed field into another owner retains it exactly once, while field reads inside closures remain non-owning. This closes the direct `alias := parent.child` class of dangling/double-release errors.

**Cycle-safety gate implemented:** AOT builds now reject allocation of direct or indirect cyclic owned struct graphs, including `Struct -> List[Struct]` edges. ARC cannot reclaim those graphs, so the compiler emits a diagnostic rather than silently leaking. A future `Weak`/arena annotation or cycle collector can re-enable them intentionally.

**Still required:** generic element support beyond `Int`/custom ARC objects, alias analysis through arbitrary call patterns, and matching ownership cleanup in the C transpiler backend. Those are essential before claiming leak-free ARC.

### Milestone C — Closure safety

Represent every closure as an actual pair:

```text
{ code_pointer, environment_pointer }
```

The environment must have explicit ownership and each capture must be marked as copied, moved, shared, or rejected. Remove any global closure environment state in the C backend.

### Milestone D — Containers

Either keep `List[Int]` as the first stable container, or design a typed/boxed element representation before accepting generic list syntax. Containers must define bounds behavior, element ownership, and destruction.

### Milestone E — Diagnostics and editor support

Carry source spans from lexer tokens through AST/MIR errors. Once diagnostics have stable file/line/column information, the VS Code extension can show problems directly in the editor and provide go-to-definition/hover through an LSP server.

## Release gate

Cranelift should become the default backend only after every supported feature has parity tests, ARC runtime instrumentation tests, and no known code path lowers an unsupported value as a placeholder or raw integer pointer.

## Safety-first rejection policy

The compiler must reject a feature when its current lowering would change the
program's meaning or violate ownership assumptions. Current examples:

- AOT `spawn` is rejected. Earlier lowering treated it as a normal closure and
  did not perform a safe thread handoff.
- AOT closures reject mutable captures. A copied environment cannot honestly
  implement mutation of an outer binding; this requires a future shared-cell or
  move-only capture model.
- AOT rejects lists other than `List[Int]` because the runtime uses an
  `int64_t` element buffer.
