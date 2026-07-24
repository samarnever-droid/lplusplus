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

1. AST (`src/frontend/ast.rs`)
2. lexer/parser
3. semantic analysis
4. type checker
5. MIR lowering
6. Cranelift lowering only if a new operator or primitive operation is introduced

They should not touch `lpp-link` unless the executable format itself changes.

## Trait / impl internals

Impl methods are **name-mangled** as `TargetType_methodName` and treated as regular top-level functions throughout the compiler pipeline. The `self` parameter type is rewritten from `Self` to the concrete target type during parsing.

### Static dispatch

UFCS dispatch (`p.method()` → `method(p)`) resolves the mangled name at MIR lowering time by inspecting the receiver's type. If `p` is a `Point`, the compiler calls `Point_method(p)` directly.

### Dynamic dispatch

When a function parameter's type is a trait name (e.g., `def f(x: Speak)`), the compiler:

1. Adds hidden function pointer parameters (one per trait method) to the function signature
2. At call sites, fills in the concrete impl method's `FuncRef` (via `Rvalue::FuncRef`)
3. Inside the function, method calls on the trait-typed param dispatch via `CallIndirect` through the function pointer

This uses the new `Rvalue::FuncRef(FuncId)` MIR instruction, which lowers to Cranelift's `func_addr` to get a callable function pointer.

### Key files

- Parser: `parse_trait()`, `parse_impl()` in `parser.rs`
- Semantic: trait method short names tracked in `trait_method_names` set
- Type checker: `func_return_types` / `func_param_types` populated for mangled names; `trait_names` set for type resolution
- MIR: `trait_defs`, `impl_registry`, `current_vtable_locals` on `MirLowerCtx`; `Rvalue::FuncRef` for function pointers
- Cranelift: `FuncRef` → `func_addr`; `CallIndirect` distinguishes closure structs from direct function pointers

## Runtime cache

The freestanding runtime object is cached at `LppData/cache/<target>/lpp_runtime_min.o`. Invalidation uses a **content hash** of the C source (stored in `runtime.hash`), not timestamps. See [[Direct Linker and Runtime]] for details.

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
