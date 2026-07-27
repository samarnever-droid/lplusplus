# Feature Status Matrix

This page separates **implemented**, **experimental**, and **planned** features so users do not confuse parsed syntax with production-ready behavior.

## Legend

| Status | Meaning |
|---|---|
| Stable | Used by normal examples/tests and expected to work |
| Working | Implemented and useful, but still young |
| Experimental | Parsed or partly implemented, but limitations are known |
| Planned | Not implemented yet |

## Language features

| Feature | Status | Notes |
|---|---:|---|
| Functions | Stable | `def name(args) -> Type:` |
| Default parameters | Working | `def add(a: Int, b: Int = 10)` |
| Immutable variables | Stable | `x := 1`, deep field mutation check enforced (`a.b.c = 2` rejected if `a` is immutable) |
| Mutable variables | Stable | `mut x := 1`, then `x = 2` |
| Constants | Working | top-level `const NAME = value` |
| Structs | Stable | positional constructor: `Point(1, 2)` |
| Nested structs | Working | nested field access works: `rect.top_left.x` |
| Method syntax / UFCS | Working | `p.method()` becomes `method(p)` |
| Enums | Working | unit and integer-payload variants work |
| Match | Working | variant arms and bindings work |
| `?` try operator | Working | works with packed Result-like enum values |
| Generic functions | Stable | Monomorphized zero-overhead generics with trait bounds (`[T: Display]`) |
| Generic structs | Stable | `struct Box[T]` with monomorphization & static cycle detection |
| Generic enums | Working | Monomorphized generic payload enums |
| Type aliases | Working | `type Name = Target` type alias substitution |
| Closures | Working | `fn(...) -> Type:` syntax, automatic ARC escape promotion for `List`, `Map`, `Str` |
| Threads | Working | `spawn fn(): ...` |
| List literals | Working | `[1, 2, 3]`, float lists, in-place `list_set` |
| Maps | Working | integer and string keys work in runtime builtins |
| String indexing | Working | `s[0]` returns a one-character `Str` |
| List indexing | Working | `list[0]` lowers to `list_get` |
| F-strings | Stable | string expressions with automatic coercion (`int_to_str`, `float_to_str`, `bool_to_str`) |
| Multiline strings | Working | triple quotes `"""..."""` |
| Hex/binary literals | Working | `0xFF`, `0b1010`, underscores allowed |
| Float modulo | Working | `%` on floats lowers to `fmod` |
| Logical operators | Working | `&&`, `||`, `!`, with short-circuit for `&&`/`||` |
| Bitwise operators | Working | `&`, `|`, `^`, `<<`, `>>` |
| `pub` keyword | Experimental/reserved | lexer recognizes it; visibility enforcement is future work |
| Import aliases | Working | parser and resolver support `import x as y` |
| Traits/interfaces | Stable | `trait Name:` + `impl Trait for Type:` with static/dynamic dispatch and generic bounds |
| FFI / extern "C" | Working | `extern "C":` blocks, auto host linker, `link "lib"` support |
| Rust-style Diagnostics | Stable | Error codes (`E0001`–`E0005`), line:col coordinates, carets, help hints, `[suggestion]` |
| Auto-Fix Engine | Stable | `lpp --checkall --fix` automatically repairs code on disk |
| Runtime Panic Engine | Stable | `lpp_panic`, signal handlers (`SIGSEGV`, `SIGFPE`, `SIGABRT`), stack backtrace |
| Stdio LSP Server | Stable | `lpp-lsp` binary for editor completions, hovers, jump-to-def, diagnostics |
| Self-Hosted PM | Stable | `lpp-pm` fully written in L++ (`pm/src/main.lpp`) with embedded Git & HTTP engine |
| Char Primitive Type | Stable | Dedicated `Char` primitive type (`'a'`, `'\n'`, `'\t'`, `'\\'`) across compiler pipeline |

## Standard library status

| Module | Status | Notes |
|---|---:|---|
| `stdlib.math` | Stable | arithmetic helpers such as `pow`, `gcd`, `fib` |
| `stdlib.strings` | Stable | helpers built on string builtins |
| `stdlib.collections` | Stable | list helpers |
| `stdlib.convert` | Stable | `int_to_str`, `bool_to_str` style helpers |
| `stdlib.assert` | Stable | assertion helpers |
| `stdlib.algo` | Stable | sorting & search algorithms using `list_set` |
| `stdlib.result` | Stable | enum Result/Option types with pattern matching and arithmetic tag unwrapping |
| `stdlib.lreact` | Stable | pure L++ Tauri-like React desktop GUI framework bridge |
| `packages/lpp-zip` | Stable | pure L++ ZIP package |
| `packages/lppstore` | Stable | enterprise WAL database engine handling 164,563 OPS |

## Runtime and linker status

| Area | Status | Notes |
|---|---:|---|
| Host runtime | Working | full libc-backed runtime path |
| Linux freestanding runtime | Working/expanding | many string/buffer functions added; direct-link path improving |
| Windows freestanding runtime | Working for core tests | PE direct linker passes King20, but runtime builtin coverage differs from host path |
| `lpp-link` ELF | Working | direct ELF executable path |
| `lpp-link` PE | Working | smallest Windows binaries; do not edit for normal language features |
| `lpp-link` Mach-O | Basic working | macOS host-link tests pass |
