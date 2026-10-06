<p align="center">
  <img src="assets/lpp-logo.svg" width="190" alt="L++ four-pillar prism logo">
</p>

<h1 align="center">L++</h1>

<p align="center"><strong>Readable by design · Native by default · Safety engineered in</strong></p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="https://lplusplus.bond">Website</a> ·
  <a href="Doc.md">Language Guide</a> ·
  <a href="wiki/">Wiki</a> ·
  <a href="benchmarks/king20/stable/v1/latest.md">Benchmarks</a>
</p>

---

## What is L++?

L++ is a statically typed, ownership-aware language built around three priorities: readable syntax, ahead-of-time native compilation, and memory safety engineered into the compiler pipeline. Its ownership planner classifies managed values as `Frame`, `Owned`, or `Shared`, while static balance checks catch common ownership mistakes before code generation. L++ does not use a tracing garbage collector, interpreter, or VM. Cranelift is the default native backend, with LLVM and direct WebAssembly output available for additional targets.

```lpp
struct User:
    name: Str
    age: Int

def greet(user: User):
    print_str(user.name)
    print(user.age)

def main():
    u := User("Alice", 30)
    greet(u)
```

### WebAssembly

The same compiler also emits WebAssembly — no toolchain, linker, or runtime
files needed, just a WASI-capable engine such as [wasmtime](https://wasmtime.dev/):

```bash
lpp hello.lpp --target wasm32-wasip1   # writes hello.wasm (imports WASI fd_write)
wasmtime hello.wasm                  # runs it
```

The wasm backend lowers the MIR straight to a binary module — structs, enums
+ `match`, tuples, lists, maps, closures and trait objects, async tasks,
slices, dynamic strings, and the portable builtin set all work, with the ARC
runtime re-implemented as pure wasm helpers. Platform-specific features that
a WASI sandbox cannot provide (threads, sockets, files, FFI, SIMD, child
processes) are rejected up front with clear diagnostics. See
[Doc.md §6.4](Doc.md) for the full support matrix.

## Key Features

| Feature | Description |
|---------|-------------|
| **Python-like syntax** | Significant whitespace, `:=` declarations, `def`/`struct`/`enum` |
| **Generics** | `def identity[T](x: T) -> T`, generic structs and enums with static cycle detection |
| **Traits + dispatch** | `trait`/`impl` with both static and dynamic dispatch |
| **FFI / extern** | `extern "C" link "SDL2"` — call any C library directly |
| **Ownership planning** | `Frame` / `Owned` / `Shared` placement, escape and cycle analysis, plus static balance checks |
| **Enums + match** | Algebraic data types with pattern matching and data extraction |
| **Error handling** | `Result` type + `?` operator for error propagation |
| **Diagnostics & Panic** | Rust-style error cards (`E0001`–`E0005`) + C runtime stack backtrace engine |
| **Default params** | `def foo(x: Int, y: Int = 10)` |
| **Multi-file modules** | `import math`, `from utils import calc`, dotted paths |
| **Native compilation** | Cranelift AOT by default; optional LLVM object backend |
| **WebAssembly** | `--target wasm32-wasip1` emits a runnable `.wasm` module with zero external tools |
| **Direct linker** | `lpp-link` produces standalone ELF / PE / Mach-O executables |
| **Arena regions** | Recursive structs use region-backed nodes with cycle-broken ownership |
| **Explicit vectors** | `VectorI64x2` operations in both Cranelift and LLVM |
| **Structural tuples** | Fixed arity 2–4, structural types, destructuring, and ARC-safe managed elements |
| **Typed variadics** | Final `...items: T` parameter receives a typed rest `List[T]`; no native C varargs |
| **Borrowed slices** | Zero-copy `StrSlice` / `Slice[T]` stack views with bounds and escape validation |
| **Async tasks** | `async def`, postfix `.await`, and a deterministic single-thread run-to-completion executor |
| **LSP Language Server** | `lpp-lsp` stdio JSON-RPC server for editor Intellisense, hovers & diagnostics |
| **Package manager** | Reliable Rust PM by default; experimental self-hosted pure-L++ PM via `LPP_SELF_HOSTED_PM=1` |
| **c2lpp package** | Pure-L++ C-header binding/package generator plus experimental scalar C-to-L++ translation |
| **100+ builtins** | strings, lists, maps, files, network, JSON, buffers |
| **Native performance** | Cranelift-first AOT compilation, with optional LLVM optimization and reproducible benchmark snapshots |
| **Compact native output** | Direct ELF / PE / Mach-O linking, including freestanding runtime configurations |

## Install

```bash
# Linux / macOS — pinned, reviewable source installation
git clone --depth 1 --branch v0.1 https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
LPP_FROM_SOURCE=1 sh install.sh

# Windows PowerShell — from the same pinned checkout
git clone --depth 1 --branch v0.1 https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
$env:LPP_FROM_SOURCE = "1"
.\install.ps1
```

Prebuilt archives and their `SHA256SUMS` manifest are published on the
[Releases](https://github.com/samarnever-droid/lplusplus/releases) page. The
installers refuse release archives that do not have a matching checksum. Do not
pipe a downloaded script directly into a shell. Development builds include only
the host Cranelift ISA by default; add `--features all-arch` when building a
cross-target compiler.

## Quick Start

```bash
# Create a project
lpp new myapp && cd myapp

# Edit src/main.lpp
cat > src/main.lpp << 'EOF'
def main():
    print_str("Hello from L++!")
    print(42)
EOF

# Build and run
lpp build && lpp run
```

## Language Overview

### Variables & Types

```lpp
x := 42              # immutable Int (inferred)
mut y := 10           # mutable
name := "Alice"       # Str
pi := 3.14159         # Float
flag := true          # Bool
```

### Functions

```lpp
def add(a: Int, b: Int) -> Int:
    return a + b

def greet(name: Str):
    print_str(str_concat("Hello, ", name))
```

### Tuples, Variadics, Slices, and Async Tasks

```lpp
def summarize(prefix: Str, ...values: Int) -> (Int, Str):
    mut total := 0
    for value in values:
        total = total + value
    return (total, prefix)

async def work() -> Str:
    return "Bhopal"

async def main():
    city := work().await
    (total, label) := summarize(city, 10, 20, 30)
    view := str_slice(label, 1, 3)       # borrowed, zero-copy
    owned := str_slice_to_str(view)      # explicit allocating escape
    print(total)
    print(owned)
```

Tuple arity is 2–4. A rest parameter must be final and is a safe typed list, not
C `...`. Borrowed views cannot be returned, captured, stored in owning
containers, reassigned through their source, or sent to a thread. Async uses a
single-thread run-to-completion executor; blocking file/network/process calls
without a readiness adapter are rejected from async call graphs.

These four features are an experimental first tier while Windows runtime
execution is still awaiting a real Windows CI run. They are not a claim of
project-wide “100% feature freeze.”

### Structs

```lpp
struct Point:
    x: Int
    y: Int

p := Point(10, 20)
print(p.x)
```

### Enums + Match

```lpp
enum Result:
    Ok(value: Int)
    Err(code: Int)

def safe_divide(a: Int, b: Int) -> Result:
    if b == 0:
        return Result.Err(1)
    return Result.Ok(a / b)

def main():
    match safe_divide(10, 3):
        Ok(v):
            print(v)
        Err(c):
            print_str("error")
```

### Error Propagation (`?` operator)

```lpp
def process(x: Int) -> Result:
    v := might_fail(x)?     # returns Err automatically if failed
    return Result.Ok(v + 1)
```

### Multi-file Imports

```lpp
import math                    # loads math.lpp
import utils.helpers           # loads utils/helpers.lpp
from stdlib.math import abs, pow   # selective import
```

### Collections

```lpp
# Lists
mut lst := list_new()
list_push(lst, 10)
list_push(lst, 20)
print(list_get(lst, 0))    # 10

# Maps
m := map_new()
map_put(m, 1, 100)
print(map_get(m, 1))       # 100
```

### Generics

```lpp
def identity[T](x: T) -> T:
    return x

struct Box[T]:
    value: T

print(identity(42))         # 42
print(identity("hello"))    # hello
```

### Traits & Dynamic Dispatch

```lpp
trait Speak:
    def speak(self) -> Int

struct Dog:
    name: Str

impl Speak for Dog:
    def speak(self) -> Int:
        print(1)
        return 1

# Accepts any Speak implementor (dynamic dispatch)
def make_speak(animal: Speak) -> Int:
    return animal.speak()
```

### FFI / Calling C Libraries

```lpp
extern "C" link "SDL2":
    def SDL_Init(flags: Int) -> Int
    def SDL_CreateWindow(title: Str, x: Int, y: Int, w: Int, h: Int, flags: Int) -> Int
    def SDL_Quit() -> Void
```

### Closures & Threads

```lpp
adder := fn(x: Int) -> Int:
    return x + 10

print(adder(5))   # 15

spawn fn():
    print_str("running in thread")
```

## Compiler Pipeline

```
Source (.lpp)
    │
    ├── Lexer → Tokens
    ├── Parser → AST
    ├── Semantic Analysis → Scopes, Bindings
    ├── Type Checker → Type Resolution
    ├── MIR Lowering → explicit tuple/slice/task/rest operations
    ├── Borrow Validator → first-tier slice non-escape rules
    ├── MIR Escape Solver → Frame / Owned / Shared facts
    │   ├── ARC Pass (retain/release insertion)
    │   ├── Closure Lifting
    │   ├── Peephole Optimization
    │   ├── Constant Propagation
    │   ├── Inlining
    │   ├── Dead Code Elimination
    │   ├── Copy Propagation
    │   ├── Strength Reduction
    │   ├── Branch Optimization
    │   └── ARC Pass (retain/release insertion)
    ├── Cranelift Codegen (default) or LLVM Codegen (optional)
    │       → Native Object (.o/.obj)
    └── Host linker or lpp-link → Executable (ELF/PE/Mach-O)
```

### Scalability controls

The Cranelift backend lowers and emits functions in **bounded batches** so its
peak memory is independent of total program size — it scales from tiny embedded
programs to very large codebases without holding the whole program's IR and
machine code in memory at once. Byte output is identical regardless of batch
size; only peak memory and wall time change.

| Env var | Effect |
| --- | --- |
| `LPP_CODEGEN_BATCH` | Functions per codegen batch. Lower values reduce peak memory; the default is 256. |
| `LPP_CODEGEN_THREADS` | Codegen worker threads. `1` forces the serial path (lowest overhead); the default uses available cores. |

## Benchmark Results (BPW v3)

| Benchmark | L++ | Rust | Go | L++ Binary | Go Binary |
|-----------|-----|------|-----|-----------|----------|
| CPU-Heavy (fib40+primes) | 4ms | 3ms | 5ms | **47KB** | 2345KB |
| RAM-Heavy (500k list) | 3ms | 2ms | 5ms | **47KB** | 2345KB |
| File I/O (400KB) | **1ms** | 6ms | 5ms | **47KB** | 2470KB |
| Win PE binary | — | — | — | **15.5KB** | — |

## Android & Termux

L++ targets Android and Termux via `--target <triple>`. Termux is a normal
aarch64/armv7 Linux build; Android uses the same ELF format and bionic libc.

```sh
# native build on a Termux device
cargo build --release --bin lpp --bin lpp-link
./target/release/lpp hello.lpp && ./hello

# cross-compile for Android arm64 / Termux 64-bit from a desktop
./target/release/lpp app.lpp --target aarch64-linux-android --emit-object
./target/release/lpp --list-targets
```

Set `ANDROID_NDK_HOME`/`ANDROID_NDK_ROOT` (or `ANDROID_CC`/`LPP_CC`) to link
Android objects with the NDK clang; `-DLPP_ANDROID` routes runtime output to
logcat, while Termux uses normal stdout. See
[`documentation/ANDROID_TERMUX.md`](documentation/ANDROID_TERMUX.md).

## Project Structure

```
src/
  frontend/     Lexer, Parser, AST
  analysis/     Semantic, Typecheck, Monomorphization, Cycle Breaker
  mir/          MIR IR, Builder, Escape Solver, ARC and cleanup passes
  backend/      Cranelift default + optional LLVM object backend
  bin/          lpp-link (ELF/PE/Mach-O direct linker)
  config.rs     User config (~/.lpp/config.json)
  builtins.rs   91 builtin function declarations
  target.rs     --target triple parsing + Android/Termux detection
  pm.rs         Package manager backend
  main.rs       CLI entry point

stdlib/         Pure L++ standard library
  math.lpp      abs, min, max, pow, gcd, fib, factorial
  strings.lpp   str_repeat, str_contains, str_reverse
  collections.lpp  list_sum, list_max, list_reverse
  algo.lpp      bubble_sort, binary_search
  result.lpp    Result, Option enums + helpers
  convert.lpp   int_to_str, bool_to_str

packages/       Published packages
  lpp-zip/      ZIP archive library (pure L++)

runtime/        Platform runtimes
  lpp_runtime.c           Host runtime (libc)
  windows_x86_64_min.c    Windows freestanding (Kernel32 only)
  linux_x86_64_min.c      Linux freestanding (syscalls only)
```

## CI Status

| Job | What it tests |
|-----|--------------|
| **king20-smoke** | 20 benchmark programs + stdlib + module imports + zip library |
| **scalability** | 10K/50K/100K line compile scaling |
| **ownership-and-parity** | ARC ownership verification suite |
| **windows-coff-fallback** | Windows PE direct linker + King20 PE gate |
| **macos-host-link** | macOS Mach-O compilation |

## License

Dual-licensed under either of:

- **MIT License** ([`LICENSE-MIT`](LICENSE-MIT))
- **Apache License, Version 2.0** ([`LICENSE-APACHE`](LICENSE-APACHE))

at your option.

## Links

- [Website](https://lplusplus.bond)
- [Language Guide](Doc.md)
- [Package Registry](https://registry.lplusplus.bond)
- [Benchmarks](benchmarks/king20/stable/v1/latest.md)
- [Native Linker Roadmap](documentation/Native_Linker_Roadmap.md)
- [Safety Mission](documentation/Safety_Mission.md)
