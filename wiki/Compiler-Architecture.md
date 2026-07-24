# Compiler Architecture

The L++ compiler is written in Rust and is organized as a classic native compiler pipeline.

```text
.lpp source
  -> lexer
  -> parser / AST
  -> semantic analysis
  -> type checking
  -> escape analysis
  -> MIR lowering
  -> MIR optimization passes
  -> Cranelift codegen
  -> object file
  -> host linker or lpp-link
  -> executable
```

## Frontend

Files:

- `src/frontend/lexer.rs`
- `src/frontend/parser.rs`
- `src/frontend/ast.rs`

The lexer handles indentation tokens, comments, literals, keywords, operators, f-strings, multiline strings, hex/binary literals, and underscore digit separators.

The parser is recursive descent and builds the AST.

Top-level AST declarations:

- `Function` — with optional type parameters and default parameter values
- `Struct` — with optional type parameters
- `Enum` — with optional type parameters and data-carrying variants
- `Import`
- `Const`
- `TypeAlias`
- `Trait` — interface definitions with method signatures
- `Impl` — trait implementations for a target type; methods mangled as `TargetType_method`; supports static and dynamic dispatch via hidden function pointer params

## Semantic analysis

File:

- `src/analysis/semantic.rs`

Responsibilities:

- lexical scopes
- binding IDs
- variable/function resolution
- mutability checks
- import resolution
- match-binding scope setup

## Type checker

File:

- `src/analysis/typecheck.rs`

Key type representations:

```rust
TypeRef::Int
TypeRef::Float
TypeRef::Str
TypeRef::Bool
TypeRef::Void
TypeRef::Custom(StructTypeId)
TypeRef::Generic(String, Vec<TypeRef>)
TypeRef::TypeParam(String)
TypeRef::Unresolved(String)
```

Generics phase 1 is implemented through type parameters and erasure to `i64` at codegen.

## Escape analysis

File:

- `src/analysis/escape.rs`

Classifies values as:

- stack
- ARC heap
- arena / rejected cycle

This is where ownership safety is enforced before code generation.

## MIR

Files:

- `src/mir/lower.rs`
- `src/mir/ir.rs`
- `src/mir/pass_*.rs`

MIR lowering desugars high-level syntax:

- `p.method()` -> `method(p)`
- `x += y` -> `x = x + y`
- `s[i]` -> `str_substr(s, i, 1)`
- list indexing -> `list_get`
- `?` -> branch + early return
- match -> tag tests + branches
- short-circuit `&&` and `||` -> control-flow branches

Optimization passes include ARC insertion, closure lifting, constant propagation, DCE, branch simplification, peephole optimization, and inlining.

## Backend

Files:

- `src/backend/cranelift/compiler.rs`
- `src/backend/cranelift/lower.rs`
- `src/backend/cranelift/types.rs`

The backend lowers MIR to Cranelift IR and emits native object files.

## Link stage

File:

- `src/bin/lpp-link.rs`

The direct linker supports:

- Linux ELF
- Windows PE/COFF
- macOS Mach-O

Most language features never touch `lpp-link`. Builtins usually require runtime work, not linker work.
