# L++ Wiki

**Current status: 2026-07-30.** Start with
[Current Capabilities](../documentation/CURRENT_CAPABILITIES.md) and the
[full status report](../documentation/STATUS-2026-07-30.md).

L++ is a native ownership-aware language. The Rust compiler frontend lowers to
MIR, solves ownership over MIR, and emits native objects through Cranelift by
default or LLVM optionally. Objects are linked by the host linker or `lpp-link`.

## Start here

1. [Getting Started](Getting-Started.md)
2. [Language Reference](Language-Reference.md)
3. [Errors and Result](Errors-and-Result.md)
4. [Modules and Packages](Modules-and-Packages.md)
5. [Standard Library and Builtins](Standard-Library-and-Builtins.md)
6. [Compiler Architecture](Compiler-Architecture.md)
7. [Type System and Safety](Type-System-and-Safety.md)
8. [Direct Linker and Runtime](Direct-Linker-and-Runtime.md)
9. [Feature Status Matrix](Feature-Status-Matrix.md)
10. [Runtime Compatibility Matrix](Runtime-Compatibility-Matrix.md)
11. [Known Historical and Negative Files](Known-Stale-and-Negative-Files.md)

## Current highlights

- immutable-by-default variables and `mut`;
- structs, enums, match, generics, traits, closures, and threads;
- MIR ownership facts: `Frame < Owned < Shared`;
- stack payloads, ARC, Arena regions, cycle breaking, and generated destructors;
- Cranelift default backend;
- optional LLVM backend with host/direct linker support;
- explicit `VectorI64x2` operations and long SIMD workload;
- Linux ELF, Windows PE, and macOS Mach-O linker paths for their verified
  subsets.

## Accuracy policy

Do not use old reports as current implementation evidence. A current feature claim
must point to a test or validation command. The primary commands are:

```sh
cargo test --release -j1
sh tests/run_aot_parity.sh
sh scripts/check_safety_mission.sh
```

Package validation runs from the package directories. Windows LLVM execution
still needs a Windows CI runner, and general automatic vectorization/LTO/PGO are
not current claims.
