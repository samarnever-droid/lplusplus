# L++ (LPlusPlus) Wiki

Welcome to the official **L++ (LPlusPlus)** documentation wiki.

L++ is a compiled, statically typed systems and application programming language designed around three pillars:
1. **Readable by design:** Clean, expressive syntax with significant indentation and inferred bindings (`:=`).
2. **Native by default:** Direct compilation to native machine code (Cranelift AOT by default, optional experimental LLVM and `wasm32-wasip1` targets) without a runtime VM.
3. **Safety engineered in:** Deterministic Automatic Reference Counting (ARC), MIR ownership placement and balance proofs (`E4403`), and Bacon-Rajan trial-deletion cycle collection on Cranelift native targets.

> For authoritative syntax, CLI, and per-backend support boundaries, see [`specs.md`](../specs.md) and [`docs/CURRENT_CAPABILITIES.md`](../docs/CURRENT_CAPABILITIES.md).

---

## Navigation

### Getting Started
| Page | Description |
|---|---|
| [Getting Started](Getting-Started.md) | Installation, CLI usage (`lpp --run`, `lpp --check`), and your first L++ program |
| [Language Tour](Language-Tour.md) | Variables, primitives, control flow, structs, enums, traits, and generics |
| [Standard Library & Builtins](Standard-Library.md) | Core I/O, string helpers, `List`/`Map`, files, math, and FFI (`extern "C"`) |

### Deep Dives & Internals
| Page | Description |
|---|---|
| [Compiler Architecture](Compiler-Architecture.md) | The modular `crates/lpp-*` pipeline from frontend AST through MIR and codegen |
| [Memory & Ownership](Memory-and-Ownership.md) | `Frame`/`Owned`/`Shared` placement, ARC headers, balance proofs, and cycle collection |
| [Type System & Safety](Type-System-and-Safety.md) | Bidirectional inference, monomorphization, pattern matching, and `Option`/`Result` |
| [Direct Linker & Platforms](Direct-Linker-and-Platforms.md) | Built-in PE/ELF/Mach-O linker (`lpp-link`), host fallback, and WASI WebAssembly |
| [Keel Package Manager](Keel-Package-Manager.md) | Dependency management, `Keel.toml` manifests, `Keel.lock`, and `llppregistry` |
| [Benchmarks & Performance](Performance-and-Benchmarks.md) | Repository benchmark suites (`benchmarks/king20`, `lto`, `pgo`, `simd`) |

---

## Quick Example

```lpp
struct User:
    name: Str
    age: Int

def main():
    user := User("Samar", 24)
    print_str("Hello, " + user.name + "! You are " + int_to_str(user.age) + " years old.")
```

Compile and execute directly:
```bash
lpp hello.lpp --run
```

---

## Official Links

- **Official Website:** [https://lplusplus.bond](https://lplusplus.bond)
- **Source Repository:** [GitHub - samarnever-droid/lplusplus](https://github.com/samarnever-droid/lplusplus)
- **Release Brief (`v0.2.0`):** [`docs/release/V0.2.0.md`](../docs/release/V0.2.0.md)
