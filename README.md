<p align="center">
  <img src="assets/lpp-logo.svg" width="190" alt="L++ four-pillar prism logo">
</p>

<h1 align="center">L++</h1>

<p align="center"><strong>Readable like Python · Safe like Rust · Fast like Go · Native by default</strong></p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="Doc.md">Language Guide</a> ·
  <a href="wiki/">Wiki</a> ·
  <a href="benchmarks/king20/stable/v1/latest.md">Benchmarks</a>
</p>

---

## What is L++?

L++ is a compiled, ownership-aware programming language that combines Python's readability with Rust's safety model and Go's compilation speed. It compiles to native executables via Cranelift AOT — no interpreter, no VM, no garbage collector.

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

## Key Features

| Feature | Description |
|---------|-------------|
| **Python-like syntax** | Significant whitespace, `:=` declarations, `def`/`struct`/`enum` |
| **Ownership & ARC** | Automatic reference counting, borrow tracking, cycle rejection |
| **Enums + match** | Algebraic data types with pattern matching and data extraction |
| **Error handling** | `Result` type + `?` operator for error propagation |
| **Multi-file modules** | `import math`, `from utils import calc`, dotted paths |
| **Native compilation** | Cranelift AOT → ELF (Linux) / PE (Windows) / Mach-O (macOS) |
| **Direct linker** | `lpp-link` produces standalone executables without `gcc`/`clang`/MSVC |
| **Package manager** | `lpp new`, `lpp build`, `lpp run` — self-hosted in L++ |
| **Standard library** | math, strings, collections, algorithms, zip archives |
| **15KB binaries** | Windows PE freestanding executables as small as 15.5KB |

## Install

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/install.ps1 | iex

# From source
git clone https://github.com/samarnever-droid/lplusplus.git
cd lplusplus && cargo build --release --bin lpp --bin lpp-link
```

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

def safe_divide(a: Int, b: Int) -> Int:
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
def process(x: Int) -> Int:
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
    ├── Escape Analysis → Ownership Classification
    ├── MIR Lowering → Mid-level IR
    │   ├── ARC Pass (retain/release insertion)
    │   ├── Closure Lifting
    │   ├── Constant Propagation
    │   ├── Dead Code Elimination
    │   ├── Branch Optimization
    │   ├── Peephole Optimization
    │   └── Inlining
    ├── Cranelift Codegen → Native Object (.o/.obj)
    └── lpp-link → Executable (ELF/PE/Mach-O)
```

## Benchmark Results (BPW v3)

| Benchmark | L++ | Rust | Go | L++ Binary | Go Binary |
|-----------|-----|------|-----|-----------|----------|
| CPU-Heavy (fib40+primes) | 4ms | 3ms | 5ms | **47KB** | 2345KB |
| RAM-Heavy (500k list) | 3ms | 2ms | 5ms | **47KB** | 2345KB |
| File I/O (400KB) | **1ms** | 6ms | 5ms | **47KB** | 2470KB |
| Win PE binary | — | — | — | **15.5KB** | — |

## Project Structure

```
src/
  frontend/     Lexer, Parser, AST
  analysis/     Semantic, Typecheck, Escape
  mir/          MIR IR, Builder, 7 optimization passes
  backend/      Cranelift AOT compiler
  bin/          lpp-link (ELF/PE/Mach-O direct linker)
  config.rs     User config (~/.lpp/config.json)
  builtins.rs   91 builtin function declarations
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

MIT

## Links

- [Language Guide](Doc.md)
- [Package Registry](https://samarnever-droid.github.io/lplusplus/registry/index.json)
- [Benchmarks](benchmarks/king20/stable/v1/latest.md)
- [Native Linker Roadmap](documentation/Native_Linker_Roadmap.md)
- [Safety Mission](documentation/Safety_Mission.md)
