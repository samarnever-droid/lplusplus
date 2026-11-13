# Compiler Architecture

The L++ compiler (`RewriteEngine`, the sole active engine in [`src/main.rs`](../src/main.rs)) is organized as a modular multi-crate Rust workspace under `crates/`.

> For full crate diagrams and phase details, see [`ARCHITECTURE.md`](../ARCHITECTURE.md) and [`specs.md`](../specs.md) §8.

---

## High-Level Pipeline

```text
Source (.lpp)
  -> lpp-frontend   (Lexer, Indentation/Escape Validator, Parser -> AST)
  -> lpp-hir        (Module Graph, Import/Package Staging, HIR)
  -> lpp-types      (Type Inference, Traits, Monomorphization)
  -> lpp-mir        (MIR Construction, Verifier, Interpreter / ARC Oracle)
  -> lpp-passes     (Scalar Opts, `--lto` / `--pgo-use` Bounded Inliner)
  -> lpp-ownership  (Frame/Owned/Shared Classification, SCC Cycles, E4403 Balance Proof)
  -> lpp-codegen-api
       ├─► lpp-codegen-cranelift  (Default Native AOT: x86_64, aarch64)
       ├─► lpp-codegen-wasm       (Experimental WASI: wasm32-wasip1)
       └─► lpp-codegen-llvm       (Experimental Clang-backed LLVM x86_64)
  -> lpp-linker (`lpp-link`) or host C toolchain fallback -> Native Executable / `.wasm`
```

---

## Verification Boundaries

- **Parser -> HIR -> Type Check**: Rejects syntax, missing modules, and type/trait mismatches before MIR construction.
- **MIR Verifier (`CORE_MIR_INVARIANTS`) & Definite Init**: Validates control-flow graphs, locals, and initializations before optimization passes run.
- **Ownership Balance (`E4403`) & ARC Leak Oracle (`E4304`)**: Verifies balanced retain/release paths and validates native reclamation (including Bacon-Rajan cycle collection) across tests.

---

## Runtime & Package Layer

- **Hosted Runtime (`crates/lpp-runtime`)**: Implements atomic ARC, the native trial-deletion cycle collector, strings, lists, maps, I/O, files, JSON, and `vec_i64_checksum` AVX2 auto-dispatch.
- **Freestanding Runtime (`runtime/`)**: Minimal syscall/Kernel32 C sources for direct-linked freestanding subsets.
- **Keel (`crates/keel` & `crates/lpp-pm`)**: Embedded project, workspace, lockfile (`Keel.lock`), and Git registry (`llppregistry`) engine invoked when running package commands without a `.lpp` source file.
