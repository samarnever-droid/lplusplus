# Compiler Architecture

## Pipeline

```
Source (.lpp) → Lexer → Parser → Semantic → Typecheck → Escape → MIR → Cranelift → lpp-link → Executable
```

## Stages

### 1. Lexer (`src/frontend/lexer.rs`)
Converts source text to tokens. Handles significant whitespace (Indent/Dedent), 21 keywords, string/int/float/bool literals, operators.

### 2. Parser (`src/frontend/parser.rs`)
Recursive descent parser. Produces AST with: functions, structs, enums, imports, statements (if/while/for/match/break/continue), expressions (binary ops, calls, field access, closures, enum constructors, try operator).

### 3. Semantic Analysis (`src/analysis/semantic.rs`)
- Scope resolution (lexical scoping with function/closure/block nesting)
- Binding tracking (every variable gets a unique BindingId)
- Name resolution (identifiers → binding lookups)
- Import resolution (multi-file, dotted paths, stdlib)

### 4. Type Checker (`src/analysis/typecheck.rs`)
- Type inference for `:=` declarations
- Explicit type checking at function boundaries
- Struct field type resolution
- Enum variant type checking
- Builtin function parameter/return type validation

### 5. Escape Analysis (`src/analysis/escape.rs`)
- Classifies every binding as Stack, Heap (ARC), or Arena
- Rule 1: Values returned from functions → promoted to Heap
- Rule 2: Values stored in containers → promoted to Heap
- Rule 3: Self-referential structs → Arena allocation
- Cycle detection and rejection

### 6. MIR Lowering (`src/mir/lower.rs`)
Lowers AST to Mid-level IR (typed SSA-like representation). 7 optimization passes:

| Pass | File | What it does |
|------|------|-------------|
| ARC | `pass_arc.rs` | Insert retain/release for ownership |
| Closure | `pass_closure.rs` | Lift closures to top-level functions |
| Const Prop | `pass_constprop.rs` | Fold constant expressions |
| DCE | `pass_dce.rs` | Remove dead code |
| Branch | `pass_branch.rs` | Simplify conditional branches |
| Peephole | `pass_peephole.rs` | Local instruction optimization |
| Inline | `pass_inline.rs` | Inline small functions |

### 7. Cranelift Codegen (`src/backend/cranelift/`)
- `compiler.rs` — Module setup, function declarations, entry point wrapper
- `lower.rs` — MIR → Cranelift IR translation, register allocation
- `types.rs` — L++ type → Cranelift type mapping

### 8. Direct Linker (`src/bin/lpp-link.rs`)
Custom linker producing standalone executables:
- **Linux ELF** — Program headers, segments, entry point
- **Windows PE** — COFF sections (.text/.rdata/.data/.idata/.reloc), IAT, base relocations
- **macOS Mach-O** — Load commands, segments

## File Sizes

| Component | Lines |
|-----------|-------|
| Compiler total | ~14,800 |
| Builtins declarations | ~1,800 |
| Linker (lpp-link) | ~2,000 |
| Package manager | ~1,800 |
| Cranelift backend | ~1,300 |
| MIR (IR + passes) | ~2,500 |
| Parser | ~900 |
| Type checker | ~800 |
| Semantic analysis | ~600 |
| Escape analysis | ~600 |
