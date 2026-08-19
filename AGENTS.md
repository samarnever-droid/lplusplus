<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **lplusplus** (21104 symbols, 74797 relationships, 300 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> Index stale? Run `node .gitnexus/run.cjs analyze` from the project root.

## Always Do
- **MUST run impact analysis before editing any symbol.** Run `impact({target: "symbolName", direction: "upstream"})` and report blast radius.
- **MUST run `detect_changes()` before committing** to verify your changes only affect expected symbols.
- Use `query({search_query: "concept"})` and `context({name: "symbolName"})` for code navigation.
<!-- gitnexus:end -->

---

# L++ (LPlusPlus) v4.8.0 — Complete AI Architecture & Language Specification

> **For any AI Assistant / Agent**: This document is the single, authoritative, complete specification of the **L++ programming language**. When generating, debugging, or analyzing L++ code, **strictly follow the syntax, semantics, standard library, and architecture detailed below.**

---

## 1. What is L++?

**L++ (LPlusPlus)** is an ultra-high-performance, compiled, systems and application programming language designed for sub-millisecond compilation, deterministic compile-time memory safety, and cross-platform native execution.

### Key Architectural Pillars:
- **Instant Compilation Speed**: Compiles complete native standalone executables in **2 to 5 milliseconds** via Cranelift AOT and the direct built-in linker (`lpp-link`).
- **Deterministic Memory Safety**: Zero Garbage Collection pauses. Uses **Compile-Time Automatic Reference Counting (ARC)** + **Static SCC Cycle-Breaking (`cyclebreak`)** + **Single-Thread Atomic Elision (`pass_arc_local`)** + **Move-Out Retain Elision (`pass_moveout`)**.
- **Dual Compilation Pipeline**:
  - **Cranelift Backend**: Default for instant development iteration (<5ms builds).
  - **LLVM Backend (`--opt`)**: For maximum production optimization (`-O3`, AVX2 SIMD vectorization).
  - **WebAssembly Backend (`--target wasm32`)**: Compiles directly to clean, validated `.wasm` modules.
- **Zero-Dependency Native Linker (`lpp-link`)**: Built-in PE/COFF (Windows), ELF (Linux), and Mach-O (macOS) linker that generates direct native executables without requiring MSVC Build Tools or GNU `ld`.
- **First-Class Desktop WebViews**: Native in-process embedded windowing (Microsoft Edge WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux) for building modern desktop applications (Tauri/Wry architecture) in pure L++.

---

## 2. CLI Toolchain Reference

```bash
# Compilation & Execution
lpp main.lpp                       # Compile and link directly to main.exe (or ./main)
lpp main.lpp -o myapp.exe          # Specify output executable name
lpp run main.lpp                   # Compile, link, and run immediately
lpp main.lpp --opt                 # Use LLVM backend with -O3 optimization
lpp main.lpp --target wasm32       # Compile to WebAssembly (output .wasm)
lpp main.lpp --linker host         # Use host C compiler (cl.exe / cc) for external C linking
lpp main.lpp -l sqlite3            # Link external C library

# Package Manager (lpp-pm)
lpp init my-app                    # Initialize a new L++ project
lpp add <pkg>                      # Add a dependency from official registry
lpp install                        # Resolve and install all dependencies in lpp.toml
lpp build                          # Build project defined in lpp.toml
lpp test                           # Run all tests in tests/
lpp publish                        # Publish package to official registry
```

---

## 3. Complete Syntax & Language Reference

### 3.1. Comments & Variables
```lpp
# Single-line comment starts with '#'

# Variable assignment & type inference (':=' declares a new local)
x := 42                            # Inferred as Int (i64)
pi := 3.14159                      # Inferred as Float (f64)
greeting := "Hello, L++!"          # Inferred as Str (ARC-managed heap string)
is_active := true                  # Inferred as Bool (i8)
ch := 'A'                          # Inferred as Char (Unicode codepoint)

# Explicit type annotation
count: Int = 100
name: Str = "Samar"

# Type Aliases (First-class compiler support)
type UserID = Str
type Milliseconds = Int

id: UserID = "usr_99812"
```

### 3.2. Primitive Types
| Type | Description | Memory / ABI Class |
| :--- | :--- | :--- |
| `Int` | 64-bit signed integer | `i64` |
| `Float` | 64-bit IEEE-754 double precision float | `f64` |
| `Str` | Immutable UTF-8 ARC-owned heap string | 24-byte ARC header + buffer |
| `Bool` | Boolean (`true` / `false`) | `i8` |
| `Char` | Unicode character | `i64` |
| `Void` | Unit / no return value | `void` |
| `StrSlice` | Zero-copy borrowed string view | 16-byte pointer + length |
| `VectorI64x2` | 128-bit SIMD vector (2x i64) | 16-byte aligned register |

### 3.3. Functions & Methods
```lpp
# Standard function
def add(a: Int, b: Int) -> Int:
    return a + b

# Function with default parameters
def connect(host: Str, port: Int = 8080, timeout_ms: Int = 5000) -> Bool:
    print_str("Connecting to " + host)
    return true

# Variadic parameters (collected into List[T])
def sum_all(*numbers: Int) -> Int:
    total := 0
    for n in numbers:
        total = total + n
    return total

# Generic function (with optional Turbofish syntax)
def identity[T](val: T) -> T:
    return val

val := identity::<Int>(42)
```

### 3.4. Structs & Implementations
```lpp
# Struct definition
struct Point:
    x: Float
    y: Float

struct User:
    id: Int
    username: Str
    tags: List[Str]

# Implementation block
impl Point:
    def magnitude(self) -> Float:
        return sqrt(self.x * self.x + self.y * self.y)
        
    def translate(self, dx: Float, dy: Float) -> Point:
        return Point(self.x + dx, self.y + dy)

# Instantiation (Positional or Named)
p1 := Point(3.0, 4.0)
p2 := Point { x: 10.0, y: 20.0 }
dist := p1.magnitude()
```

### 3.5. Enums & Pattern Matching
```lpp
# Enums can carry payloads (Algebraic Data Types)
enum WebEvent:
    PageLoad
    KeyPress(Int)
    Click(Int, Int)
    Message(Str)

# Pattern matching is exhaustive
def handle_event(evt: WebEvent):
    match evt:
        WebEvent.PageLoad =>
            print_str("Page loaded")
        WebEvent.KeyPress(key) =>
            print_str("Key pressed: " + int_to_str(key))
        WebEvent.Click(x, y) =>
            print_str("Clicked at: " + int_to_str(x) + ", " + int_to_str(y))
        WebEvent.Message(msg) =>
            print_str("Message received: " + msg)
```

### 3.6. Traits & Polymorphism
```lpp
trait Renderable:
    def render(self) -> Str

impl Renderable for Point:
    def render(self) -> Str:
        return "(" + float_to_str(self.x) + ", " + float_to_str(self.y) + ")"

# Trait bound on generics
def draw[T: Renderable](item: T):
    print_str("Drawing: " + item.render())
```

### 3.7. Collections: Lists & Maps
```lpp
# Lists (Dynamic, type-safe arrays)
numbers := [10, 20, 30, 40]
numbers.push(50)
first := numbers[0]
count := numbers.len()

# Maps (Hash tables)
m := map_new()
map_set(m, "theme", "dark")
map_set(m, "lang", "en")

if map_contains(m, "theme"):
    current_theme := map_get(m, "theme")
    print_str("Current theme: " + current_theme)

# Tuples
pair := (100, "Success")
code := pair.0
status := pair.1
```

### 3.8. Control Flow
```lpp
# If / Elif / Else
if score >= 90:
    print_str("Grade: A")
elif score >= 80:
    print_str("Grade: B")
else:
    print_str("Grade: C")

# For in Range
for i in 0..5:
    print(i)                       # Prints 0, 1, 2, 3, 4

# For in List
items := ["apple", "banana", "cherry"]
for item in items:
    print_str(item)

# While loop
mut count := 3
while count > 0:
    print(count)
    count = count - 1
```

### 3.9. Concurrency: Async / Await
```lpp
# Asynchronous tasks
async def fetch_data(source_id: Int) -> Str:
    sleep(50)                      # Async non-blocking sleep
    return "Data from source " + int_to_str(source_id)

def main():
    # Spawn background concurrent tasks
    task1 := spawn fetch_data(1)
    task2 := spawn fetch_data(2)

    # Await completion
    res1 := await task1
    res2 := await task2
    print_str("Results: " + res1 + " | " + res2)
```

### 3.10. FFI (Foreign Function Interface)
```lpp
# Declare C symbols
extern "C":
    def abs(x: Int) -> Int
    def puts(s: Str) -> Int
    def MessageBoxA(hwnd: Int, text: Str, caption: Str, utype: Int) -> Int

def main():
    val := abs(-100)
    puts("Direct C output")
    # Windows native popup
    MessageBoxA(0, "Running from pure L++ Direct Native!", "L++ FFI", 0)
```

---

## 4. Builtin Standard Library Functions

### 4.1. Printing & Output
- `print(x: Int)` — Print integer + newline.
- `print_str(s: Str)` — Print string + newline.
- `print_float(f: Float)` — Print float + newline.
- `print_bool(b: Bool)` — Print boolean (`true`/`false`) + newline.

### 4.2. String Operations
- `str_len(s: Str) -> Int`
- `str_concat(a: Str, b: Str) -> Str` (or `a + b`)
- `str_contains(haystack: Str, needle: Str) -> Bool`
- `str_starts_with(s: Str, prefix: Str) -> Bool`
- `str_ends_with(s: Str, suffix: Str) -> Bool`
- `str_find(haystack: Str, needle: Str) -> Int`
- `str_replace(s: Str, target: Str, replacement: Str) -> Str`
- `str_trim(s: Str) -> Str`
- `str_to_lower(s: Str) -> Str`
- `str_to_upper(s: Str) -> Str`
- `int_to_str(n: Int) -> Str`
- `str_to_int(s: Str) -> Int`
- `float_to_str(f: Float) -> Str`
- `bool_to_str(b: Bool) -> Str`

### 4.3. Filesystem & OS
- `read_file(path: Str) -> Str`
- `write_file(path: Str, content: Str) -> Int`
- `append_file(path: Str, content: Str) -> Int`
- `file_exists(path: Str) -> Bool`
- `dir_create(path: Str) -> Bool`
- `dir_list(path: Str) -> List[Str]`
- `env_get(key: Str) -> Str`
- `env_set(key: Str, val: Str) -> Bool`
- `exit(code: Int)`
- `time_ms() -> Int`
- `sleep(ms: Int)`

### 4.4. Math & Random
- `abs(x: Int) -> Int`
- `min(a: Int, b: Int) -> Int`
- `max(a: Int, b: Int) -> Int`
- `floor(x: Float) -> Float`
- `ceil(x: Float) -> Float`
- `pow(base: Float, exp: Float) -> Float`
- `sqrt(x: Float) -> Float`
- `random() -> Int`
- `random_range(lo: Int, hi: Int) -> Int`

### 4.5. Native Desktop WebView Windowing (SamarBook / GUI Architecture)
- `webview_window_create(title: Str, width: Int, height: Int, flags: Int) -> Int`
- `webview_navigate(handle: Int, url_or_path: Str)`
- `webview_set_html(handle: Int, html_content: Str)`
- `webview_run(handle: Int)` — Runs native OS event loop until window closes.
- `webview_terminate(handle: Int)`
- `webview_destroy(handle: Int)`

---

## 5. Building an App in L++ (Project Blueprint)

### 5.1. Project Structure
```
my-app/
├── lpp.toml               # Package manifest
├── lpp.lock               # Deterministic dependency lockfile
├── src/
│   ├── main.lpp           # Main entry point (def main())
│   ├── app.lpp            # Business logic / services
│   └── models.lpp         # Data structures & structs
├── ui/                    # (Optional) HTML/JS/React/TanStack assets for WebView apps
│   ├── index.html
│   └── assets/
└── tests/
    └── test_main.lpp      # Unit tests
```

### 5.2. `lpp.toml` Manifest
```toml
[package]
name = "my-app"
version = "0.1.0"
authors = ["Samar <samar@lplusplus.bond>"]
description = "High-performance native application in pure L++"
license = "MIT"

[dependencies]
lpp-json = "1.2.0"
lpp-math = "0.4.0"
```

### 5.3. Native WebView Desktop Application Template
```lpp
# src/main.lpp
def main():
    print_str("Starting L++ Desktop Application...")
    
    # 1. Create in-process native OS WebView window (WebView2 / WKWebView / WebKitGTK)
    wv := webview_window_create("My L++ Application", 1280, 800, 0)
    if wv < 0:
        print_str("Error: Failed to initialize native window")
        return

    # 2. Load bundled UI or URL
    webview_navigate(wv, "ui/index.html")

    # 3. Enter native event loop
    webview_run(wv)
    
    # 4. Clean up on exit
    webview_destroy(wv)
    print_str("Application closed cleanly.")
```

---

## 6. Rules for AI Generating L++ Code
1. **Always provide `def main():`** as the program entry point in root files.
2. **Use `:=` for declaring new variables** with inferred types, and `=` for reassigning existing mutable variables.
3. **Strings are strongly-typed `Str`**: Use `print_str(msg)` to print strings and `print(n)` to print integers.
4. **Always use exact L++ types**: `Int`, `Float`, `Str`, `Bool`, `Char`, `Void`, `List[T]`, `Map[K, V]`, `(T1, T2)`.
5. **No raw memory pointers or manual freeing needed**: L++ automatically manages memory with static ARC and compile-time cycle breaking.
6. **For desktop GUIs**: Use the built-in `webview_*` API to deliver in-process native windowing with zero runtime dependencies.
