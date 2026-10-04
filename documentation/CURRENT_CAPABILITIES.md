# Current capabilities

**Last reviewed: 2026-07-30.** This file is a compact index; the detailed
implementation status is in [STATUS-2026-07-30.md](STATUS-2026-07-30.md).

## Compiler

| Capability | Status |
|---|---|
| Python-readable indentation syntax | Available |
| Functions, defaults, loops, break/continue | Available |
| Structs, enums, match, `?` | Available |
| Generics and turbofish | Available and monomorphized |
| Traits with static/dynamic dispatch | Available |
| FFI / `extern "C"` | Available through host linking |
| Closures and thread spawn | Available |
| Fixed structural tuples | Experimental first tier: arity 2–4, managed elements, destructuring |
| Typed variadics | Experimental first tier: final owned rest `List[T]`; extern C varargs rejected |
| Borrowed slices | Experimental first tier: zero-copy `StrSlice` / `Slice[T]`, conservative non-escape rules |
| Async/await | Experimental first tier: task state records and single-thread run-to-completion executor |
| Lists, maps, strings, buffers, files, networking | Available in the supported runtime paths |
| LSP server and diagnostics | Available |
| Package tooling | Rust PM is production default; pure-L++ PM is experimental opt-in (`LPP_SELF_HOSTED_PM=1`) |
| `c2lpp` package | Phase 1 C-header/native-package generation complete; Phase 2 audited scalar translation started |

## Ownership

- MIR is the single source of escape/storage truth.
- `Frame < Owned < Shared` is the escape lattice.
- Non-escaping ordinary structs use stack payloads.
- Escaping values use ARC.
- Self-referential structs use Arena regions with ARC-compatible node headers.
- The static cycle breaker demotes one edge per ownership cycle to non-owning.
- Closures use ARC environments; non-escaping closure capsules can be stack-resident.
- Tuples and tasks are ARC aggregates with runtime child/environment teardown.
- Slice view records are stack-resident and borrowed; the MIR validator prevents
  first-tier escapes and the runtime validates bounds/generation at each access.

## Backends and linkers

| Combination | Status |
|---|---|
| Cranelift + host linker | Production/default |
| Cranelift + direct ELF/PE/Mach-O linker | Production for the verified targets |
| LLVM + host linker | Optional, tested |
| LLVM + direct linker | Optional, tested |
| Windows LLVM execution | Needs Windows CI validation |

Select the optional backend explicitly:

```sh
lpp app.lpp --backend llvm --linker direct
```

There is no Turbo mode in the current repository. Cranelift remains the fast
compile-time default; LLVM is the optional optimization backend.

## Explicit vectors

The common API supports `VectorI64x2` construction, splat, add, subtract,
multiply, XOR, constant right shift, lane extraction, and horizontal sum. The
long checksum workload also has a four-lane LLVM IR path and an AVX2 runtime
path for Cranelift.

## Verified gates

- Cargo tests: 71/71.
- Cranelift AOT parity: 44/44.
- LLVM corpus in the LLVM validation clone: 86/86.
- lppsqlite differential: 118/118.
- compresslpp cross-verification: all pass.
- Safety mission: pass.
- Sanitizer coverage: closures, strings, owned fields, recursive structures,
  Arena return, vectors, and the tuple/rest/slice/task batch are clean in the
  recorded runs.
- Four-feature gate: 17 positive cases on both backends and both link paths,
  9 compile-time rejection contracts, 5 ASan/UBSan cases, and TSan: pass.
- Canonical list-policy gate: eight positive scalar/managed cases on both backends
  and both linkers, three rejection contracts, and six ASan/UBSan cases: pass.

## Not yet claimed

- Project-wide “100% feature freeze”; these four additions remain experimental.
- Async I/O readiness, backpressure, multi-thread work stealing, or C100K claims.
- General lifetime-parameterized slices, mutable slices, or slice thread send.
- Unsafe native C variadic ABI or tuples outside arity 2–4.
- General source-level automatic vectorization of arbitrary loops.
- Windows LLVM runtime execution.
- LLVM LTO/PGO.
- Arena bump/chunk allocation performance optimization.
