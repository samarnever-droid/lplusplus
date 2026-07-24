# Roadmap to Self Hosting

Self-hosting means writing the L++ compiler in L++.

L++ already has many compiler-writing ingredients:

- functions
- structs
- enums
- match
- imports
- lists and maps
- strings and files
- generics phase 1
- error propagation
- native compilation

The remaining important gaps are below.

## 1. Stronger generics

Needed for:

```lpp
List[Token]
Map[Str, Type]
Result[AstNode]
```

Current generics are phase 1 with type erasure. Self-hosting needs more precise generic substitution, better payload handling, and likely monomorphization or a stable erased representation with runtime tags.

## 2. Traits / interfaces

Needed for reusable compiler abstractions:

```text
Display
Iterator
Visitor
ParserBackend
CodegenTarget
```

## 3. Char type and richer string iteration

A lexer wants to process text one character at a time. L++ currently has string indexing and `char_at`, `ord`, `chr`, but a dedicated `Char` type and efficient string iteration would make lexer code cleaner.

## 4. Better Result and error objects

Current `Result` examples use integer error codes. A compiler needs structured errors:

```text
message
file
line
column
error kind
```

## 5. Arrays and slices

Useful for token buffers, bytecode buffers, and linker data structures.

## 6. Standard library hardening

Self-hosting requires stable modules for:

- string builder
- file paths
- JSON/TOML parsing
- command-line parsing
- hash maps with string keys
- testing helpers

## Suggested order

1. Fix type alias substitution fully.
2. Strengthen generics.
3. Add traits/interfaces.
4. Add Char and string iterator support.
5. Add structured compiler errors.
6. Start a small self-hosted lexer in L++.
7. Expand to parser, AST, type checker, MIR.
