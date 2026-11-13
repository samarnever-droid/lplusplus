# Getting Started with L++

This guide covers installing the L++ toolchain, verifying your setup, and compiling and running your first native and WebAssembly programs.

---

## 1. Installation

### Linux & macOS
```bash
git clone --depth 1 --branch v0.2.0 https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
LPP_FROM_SOURCE=1 sh install.sh
```

### Windows (PowerShell)
```powershell
git clone --depth 1 --branch v0.2.0 https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
$env:LPP_FROM_SOURCE = "1"
.\install.ps1
```

Prebuilt release archives and `SHA256SUMS` manifests are published on the GitHub Releases page; the installers verify archive checksums before unpacking.

### Building from Source (Developers)
```bash
git clone https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
cargo build --locked --release --bin lpp --bin lpp-link --bin keel
```
Ensure `target/release` (or your `LPP_INSTALL_DIR`) is added to your system `PATH`.

---

## 2. Verifying Your Installation

Run the built-in environment diagnostics:
```bash
lpp doctor
```

Check the installed version:
```bash
lpp --version
```
Output:
```text
L++ v0.2.0 (rewrite engine)
```

---

## 3. Your First Program

Create a file named `hello.lpp`:

```lpp
def main():
    name := "World"
    print_str("Hello, " + name + " from L++!")
```

### Running Directly
Compile and run a single `.lpp` source file:
```bash
lpp hello.lpp --run
```
(When the first positional argument is a `.lpp` file, `lpp run hello.lpp` also runs the file compiler path.)

Output:
```text
Hello, World from L++!
```

### Compiling to a Standalone Executable
```bash
lpp hello.lpp -o hello.exe
```

Run the generated binary:
```bash
./hello.exe
```

### Fast Static Checking
`--check` validates syntax, scopes, types, ownership balance, and backend object generation without linking or writing an output executable:
```bash
lpp hello.lpp --check
```

---

## 4. CLI Reference

| Command | Usage | Description |
|---|---|---|
| `lpp <file.lpp> --run` | `lpp src/main.lpp --run` | Compile, link, and execute a source file |
| `lpp <file.lpp> -o <out>` | `lpp src/main.lpp -O2 -o app` | Compile to a standalone native executable |
| `lpp <file.lpp> --check` | `lpp src/main.lpp --check` | Check syntax, types, ownership, and codegen without linking |
| `lpp doctor` | `lpp doctor` | Report host, backend, linker, LLVM, and WASI tool availability |
| `lpp new <name>` | `lpp new my-app` | Scaffold a new Keel project with `Keel.toml` and `src/main.lpp` |
| `lpp build` / `lpp run` / `lpp test` | `lpp test` | Dispatch Keel package/workspace commands |

### Common CLI Options

- `-o <path>` / `--output <path>`: Output executable, object, or `.wasm` path.
- `-O <0|1|2|3|s|z>`: Optimization level (defaults to `O0` for direct compiler invocations).
- `--backend <cranelift|llvm|wasm>`: Select backend (`cranelift` is the default).
- `--target <target>`: Target architecture (`x86_64`, `aarch64`, `wasm32-wasip1`, etc.).
- `--linker <direct|cc|host>`: Select direct `lpp-link` or host C compiler linker.
- `--emit-object` (`-c`): Write `.o`/`.obj` or `.wasm` without native linking.
- `--lto`, `--pgo-generate`, `--pgo-use <file>`: Bounded MIR inlining and Cranelift PGO options.

---

## 5. WebAssembly Target (`wasm32-wasip1`)

```bash
lpp hello.lpp --target wasm32-wasip1 -o hello.wasm
wasmtime hello.wasm
```
L++ emits standalone validated `.wasm` modules directly without requiring an external C compiler or wasm linker. Passing a nonzero optimization level invokes `wasm-opt` when available.
