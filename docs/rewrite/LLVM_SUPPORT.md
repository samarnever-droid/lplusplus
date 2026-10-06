# LLVM backend support policy for L++ v0.1

## Release status

The LLVM backend is **experimental and opt-in** in L++ v0.1. It is not the
default native backend and is not part of the v0.1 stable compatibility
promise. The supported native release path is Cranelift.

Select LLVM explicitly with either:

```sh
lpp app.lpp --backend llvm
lpp config set backend llvm
```

Configure its external compiler with `lpp setup llvm [path]`, persisted
`llvm-path`, or the per-process `LPP_LLVM_CC` override.

## Implemented LLVM surface

The rewrite LLVM emitter currently covers:

- scalar integer, boolean, character, and floating-point locals;
- strings and the supported string/printing runtime calls;
- direct calls and returns;
- branches, loops, comparisons, and integer truthiness;
- implicit `main` return as C ABI `i32 0`;
- bare non-capturing function values, aliases, and typed indirect scalar calls;
- deterministic textual LLVM IR and external object assembly.

## Explicitly unsupported in the v0.1 LLVM tier

Programs requiring the following receive a typed unsupported-construct error
rather than silently falling back or emitting partial IR:

- lists, maps, tuples, slices, structs, and enum payloads;
- capturing closures and their environments;
- async tasks, `await`, and `spawn`;
- managed aggregate ownership operations;
- SIMD/vector values;
- indirect callable capsules stored inside managed containers.

These features remain available through the stable Cranelift backend and, where
defined by its capability boundary, the direct WASM backend.

## Gates

No-compiler textual gates cover plan/emitter agreement, string builtins,
integer truthiness, implicit entry returns, typed unsupported rejection, and
bare function values. Object/execution gates run when an LLVM-compatible
compiler is available and otherwise report an explicit environment skip. The
Linux CI and release preflight require `clang --version` before running the
full workspace suite, so a missing release-gate compiler cannot turn into a
silent pass.

LLVM can enter the stable compatibility promise only after the managed-data
surface has executable parity coverage and the release matrix explicitly gates
it on every advertised LLVM host.
