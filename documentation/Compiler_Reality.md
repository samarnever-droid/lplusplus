# L++ Compiler: Reality & Implementation Status

This document describes the exact implementation details, pipeline architecture, and current limitations of the L++ compiler as of version 0.1.0.

Current classification:

- Stable enough for local experimentation: frontend pipeline, semantic/type passes, MIR lowering, C backend, cross-platform CLI flow
- Experimental: Cranelift AOT coverage outside the core subset, package manager ergonomics, runtime ownership details
- Stubbed or partial: full closure environment lowering, backend parity for all list behaviors, cycle collection

---

## 1. Compiler Pipeline Architecture

The L++ compiler is written in Rust and operates via a standard multi-stage ahead-of-time (AOT) pipeline:

```
[L++ Source File]
       │
       ▼ (lexer::Lexer)
   [Tokens]
       │
       ▼ (parser::Parser)
  [AST (Abstract Syntax Tree)]
       │
       ▼ (semantic::Resolver)
[Scope-Resolved AST & Symbol Table]
       │
       ▼ (typecheck::TypeChecker)
  [Typechecked AST & Type Table]
       │
       ▼ (escape::EscapeAnalyzer)
[Storage Classification Map (Value/Arc)]
       │
       ▼ (mir::lower::MirLowerCtx)
  [Mid-level IR (MIR) in CFG form]
       │
       ▼ (mir::pass_arc)
[ARC Reference Counting Insertions]
       │
 ┌─────┴────────────────────────┐
 ▼ (codegen::Codegen)          ▼ (cranelift_backend::compiler::AotCompiler)
[Transpiled C Code]           [Native Object File (.o)]
 │                             │
 ▼ (cl.exe / GCC / Clang)      ▼ (link.exe / MSVC Linker)
[Executable (.exe)]           [Native Executable (.exe)]
```

---

## 2. Compiler Stages in Detail

### 2.1 Lexer (`src/frontend/lexer.rs`)
*   **What it does:** Tokenizes L++ source code into a stream of tokens.
*   **Indentation Handling:** Converts spaces/tabs at the beginning of lines into `Token::Indent` and `Token::Dedent` markers to represent lexical blocks (similar to Python).
*   **Reality:** Solid for the current language subset. Handles nested scopes, newlines, comments (`#`), string literals, and now reports out-of-range integer literals as lexer errors instead of crashing.

### 2.2 Parser (`src/frontend/parser.rs`)
*   **What it does:** Parsers the token stream into an Abstract Syntax Tree (AST) structure defined in `src/frontend/ast.rs`.
*   **Reality:** Parses top-level functions, structs, variables, function calls, arithmetic/boolean expressions, `if/else` conditionals, and `while` loops. Skip-newline logic is implemented before checking block indentations to handle blank lines between statements correctly.

### 2.3 Semantic Analysis (`src/analysis/semantic.rs`)
*   **What it does:** Performs identifier binding resolution, scope checking, and name binding lookup. Generates a scope tree and registers global user functions and structs.
*   **Built-in Registry:** Built-ins like `print`, `print_str`, `input`, `read_file`, `write_file`, and `parse_int` are registered as known identifiers that do not require user-defined declarations.

### 2.4 Typechecker (`src/analysis/typecheck.rs`)
*   **What it does:** Performs static type-inference and type safety checks on the scope-resolved AST.
*   **Types supported:** `Int` (64-bit signed), `Str` (immutable string pointer), `Bool` (boolean), `Void` (no value), `Custom(StructId)` (struct type), and `Generic("List", [T])` (templated list type).
*   **Reality:** Statically checks parameter matching, loop/conditional conditional types, assignment compatibility, and struct field layout types. Assigning incompatible types or passing incorrect arguments triggers typecheck compilation errors.

### 2.5 Escape Analysis (`src/analysis/escape.rs`)
*   **What it does:** Determines whether variables can be allocated on the execution stack (`StorageClass::Value`) or require heap allocation with reference counting (`StorageClass::Arc`).
*   **Reality:** Fully implemented. Walks the AST and constructs safety bounds. Currently defaults custom structs and structures to stack/heap based on escapes, preparing references for automatic memory management.

### 2.6 Mid-level IR (MIR) (`src/mir/`)
*   **What it does:** Lowers the high-level AST into a flat control-flow graph (CFG) representation comprising Basic Blocks (`bb0`, `bb1`, etc.) containing 3-address instructions.
*   **ARC Pass (`src/mir/pass_arc.rs`):** Inspects the MIR instructions and automatically inserts increment (`Retain`) and decrement (`Release`) instructions for reference-counted variables.
*   **Reality:** The lowered MIR is clean and handles loops/branches via explicit jump conditionals (`goto`, `if goto else`). Call temporaries are typed from known signatures, and MIR lowering now returns structured errors for missing bindings instead of panicking.

---

## 3. Backends & Compilation Reality

### 3.1 C Transpilation Backend (`src/backend/codegen.rs`)
*   **How it works:** Directly walks the AST and emits standard, conforming C99 code.
*   **MSVC Compatibility:** Refactored to not rely on GCC statement expressions (`({ ... })`) or the `__auto_type` keyword. Uses concrete variable types resolved from the symbol table. It is intended to compile under Microsoft Visual C++ (`cl.exe`), GCC, and Clang, but backend feature parity is still incomplete.
*   **Built-ins:** Standard built-ins are generated as static C helper functions prepended to the output file.

### 3.2 Cranelift AOT Backend (`src/backend/cranelift/`)
*   **How it works:** Directly lowers the flat MIR control-flow graph into Cranelift IR using `cranelift-frontend` and outputs an ELF/COFF object file (`.o`).
*   **Struct Memory Layout:** Custom structures compile to flat 64-bit unboxed fields in heap memory. Offset for field `i` is calculated as `i * 8` bytes.
*   **Dynamic Allocations:** Struct allocations are compiled as native calls to the runtime function `lpp_alloc(size)` (backed by `calloc` in C).
*   **String Literals:** Handled by creating static symbols in the object file's data section (`DataDescription`), loading the symbol address dynamically inside the builder context.
*   **Reliability note:** Recent cleanup replaced several `unwrap()`-style backend assumptions with propagated `Result` errors so lowering failures are surfaced as compiler diagnostics instead of process crashes.
*   **Linking:** Native project builds now prefer platform-aware linking in the package manager: `link.exe` plus `lpp_runtime.obj` on Windows when available, otherwise `cl.exe`, `cc`, `gcc`, or `clang` compile/link against `lpp_runtime.c`. Installer helpers exist for both `install.ps1` and `install.sh`.

---

## 4. Current Limitations & Stubbed Features

As of version 0.1.0, some advanced language features are syntactically parsed and typechecked, but their backend implementations are stubs:

1.  **Closures & Environment Lifting (`Rvalue::MakeClosure`, `Rvalue::CallIndirect`):**
    *   *Parsed & Typechecked:* Yes.
    *   *MIR representation:* Exists.
    *   *Backend compilation:* Currently stubbed to return `0` (null/empty values) in the Cranelift AOT backend. Environment struct lifting (wrapping captured free variables into a dynamic closure structure) is not yet implemented.
2.  **Lists (`Rvalue::AllocateList`, `lpp_list_*` built-ins):**
    *   *Parsed & Typechecked:* Yes.
    *   *Backend compilation:* Basic list constructors compile to `lpp_list_new()` stubs. List index access (`list[i]`) and mutation are not fully connected in AOT codegen.
3.  **Garbage Collection / Reference Cycle Collector:**
    *   *ARC insertions:* Reference count increment and decrement instructions are inserted in the MIR, but the physical runtime decrements/frees for cyclic structures (e.g. self-referencing nodes) require a cycle collector which is not yet present.
