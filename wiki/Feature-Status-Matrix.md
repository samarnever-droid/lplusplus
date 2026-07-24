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
| Immutable variables | Stable | `x := 1` |
| Mutable variables | Stable | `mut x := 1`, then `x = 2` |
| Constants | Working | top-level `const NAME = value` |
| Structs | Stable | positional constructor: `Point(1, 2)` |
| Nested structs | Working | nested field access works: `rect.top_left.x` |
| Method syntax / UFCS | Working | `p.method()` becomes `method(p)` |
| Enums | Working | unit and integer-payload variants work |
| Match | Working | variant arms and bindings work |
| `?` try operator | Working | works with packed Result-like enum values |
| Generic functions | Experimental | phase 1, type-erased, common inference works |
| Generic structs | Experimental | `struct Box[T]` parses and works for simple cases |
| Generic enums | Experimental | syntax exists, payload limitations remain |
| Type aliases | Experimental | parsed, but full typechecker substitution is incomplete |
| Closures | Working | `fn(...) -> Type:` syntax and captures |
| Threads | Working | `spawn fn(): ...` |
| List literals | Working | `[1, 2, 3]`, float lists also work |
| Maps | Working | integer and string keys work in runtime builtins |
| String indexing | Working | `s[0]` returns a one-character `Str` |
| List indexing | Working | `list[0]` lowers to `list_get` |
| F-strings | Working | string expressions work; use `int_to_str(x)` for integers |
| Multiline strings | Working | triple quotes `"""..."""` |
| Hex/binary literals | Working | `0xFF`, `0b1010`, underscores allowed |
| Float modulo | Working | `%` on floats lowers to `fmod` |
| Logical operators | Working | `&&`, `||`, `!`, with short-circuit for `&&`/`||` |
| Bitwise operators | Working | `&`, `|`, `^`, `<<`, `>>` |
| `pub` keyword | Experimental/reserved | lexer recognizes it; visibility enforcement is future work |
| Import aliases | Experimental | parser supports `import x as y`; namespace behavior is still limited |
| Traits/interfaces | Working | `trait Name:` + `impl Trait for Type:` with static and dynamic dispatch |
| Full monomorphized generics | Planned | current generics use type erasure; monomorphization is future work |
| Char type | Planned | currently use one-character `Str`, `ord`, `chr`, `char_at` |

## Standard library status

| Module | Status | Notes |
|---|---:|---|
| `stdlib.math` | Working | arithmetic helpers such as `pow`, `gcd`, `fib` |
| `stdlib.strings` | Working/experimental | helpers built on string builtins |
| `stdlib.collections` | Working/experimental | list helpers |
| `stdlib.convert` | Working | `int_to_str`, `bool_to_str` style helpers |
| `stdlib.assert` | Experimental | implemented in pure L++; process-exit behavior is not a true compiler trap |
| `stdlib.algo` | Experimental | currently depends on missing `list_set` in some functions |
| `stdlib.result` | Experimental | enum helper arithmetic is not fully type-safe yet |
| `packages/lpp-zip` | Experimental | pure L++ ZIP package, host-runtime path recommended |

## Runtime and linker status

| Area | Status | Notes |
|---|---:|---|
| Host runtime | Working | full libc-backed runtime path |
| Linux freestanding runtime | Working/expanding | many string/buffer functions added; direct-link path improving |
| Windows freestanding runtime | Working for core tests | PE direct linker passes King20, but runtime builtin coverage differs from host path |
| `lpp-link` ELF | Working | direct ELF executable path |
| `lpp-link` PE | Working | smallest Windows binaries; do not edit for normal language features |
| `lpp-link` Mach-O | Basic working | macOS host-link tests pass |
