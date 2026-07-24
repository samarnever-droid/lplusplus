# Compiler Developer Notes

This page is for contributors working inside the Rust compiler implementation.

## Important files

| File | Purpose |
|---|---|
| `src/frontend/lexer.rs` | tokens, literals, indentation |
| `src/frontend/parser.rs` | recursive descent parser |
| `src/frontend/ast.rs` | AST definitions |
| `src/analysis/semantic.rs` | scopes and binding resolution |
| `src/analysis/typecheck.rs` | type table and inference |
| `src/analysis/escape.rs` | ownership / escape analysis |
| `src/mir/lower.rs` | AST to MIR lowering |
| `src/mir/pass_*.rs` | MIR optimizations |
| `src/backend/cranelift/lower.rs` | MIR to Cranelift |
| `src/bin/lpp-link.rs` | direct linker |
| `src/builtins.rs` | builtin signatures |

## Rule of thumb

Language features usually touch:

1. AST
2. lexer/parser
3. semantic analysis
4. type checker
5. MIR lowering
6. Cranelift lowering only if a new operator or primitive operation is introduced

They should not touch `lpp-link` unless the executable format itself changes.

## Builtin features

New builtins usually touch:

1. `src/builtins.rs`
2. host runtime implementation
3. freestanding runtime implementation if direct linker support is required

## Always verify examples

For documentation examples, use a clean directory and run:

```bash
target/release/lpp --checkall
```

Do not use repo-wide `--checkall` as a documentation gate unless old negative/scratch files are excluded.
