# L++ Wiki

Welcome to the L++ wiki.

L++ is a native, ownership-aware programming language built around a simple idea:

> **Readable like Python, safe like Rust, fast like Go, native by default.**

L++ is not an interpreter and not a VM language. Source files compile ahead-of-time through a Rust compiler frontend, a MIR optimization pipeline, Cranelift native code generation, and either the system linker or the custom `lpp-link` direct linker.

## Start here

1. [[Getting Started]] — install, build, run, and first project
2. [[Language Reference]] — syntax, types, operators, functions, structs, enums, match, generics
3. [[Errors and Result]] — `enum Result`, `match`, and the `?` operator
4. [[Modules and Packages]] — imports, package manager, registry
5. [[Standard Library and Builtins]] — strings, lists, maps, files, buffers, network, JSON
6. [[Compiler Architecture]] — lexer to executable
7. [[Type System and Safety]] — mutability, ownership, ARC, escape analysis
8. [[Direct Linker and Runtime]] — ELF, PE, Mach-O, freestanding runtimes
9. [[Verified Examples]] — examples checked with `lpp --checkall`
10. [[Benchmarks and CI]] — BPW benchmark workflow and CI jobs
11. [[Feature Status Matrix]] — stable vs experimental vs planned features
12. [[Runtime Compatibility Matrix]] — host vs direct runtime builtin support
13. [[Compiler Debugging Guide]] — dump flags, MIR, lpp-link inspect
14. [[Package Registry and lpp-zip]] — registry format and ZIP package API
15. [[Known Stale and Negative Files]] — why repo-wide checkall finds intentional failures
16. [[Roadmap to Self Hosting]] — what is still needed for a compiler written in L++

## Current capability snapshot

L++ currently supports:

- Native AOT compilation with Cranelift
- Direct executable generation through `lpp-link`
- Python-like indentation syntax
- Immutable-by-default variables and `mut`
- Functions with typed parameters and default parameter values
- Structs and UFCS-style method calls
- Enums with data-carrying variants
- `match` with bindings
- Rust-like `?` error propagation
- Generic functions, structs, and enums, phase 1
- Constants
- Type aliases, parsed/experimental
- List literals and indexing
- String indexing, f-strings, multiline strings
- `if`, `elif`, `else`, `while`, `for`, `range(start, end, step)`
- Short-circuit `&&` and `||`
- Unary `-x` and `!flag`
- Bitwise operators
- File, directory, process, buffer, JSON, HTTP, TCP/UDP builtins
- A pure L++ standard library
- A GitHub Pages JSON package registry

## Important accuracy note

The examples in this wiki were checked in a clean documentation example project using:

```bash
target/release/lpp --checkall
```

The full repository also contains old negative tests and deliberately invalid files used to test compiler rejection paths, so repo-wide `--checkall` is not the same as documentation-example validation.
