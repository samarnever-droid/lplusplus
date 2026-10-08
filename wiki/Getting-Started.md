# Getting Started with L++

This guide covers installing the L++ toolchain, setting up your environment, running your first program, and navigating the CLI.

---

## 1. Installation

### Windows (PowerShell)
You can install L++ directly using PowerShell:
```powershell
# Automated setup
iwr -useb https://lplusplus.bond/install.ps1 | iex
```

Or from source:
```powershell
git clone https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
$env:LPP_FROM_SOURCE = "1"
.\install.ps1
```

### Linux / macOS (Bash)
```bash
# Automated setup
curl -fsSL https://lplusplus.bond/install.sh | sh
```

Or from source:
```bash
git clone https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
LPP_FROM_SOURCE=1 sh install.sh
```

### Building with Cargo
L++ is written in Rust (2024 edition). If you already have Rust and Cargo installed:
```bash
git clone https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
cargo build --release -p lpp
```
The executable is generated at `target/release/lpp` (or `target/release/lpp.exe` on Windows).

---

## 2. Verifying the Toolchain

Run `lpp doctor` to verify that your environment, linker paths, and compiler tools are correctly detected:

```bash
lpp doctor
```

Example output:
```text
=== L++ Environment & Toolchain Doctor ===
Compiler Binary:        C:\Users\khati\lpp\target\release\lpp.exe
Target Triple:          x86_64-pc-windows-msvc
Rust Host Toolchain:    rustc 1.85.0
Linker:                 Direct PE In-Process Linker (Native MSVC compatible)
C Compiler Backend:     Pure Native Cranelift AOT
WebAssembly Support:    Built-in (wasm32-wasip1)
Status:                 Ready for compilation
```

Check the installed version:
```bash
lpp --version
```
Output:
```text
L++ Compiler v0.1.0 (Pure Native AOT)
```

---

## 3. Your First Program

Create a file named `hello.lpp`:

```lpp
def main():
    name := "World"
    println("Hello, " + name + " from L++!")
```

### Running Directly
The `run` subcommand compiles the program to a native binary and immediately executes it:

```bash
lpp run hello.lpp
```
Output:
```text
Hello, World from L++!
```

### Compiling to a Standalone Executable
The `build` subcommand produces an optimized standalone executable without running it:

```bash
lpp build hello.lpp -o hello.exe
```

Run the generated binary:
```bash
./hello.exe
```

### Fast Static Checking
The `check` subcommand performs lexing, parsing, scope resolution, type checking, and borrow verification without invoking code generation or linking. This gives sub-millisecond feedback in IDEs and terminals:

```bash
lpp check hello.lpp
```

---

## 4. CLI Reference

| Command | Usage | Description |
|---|---|---|
| `lpp run <file.lpp>` | `lpp run src/main.lpp` | Compile and run target immediately |
| `lpp build <file.lpp>` | `lpp build -o app.exe src/main.lpp` | Compile to standalone native executable |
| `lpp check <file.lpp>` | `lpp check src/main.lpp` | Validate syntax, types, and ownership without codegen |
| `lpp doctor` | `lpp doctor` | Print diagnostics on compiler environment and linkers |
| `lpp new <name>` | `lpp new my_project` | Scaffold a new L++ project with `lpp.toml` |
| `lpp test` | `lpp test` | Run test suites declared within the project |
| `lpp clean` | `lpp clean` | Remove compiler build artifacts and cache |

### Common CLI Options

- `-o, --output <path>`: Specify the output executable or object path.
- `--opt, -O <0|1|2|3|s|z>`: Set optimization level (default: 2 for build, 0 for run).
- `--target <triple>`: Target architecture (e.g., `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `wasm32-wasip1`).
- `--emit <asm|mir|hir|obj|exe>`: Emit intermediate representations for inspection.
- `--verbose, -v`: Enable verbose compiler pipeline logging.

---

## 5. WebAssembly Target

To build a standalone `.wasm` module for WebAssembly runtimes (e.g., [wasmtime](https://wasmtime.dev/) or browsers with WASI):

```bash
lpp build hello.lpp --target wasm32-wasip1 -o hello.wasm
wasmtime hello.wasm
```
No external C compiler or wasm linker is required. L++ emits standalone WASI binary modules directly.
