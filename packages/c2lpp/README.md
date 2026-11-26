# c2lpp v0.4.0 — Native-First C-to-L++ Translator & FFI Binder

`c2lpp` maps C types, structs, and functions directly to L++'s native ABI types (`I8`–`I64`, `U8`–`U64`, `USize`, `ISize`, `Float`, `Bool`, `Ptr[T]`, `RawPtr`, `Str`, `@repr(C)` structs) with zero `Int` erasure and zero 64-byte fat handles.

## Memory & Performance Model
- **`CSpan` (16 bytes)**: `@repr(C) struct CSpan { base: RawPtr, len: USize }` — fits in 2 CPU registers, stack-placed in `Frame` with zero ARC traffic, single-branch bounds + null check before direct `ptr_read[T]` / `ptr_write[T]`.
- **`FastHandle` (8 bytes)**: Packs `(index: 32-bit | generation: 32-bit)` into a single 64-bit register when generation-checked arena handles are needed.
- **Direct `@repr(C)` Structs & `extern "C"`**: Zero marshaling overhead.

## Usage
```bash
keel add c2lpp
lpp packages/c2lpp/src/main.lpp --backend llvm -O3 --lto --run -- --selftest
lpp packages/c2lpp/src/main.lpp --backend llvm -O3 --lto --run -- --bench
```
