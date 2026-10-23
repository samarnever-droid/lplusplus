# Imported C ABI calls

## Supported surface

The native Cranelift backend lowers direct, non-variadic `extern "C"` calls and passes each declared `link "library"` dependency to the host linker. The current signature subset is primitive values (`Bool`, `Int`, `Float`, `U8`/`I8`, `U16`/`I16`, `U32`/`I32`) plus `Str`; `Void` is permitted as a result only. Generic, variadic, defaulted, aggregate, callback, and Unicode `Char` signatures are rejected. The LLVM and WASM backends currently return a typed unsupported-construct error for external calls.

The extern declaration is the ABI contract: its parameter/result widths and calling convention must match the C prototype. In particular, L++ `Int` is 64-bit; use a fixed-width type such as `I32` when the C prototype uses a 32-bit integer. A foreign pointer/handle can be represented as `Int`, but its ownership and validity remain the caller's responsibility.

## `Str` at the boundary

An L++ `Str` is **not** an owned raw C string pointer. It is an ARC-managed allocation whose value points to a NUL-terminated payload, with the ARC header immediately before that payload. Native lowering handles the two directions differently:

- **L++ `Str` argument → C:** pass the payload address as a borrowed C string for the duration of the call. The foreign function must treat it as read-only and must not retain the pointer after returning. This does not transfer the L++ ARC reference. APIs that store a string pointer need an explicitly managed C buffer/handle instead.
- **C `char *` result → L++ `Str`:** treat the returned pointer as borrowed, NUL-terminated data that remains readable through the immediate copy, and copy its bytes into a fresh ARC-managed L++ string before normal MIR cleanup. The foreign pointer is never adopted or released. A `NULL` result follows the runtime string helper's existing contract and becomes the empty string.

These are FFI preconditions, not guarantees the compiler can verify. Do not declare mutable C string parameters as `Str`, and do not let a C function retain an L++ `Str` argument.

## Linking

Library names are bare linker names, for example `extern "C" link "sqlite3":`. The native driver collects the distinct libraries used by lowered external calls and passes them to the selected linker. The WASM backend does not currently support this host-library mechanism.
