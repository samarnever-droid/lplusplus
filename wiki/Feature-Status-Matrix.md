# Feature status matrix

**Last reviewed: 2026-07-30.** This page describes the current tree, not old
prototype reports.

| Feature | Status | Notes |
|---|---|---|
| Functions/variables/control flow | Stable | Functions, defaults, immutable-by-default variables, loops, break/continue |
| Structs and fields | Stable | Stack promotion, ARC heap, generated destructors |
| Recursive structs | Stable in verified targets | Arena nodes plus static cycle breaking |
| Enums/match/`?` | Working | Full-width payload layouts in the tested AOT subset |
| Generics/turbofish | Stable in tested subset | Monomorphized functions, structs, enums, methods, trait impls |
| Traits and dispatch | Working | Static and dynamic dispatch |
| Closures | Working | ARC environments, stack capsules when non-escaping, indirect calls |
| Threads | Working | Move-out optimization is conservative |
| Strings | Stable in verified paths | Immortal literals and ARC heap strings use distinct safe layouts |
| Lists/maps | Stable in tested packages | Host/direct runtime coverage is verified |
| Buffers/files/networking/JSON | Working | Runtime/platform coverage differs by target |
| Arena regions | Working | Self-referential nodes; correctness-first region lifetime implementation |
| Structural tuples | Experimental/working | Arity 2–4, structural typing, destructuring, managed child teardown; both backends/link paths |
| Typed variadics | Experimental/working | Final typed rest becomes `List[T]`; extern/native C varargs rejected |
| Borrowed slices | Experimental/working | Stack `StrSlice`/`Slice[T]`, zero-copy, bounds/generation checks, conservative non-escape validator |
| Async/await | Experimental/working | Task state/environment records, postfix `.await`, single-thread run-to-completion executor |
| Async readiness/backpressure | Planned | Blocking file/network/process/input calls are currently rejected transitively |
| General lifetime-parameterized/mutable slices | Planned | First tier is read-only and non-escaping |
| Explicit VectorI64x2 | Working | Both Cranelift and LLVM backends |
| Long SIMD checksum | Working | LLVM vector IR and Cranelift/runtime AVX2 path |
| Automatic vectorization of arbitrary loops | Planned | Not claimed; explicit vectors are available |
| Cranelift backend | Production/default | Fast compilation, full current parity corpus |
| LLVM backend | Optional/working | Full current 86-case validation clone; slower compile |
| Direct ELF linker | Production for verified Linux subset | Freestanding runtime |
| Direct PE linker | Working for verified Windows subset | Windows runner validation remains required for LLVM |
| Direct Mach-O linker | Working subset | Platform-specific boundaries apply |
| LSP/diagnostics | Working | Editor protocol and compiler diagnostics |
| Package manager | Working tested workflows | lppsqlite/compresslpp validation available |
| LLVM LTO/PGO | Planned | No current implementation |
| Arena bump/chunk allocator | Planned optimization | Current Arena prioritizes correctness and lifetime safety |

## Commands

```sh
lpp app.lpp                         # Cranelift default
lpp app.lpp --backend llvm          # optional LLVM backend
lpp app.lpp --linker direct         # direct linker
lpp app.lpp --dump-escape           # MIR ownership facts
```

## Verification snapshot

- cargo tests: 70/70;
- Cranelift AOT parity: 44/44;
- LLVM validation corpus: 86/86 in the LLVM clone;
- lppsqlite differential: 118/118;
- compresslpp: all cross-verification pass;
- targeted ASan/UBSan/TSan ownership tests: clean;
- four-feature batch: 17 positive cases on Cranelift/LLVM × host/direct,
  9 rejection contracts, 5 ASan/UBSan cases, and combined TSan: pass.

This matrix does **not** label the repository “100% Feature Freeze.” The four
new language tiers remain experimental pending broader fuzzing and a real
Windows runtime/backend execution gate.
