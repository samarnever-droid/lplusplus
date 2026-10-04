# L++ (LPlusPlus) v1.0.0 Workspace Instructions

See [AGENTS.md](file:///c:/Users/khati/lpp/AGENTS.md) for the complete language manual, standard library builtins, and package manager instructions.

## Quick CLI Reference
- `lpp main.lpp -o app.exe` — Compile and link directly to a native executable in <5ms.
- `lpp run main.lpp` — Compile and run immediately.
- `lpp --opt main.lpp` — Compile with LLVM -O3 optimizations.
- `lpp --target wasm32 main.lpp -o app.wasm` — Compile directly to WebAssembly.
- `lpp --linker host main.lpp` — Compile with system host linker (cl.exe / cc) for external C libraries.
