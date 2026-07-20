# L++ Compiler Usage Guide

This guide explains how to compile L++ programs, configure execution modes, and use the compiler's debugging interface.

---

## 1. CLI Commands & Wrapper Options

When installed globally via `.\install.ps1`, the L++ compiler command `lpp` is added to your environment `PATH`.

### 1.1 Compilation
To compile an L++ source file into a native Windows executable:
```cmd
lpp [filename.lpp]
```
This performs ahead-of-time (AOT) compilation using the Cranelift backend to generate an object file, links it with the precompiled L++ C runtime library using MSVC `link.exe`, and outputs a standalone executable `[filename].exe` in the same directory.

### 1.2 Run Compiled Binary Directly
To compile and execute the program immediately in one step:
```cmd
lpp [filename.lpp] --run
```

---

## 2. Compiler Debugging Dumps

For inspecting the compiler's internal pipeline representation, use the `--dump-*` options when running the compiler binary:

| Flag | Purpose |
|---|---|
| `--dump-ast` | Prints the parsed Abstract Syntax Tree (AST) structure in Rust debug format. |
| `--dump-symbols` | Prints the symbol tree and scope binding associations resolved during semantic analysis. |
| `--dump-types` | Prints the typechecker's inferred types for all identifiers and expressions. |
| `--dump-escape` | Prints the storage classification map (which variables are allocated on stack vs. heap). |
| `--dump-mir` | Prints the lowered Mid-level IR (MIR) control-flow graph basic blocks. |
| `--dump-c` | Prints the transpiled C99 code produced by the C backend. |

*Example:* To dump the control-flow graph (MIR) of your calculator:
```cmd
lpp-compiler calc.lpp --dump-mir
```

---

## 3. Local Script Runners (For Compiler Developers)

Inside the repository root, there are two utility scripts:

1.  **`.\run.ps1 [file.lpp]`**
    *   Compiles the file, runs it, and automatically cleans up the generated object (`.o`) and executable (`.exe`) files. Very useful for quick tests.
2.  **`.\install.ps1`**
    *   Builds the compiler in release mode, copies the binary to `%USERPROFILE%\.lpp\bin`, compiles `lpp_runtime.c` to `lpp_runtime.obj`, generates the global command wrapper, and appends it permanently to the user's `PATH`.

---

## 4. Environment Variables

*   **`LPP_AOT=1`**: Triggers Cranelift compilation to a native object file.
*   **`BENCHMARK=1`**: Suppresses diagnostic logs and prints timing metrics as a single JSON line:
    ```json
    TIMING_JSON: {"io": 0.00016, "lex": 0.00006, "parse": 0.00004, "semantic": 0.00003, "typecheck": 0.000008, "escape": 0.002, "mir": 0.00004, "aot": 0.002, "total": 0.003}
    ```

## Current v0.1.3 status note — 2026-07-20

This document is historical/design context. For current public capability claims,
platform boundaries, filesystem APIs, package cache layout, and known missing
features, see [Current Capabilities](CURRENT_CAPABILITIES.md).

Current rules:

```text
- Do not claim fixed compile-time, binary-size, or C/Rust parity numbers.
- Do not claim language-wide Rust-equivalent safety.
- Host-linked AOT is the compatibility path for filesystem and networking work.
- Linux direct ELF remains a verified subset; filesystem/networking are not direct-link features yet.
- macOS ARM64 static direct output is rejected; dynamic libSystem imports are required.
- L++ package outputs/cache are LppData/build/release and LppData/cache.
```
