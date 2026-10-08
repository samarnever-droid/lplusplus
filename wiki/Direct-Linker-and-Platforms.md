# Direct Linker & Target Platforms

One of the defining innovations of the L++ compiler is its **in-process direct linker** (`lpp-linker`). Rather than shelling out to external toolchain linkers (such as Microsoft's `link.exe` or GNU `ld`), L++ can synthesize native executable binaries directly in memory and write them to disk.

---

## 1. Why Direct In-Process Linking?

Traditional compilers (such as Clang, Rustc, or GCC) compile code into intermediate object files (`.o` or `.obj`), then launch a heavy external subprocess linker. This causes:
1. **Slow Build Times:** Process spawning, disk I/O for temporary `.obj` files, and command-line parsing add hundreds of milliseconds even for small programs.
2. **Fragile Host Dependencies:** On Windows, compiling native code typically requires a multi-gigabyte Visual Studio or C++ Build Tools installation just for `link.exe` and SDK libraries.
3. **Complex Cross-Compilation:** Cross-compiling across operating systems requires setting up complex sysroots and target linkers.

L++ solves this by bundling a native PE/COFF and ELF linker directly into the compiler binary:
- **Sub-10ms Link Times:** Links standard binaries entirely in memory.
- **Zero-Dependency Native Binaries:** Produces valid Windows `.exe` files out of the box.
- **Hermetic Reproducibility:** Identical binary section layouts across host environments.

---

## 2. In-Process PE/COFF Direct Linker (Windows)

When targeting `x86_64-pc-windows-msvc`, `lpp-linker` synthesizes PE32+ executables:

```text
[DOS Header & Stub]
        │
[PE Header (Signature, COFF Header, Optional Header 64-bit)]
        │
[Section Table]
├── .text   ── Executable code (from Cranelift AOT)
├── .rdata  ── String literals, constants, import address table (IAT)
├── .data   ── Mutable global state
├── .pdata  ── Exception handling unwind function table
├── .xdata  ── Unwind info and backtrace descriptors
└── .idata  ── Direct import descriptors for KERNEL32.dll, MSVCRT.dll
```

### Automatic Import Resolution
The direct PE linker automatically constructs import tables for essential Windows system DLLs:
- `KERNEL32.dll` (`ExitProcess`, `GetStdHandle`, `WriteFile`, `ReadFile`, `VirtualAlloc`, `VirtualFree`)
- `MSVCRT.dll` or Universal CRT (`malloc`, `free`, `printf`, `memcpy`)

---

## 3. Direct ELF Linker (Linux)

When targeting `x86_64-unknown-linux-gnu`, `lpp-linker` synthesizes standard 64-bit System V ELF executables:
- Generates `Elf64_Ehdr`, `Elf64_Phdr` (Program Headers), and section tables.
- Creates `PT_LOAD` segments for code (`.text`) and data (`.rodata`, `.data`).
- Synthesizes entry trampolines adhering strictly to the System V AMD64 ABI.

---

## 4. System Linker Fallback

When a project links against external C libraries using `extern "C" link "..."` (e.g. `SDL2`, `sqlite3`, `curl`), the compiler can automatically escalate to the host system's native linker:
- On Windows: Detects and invokes `link.exe` via the Microsoft vswhere locator.
- On Linux/macOS: Invokes `cc` / `gcc` / `clang`.

You can explicitly force the system linker or the direct linker via CLI flags:
```bash
# Force internal direct linker
lpp build main.lpp --linker direct

# Force host system linker
lpp build main.lpp --linker system
```

---

## 5. Supported Target Triples

| Target Triple | Output Format | Linker Engine | Status |
|---|---|---|---|
| `x86_64-pc-windows-msvc` | PE32+ `.exe` | Direct PE / MSVC `link.exe` | **Tier 1 (Production)** |
| `x86_64-unknown-linux-gnu` | ELF64 Executable | Direct ELF / `gcc` / `ld` | **Tier 1 (Production)** |
| `wasm32-wasip1` | WebAssembly `.wasm` | Direct WASM binary emission | **Tier 1 (Production)** |
| `aarch64-unknown-linux-gnu` | ELF64 Executable | Cross-compile via Cranelift | **Tier 2 (Supported)** |
| `x86_64-apple-darwin` | Mach-O 64-bit | Direct Mach-O / `clang` | **Tier 2 (Supported)** |

To specify a target triple:
```bash
lpp build main.lpp --target wasm32-wasip1 -o output.wasm
lpp build main.lpp --target x86_64-unknown-linux-gnu -o output.elf
```
