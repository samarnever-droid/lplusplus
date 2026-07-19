# Architecture

```text
source
  → lexer/parser
  → semantic resolution + type checking
  → ownership-aware MIR
  → ARC/escape analysis
  → Cranelift object generation
  → host linker or supported direct linker
  → native executable
```

## Authority boundaries

- MIR + Cranelift AOT are authoritative for ownership semantics.
- The C backend is a compatibility/debug reference.
- `lpp-link` owns direct-object validation and output construction.
- Runtime ABIs must be versioned/documented before compiler calls are added.

## Runtime layers

- `lpp_runtime.c`: host-link compatibility runtime
- `runtime/linux_x86_64_min.c`: Linux direct ELF minimal runtime
- `runtime/windows_x86_64_min.c`: Windows native runtime work
- `runtime/lpp-net`: Rust networking static-runtime foundation

## Design constraints

Every new runtime capability needs: a source-level contract, type checking, lowering/ABI support, ownership behavior, link packaging, cross-platform scope, and negative tests. A function declaration alone is not a completed language feature.
