# Roadmap to Self Hosting

Self-hosting means writing the L++ compiler in L++.

## What is already done

L++ now has most compiler-writing ingredients:

- ✅ Functions with default parameters
- ✅ Structs with field access
- ✅ Enums with match and data-carrying variants
- ✅ Generics (phase 1, type-erased): `def identity[T](x: T) -> T`
- ✅ Traits and impl blocks: `trait Display` + `impl Display for Point`
- ✅ Imports and multi-file modules
- ✅ Lists and maps
- ✅ 15+ string builtins: `char_at`, `ord`, `chr`, `str_find`, `str_contains`, `str_replace`, `str_upper`, `str_lower`, `str_trim`, `int_to_str`, `str_to_int`
- ✅ Error propagation with `?` try operator
- ✅ Native AOT compilation to ELF/PE/Mach-O
- ✅ Constants, closures, threads
- ✅ F-strings, hex/binary literals, multiline strings

## Remaining gaps

### 1. Stronger generics

Needed for:

```lpp
List[Token]
Map[Str, Type]
Result[AstNode]
```

Current generics use type erasure. Self-hosting needs monomorphization or runtime type tags for safe generic containers.

### 2. Trait bounds on generics

```lpp
def print_all[T: Display](items: List[T]):
    for item in items:
        print_str(item.display())
```

Traits exist but cannot yet constrain generic parameters.

### 3. Char type and string iteration

A lexer wants to process text one character at a time. `char_at`, `ord`, and `chr` work, but a dedicated `Char` type and efficient string iteration (`for c in s:`) would make lexer code cleaner.

### 4. Structured error objects

Current `Result` examples use integer error codes. A compiler needs structured errors with message, file, line, column, and error kind.

### 5. String-keyed hash maps

The current `map_*` builtins support integer keys. A compiler needs `Map[Str, Type]` for symbol tables and scope resolution.

### 6. Standard library hardening

Self-hosting requires stable modules for:

- String builder (efficient concatenation)
- File path operations
- Command-line argument parsing
- Testing helpers

## Suggested order

1. Add string-keyed map builtins.
2. Add trait bounds on generics.
3. Start a self-hosted lexer in L++.
4. Add Char type and string iteration.
5. Add structured compiler errors.
6. Expand to parser, AST, type checker, MIR.
