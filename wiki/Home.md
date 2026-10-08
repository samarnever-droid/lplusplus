# L++ Documentation Wiki

Welcome to the official **L++ (LPlusPlus)** documentation wiki.

L++ is a modern, statically typed, ahead-of-time (AOT) compiled programming language engineered around three core tenets:
1. **Readable by design:** Clean, expressive syntax with significant indentation, eliminating unnecessary visual noise.
2. **Native by default:** Direct compilation to standalone native machine code (x86_64 PE on Windows, ELF on Linux, Mach-O on macOS) and standalone WebAssembly (`wasm32-wasip1`) without requiring a VM, interpreter, or runtime tracing garbage collector.
3. **Safety engineered in:** Deterministic Automatic Reference Counting (ARC) with static escape analysis, borrow verification, and compile-time ownership validation.

---

## Wiki Directory

| Guide | Description |
|---|---|
| [Getting Started](Getting-Started.md) | Installation, CLI usage (`lpp build`, `lpp run`, `lpp check`, `lpp doctor`), and your first program |
| [Language Tour](Language-Tour.md) | Comprehensive syntax overview: variables, types, structs, enums, pattern matching, closures, and control flow |
| [Type System & Safety](Type-System-and-Safety.md) | Static type safety, Hindley-Milner bidirectional inference, generics, traits, and error handling |
| [Memory & Ownership](Memory-and-Ownership.md) | ARC memory model, `Frame`/`Owned`/`Shared` classifications, borrow validation, and zero-pause reclamation |
| [Compiler Architecture](Compiler-Architecture.md) | Detailed walkthrough of the modular compiler pipeline: Frontend, HIR, Types, MIR, Passes, Ownership, Codegen, and Linker |
| [Direct Linker & Platforms](Direct-Linker-and-Platforms.md) | Direct in-process PE/COFF and ELF linkers, cross-compilation, and platform target specifics |
| [Standard Library & Builtins](Standard-Library.md) | Builtin functions, string manipulations, collections (`List`, `Map`), I/O, and C FFI integration |
| [Keel Package Manager](Keel-Package-Manager.md) | Dependency management, `lpp.toml` manifests, package publishing, and workspace tooling |
| [Performance & Benchmarks](Performance-and-Benchmarks.md) | Compiler compilation throughput (~50,000+ lines/sec), runtime benchmarks (King20 suite vs C++, Rust, Go, Python) |

---

## Quick Example

```lpp
struct User:
    name: Str
    age: Int

def greet(user: User):
    println("Hello, " + user.name + "! You are " + str(user.age) + " years old.")

def main():
    alice := User("Alice", 25)
    greet(alice)
```

Compile and run with zero external toolchain configuration:

```bash
lpp run hello.lpp
```

---

## Community & Resources

- **Official Website:** [https://lplusplus.bond](https://lplusplus.bond)
- **Source Repository:** [GitHub - samarnever-droid/lplusplus](https://github.com/samarnever-droid/lplusplus)
- **Package Registry:** [https://registry.lplusplus.bond](https://registry.lplusplus.bond)
