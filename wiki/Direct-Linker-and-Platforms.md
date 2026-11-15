# Direct Linker & Platforms

L++ includes an in-process direct linker (`crates/lpp-linker` / `lpp-link`) supporting ELF, PE/COFF, and Mach-O object output alongside `--linker host` fallback.

---

## 1. Supported Targets & Backends (`v0.2.0`)

| Target Triple / Alias | Architecture / OS | Default Backend | Notes |
|---|---|---|---|
| Host `x86_64` | Linux (ELF), Windows (PE), macOS (Mach-O) | Cranelift | Primary native target; single-file build/run links for the compiler's host OS |
| Host `aarch64` | Linux (ELF), macOS (Mach-O) | Cranelift | Cross-arch native objects can be emitted via `--emit-object` / `--check` |
| `wasm32-wasip1` / `wasm32` | WebAssembly (WASI) | WASM (`lpp-codegen-wasm`) | Emits validated `.wasm` directly without external wasm linkers |
| `--backend llvm` | `x86_64` | LLVM (Experimental) | Requires `clang`; scalar/direct-call subset (`specs.md` §8.1) |

*(Compatibility target aliases such as `aarch64-linux-android` map into the generic `Aarch64` backend target and require setting NDK `ANDROID_CC` / `LPP_CC` to link; L++ does not bundle Android or cross-OS sysroots.)*

---

## 2. Direct Linker vs Host Linker (`--linker`)

Select linker behavior via `--linker direct|cc|host` or `lpp config set linker <mode>`:

- **`auto` / `direct`**: Uses `lpp-linker` (`lpp-link`) to emit native ELF, PE, or Mach-O binaries in memory where supported, and in `auto` mode prints diagnostics and falls back to the host toolchain if an object/import falls outside the direct-linker subset.
- **`host` / `cc`**: Delegates linking to the host C toolchain (`clang` / `gcc` / `cl.exe`), required for general external C libraries (`-l <lib>`) on platforms where direct linking does not yet handle those imports.

---

## 3. Standalone `lpp-link`

`lpp-link` is also built as a standalone binary with explicit `elf`, `pe`, `macho`, or auto-detected input format modes:
```bash
lpp-link --help
```
See [`docs/Native_Linker_Roadmap.md`](../docs/Native_Linker_Roadmap.md) and [`specs.md`](../specs.md) §8.2–§8.3 for the exact boundaries of direct-link vs host-link execution.
