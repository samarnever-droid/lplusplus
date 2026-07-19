# L++ Language Documentation

L++ is an experimental prototype language aiming to be as readable as Python, as fast as C, and as safe as Rust. Its primary goal is to abstract away memory management without exposing a borrow checker or relying on a Tracing Garbage Collector (GC). 

**Current Reality:** L++ is an experimental compiled-language toolchain with a verified Linux x86-64 Cranelift AOT subset, a direct ELF linker path, and a C compatibility backend. The ownership model is stronger than a parser-only prototype: it has ownership-aware MIR, ARC allocation, `Borrow`/`Move`/`ReturnOwned`, generated destructor chains, closure capsules, `List[Int]`, `List[Custom]`, and strong-cycle rejection. It is still not a complete Rust-equivalent guarantee across every platform and library feature. Files, networking, threads, JSON, writable direct-link data, Windows PE, macOS Mach-O, and full C/AOT semantic parity remain in progress. Read `documentation/Cranelift_Ownership_Audit_2026-07-19.md`, `documentation/Native_Linker_Roadmap.md`, and `documentation/Windows_Native_Toolchain.md` for current boundaries.

This guide breaks down the current working state of L++ and explains exactly how the compiler manages your memory under the hood.

---

## Current verified capability matrix

| Area | Verified now | Explicit boundary |
|---|---|---|
| Cranelift AOT | Linux x86-64, COFF object generation on Windows | Direct PE/Mach-O output is not implemented yet. |
| Ownership | ARC structs, aliases, owned/borrowed returns, closures, lists | Strong cycles are rejected until `Weak`/arena/cycle-collection support exists. |
| Direct linker | `.text`, `.rodata`, GOT imports, ARC, closures, lists, King20 20/20 | File I/O, networking, threads, JSON, `.data`/`.bss`, and dynamic imports use fallback linking. |
| C backend | Compatibility/debug path and observable parity suite | It is not the authoritative ownership implementation. |
| Windows | COFF + `lpp_runtime.obj` + MSVC host-link fallback | Direct PE runtime and King20 gate are in progress. |
| macOS | Mach-O object + clang host-link fallback | Direct Mach-O work is planned; Intel and Apple Silicon release packaging is added. |

### Ownership vocabulary

The compiler uses these internal concepts; they are not syntax users must write:

```text
AllocateArc   create one owned ARC reference
Borrow        read an existing owner without transfer
Move          transfer a temporary owner
Retain        create another owner from a borrow
Release       remove an owner
ReturnOwned   transfer one owner to the caller
```

A normal user program stays concise:

```lpp
def identity(value: Box) -> Box:
    return value
```

Internally, a borrowed parameter return is lowered as:

```text
retain(value)
return_owned(value)
```

---

## 1. Syntax Basics and Significant Whitespace

L++ uses significant whitespace (indentation) and colons (`:`) to define blocks, just like Python. This forces clean, readable code and eliminates the need for curly braces.

```lpp
def main():
    print("Hello, World!")
```

## 2. Mutability: Shadowing vs Mutation

By default, all variables in L++ are **immutable**. This eliminates an entire class of concurrency and logic bugs.

In L++, you declare a **new** variable using `:=`. You mutate an **existing** variable using `=`.

```lpp
def calculate():
    x := 5
    # x = 6  <-- ERROR: x is immutable by default

    mut y := 10
    y = 20   # OK: y was declared as mut
```

**Shadowing:** 
Each declaration creates a completely new lexical binding. You can declare a *new* variable with the same name in the same or nested scope using `:=`. This does not mutate the old variable; it safely shadows it, with zero risk of collision!

```lpp
def greet():
    prefix := "Hello"
    prefix := "Goodbye" # Valid! This creates a new, distinct binding, shadowing the old one.
```

## 3. Functions and Explicit Signatures

To reduce compile-time complexity through explicit interfaces, L++ requires explicit types on function parameters and return types. The compiler uses these to type-check without needing to heavily analyze the entire call graph.

```lpp
def add(a: Int, b: Int) -> Int:
    return a + b
```

If a function doesn't return anything, you can omit the arrow or explicitly return `Void`. Local variables inside the function body are automatically type-inferred!

## 4. Structs

Structs define custom data types. You do not need to worry about memory allocation modifiers (like `Box` or `&`). The compiler will automatically use Value Semantics (stack allocation) where possible, and promote to Managed Heap if the struct is self-referential or escapes its scope.

```lpp
struct Node:
    value: Int
    next: Node  # The compiler detects self-reference and automatically uses Arenas/Managed Heap!
```

## 5. Closures

Closures in L++ use the `fn` keyword. Because they are often short-lived, parameter and return types can be inferred automatically.

**Inline Closures:**
```lpp
def process():
    map(items, fn(x) -> x * 2)
```

**Block Closures:**
```lpp
def process():
    callback := fn(x):
        mut y := x * 2
        return y + 1
```

Closures with immutable captures are experimental. Mutable capture-by-reference and safe cross-thread closure transfer are not implemented in the Cranelift backend; unsupported AOT cases are rejected rather than silently compiled with changed semantics.

## 6. Primitives (The `Int` Type) and Scalar Values

In L++, `Int` represents a primitive integer. It is a **scalar value**, meaning it is purely data with no internal pointers or references.

Because `Int` (and other scalars like `Bool`) are just flat data, they are exclusively stack-allocated and passed around by **copy**. The compiler's escape analyzer knows that returning an `Int` or passing it to another function is completely safe and carries no risk of dangling references. Therefore, a scalar like `Int` will **never** trigger heap promotion (Managed Heap or Arena) on its own unless it is mutable and captured across a concurrency boundary. 

This behavior is especially powerful when interacting with structs:

```lpp
struct Box:
    inner: Node
    count: Int

def safe_return() -> Int:
    my_box := Box()
    return my_box.count # SAFE: 'count' is an Int (scalar). It gets copied out.
                        # 'my_box' is safely destroyed at the end of the scope.
```

By understanding that `Int` operates purely by value, you can confidently write code that extracts and returns data from objects without forcing those objects to live on the heap.

---

## 7. The Magic of L++: Escape Analysis

The core innovation of L++ is its **Hybrid Memory Model**. You never write pointer or allocation modifiers (like `&`, `*`, `Box`, `Rc`, or `Arc`). You write simple, Python-like code, and the compiler performs a semantic pass called **Escape Analysis** to figure out how to optimally allocate your memory.

### How it Works

Every variable starts its life as a **Value** (Stack-allocated). Stack allocation is zero-cost and blazing fast.

The compiler then checks a series of rules. If a variable breaks a rule, its storage is monotonically "promoted" to the Heap. The Escape Analyzer produces a side-table mapping every variable to its required storage class. The codegen backend uses this table to automatically manage memory (using managed heap storage or arenas) in the generated binary.

### Summary of Storage Classes
- **Value**: Stack-allocated. The default, zero-cost abstraction.
- **Managed Heap (ARC)**: Heap-allocated storage managed automatically via **Automatic Reference Counting (ARC)**. The compiler automatically prepends a reference count header (`LppArcHeader`) containing an atomic count to every allocated object, incrementing (`retain`) or decrementing (`release`) it as references are passed around or go out of scope, and freeing it when the count hits zero.
- **Arena**: Specialized high-performance heap allocation for recursive structures.

### The Promotion Rules

*L++'s design specifies six promotion rules in total. Rule 6 (required aliasing) depends on further language features that haven't been added yet, and will be documented here once implemented.*

1. **Rule 1: Returned by Reference**
   If a local variable or field reference is returned to the caller, it "escapes" its original stack frame. The compiler detects this and promotes the base object to **Managed Heap** storage.
   ```lpp
   struct Item:
       value: Int

   def create_item() -> Item:
       item := Item()
       return item # item is promoted to Managed Heap storage!
   ```
   *(Note: Returning a computed primitive value or field like `return box.count` does NOT promote the base object, because you are returning a copy of a primitive scalar rather than returning a reference to the escaping object storage.)*

2. **Rule 2: Closure Capture**
   If a reference is captured by an escaping closure, it escapes. Any custom struct captured will be promoted to **Managed Heap** storage. Immutable scalar primitives are cloned by value safely onto the stack! 
   ```lpp
   def process():
       multiplier := 5
       callback := fn(x) -> Int:
           return x * multiplier # multiplier is an immutable scalar, safely cloned by value!
   ```

3. **Rule 3: Unbounded-Lifetime Containers**
   If a reference is inserted into a heap-allocated container (like a `List`), it escapes because the container's lifetime is unbounded and dynamic.
   ```lpp
   def build_list() -> Void:
       node := Node()
       my_list := [node] # node is promoted to Managed Heap because it is stored in a list!
   ```

4. **Rule 4: Concurrency Boundary**
   This is a language-design rule, not yet a completed direct-AOT promise. The Cranelift backend currently rejects `spawn` because safe thread transfer needs an explicit ownership and synchronization contract for the closure environment. L++ will add this only when moved, shared, and atomic capture modes are defined and tested.

   ```text
   Current AOT behavior: reject unsafe spawn transfer clearly.
   Future behavior: explicit compiler-selected move/share contract.
   ```

5. **Rule 5: Self-Referential Structs (Arenas)**
   If a struct contains a field of its own type (like a Linked List `Node`), the compiler detects this self-reference at the type level and automatically promotes instances of this struct to an **Arena**. 
   Arenas are incredibly fast bulk-allocators used for graph-like data structures.

L++ handles all of this invisibly, leaving you to focus entirely on your business logic.

---

## 8. Current Features & Capabilities

Because L++ is an active prototype, only a subset of planned language features is currently parsed and transpiled. 

#### Language Features
- **Data Types:** `Int` (64-bit), `String`, `Void`, and custom `struct` definitions.
- **Variables & Mutability:** `:=` for initialization, `=` for assignment. `mut` keyword for mutable state.
- **Functions:** `def` with typed arguments and return types.
- **Closures:** Inline and block closures using `fn`, with lexical closure capture.
- **Math Operations:** Basic arithmetic (`+`, `-`, `*`, `/`, `%` modulo).
- **Data Structures:** 
  - Struct instantiation and field access (`obj.field`).
  - Heap-allocated Lists using square brackets (`[1, 2, 3]`).
  - Dynamic Lists via standard library built-ins.
- **Concurrency:** `spawn` is parsed, but direct AOT currently rejects it until closure transfer ownership is complete.
- **Custom Local Libraries:** Relational module merging imports (e.g. `import math_helper` merges source elements).

### Control Flow
- **If / Else**: Fully implemented for branching logic (`if x == 10: ... else: ...`).
- **While Loops**: Fully implemented for conditional repetition (`while i < 10: ...`).
- **Relational Operators**: `==`, `!=`, `<`, `>`, `<=`, `>=` all return `Bool`.
*(Note: `for` loops are currently under development and will require iterator protocols).*

### Standard Library (Built-ins)
L++ provides a growing set of built-in functions for common operations, which map directly to optimal C stdlib calls:
- **Console I/O**: `print(value)` (prints strings or integers via automatic format selection), `print_str("string")`, `input()` (reads line from stdin), `parse_int("string")` (parses a string into an integer)
- **File I/O**: `read_file("path")` (returns string), `write_file("path", "data")`
- **Dynamic Lists**:
  - `list_new()`: Creates a list with an inferred supported element type.
  - `list_push(list, value)`: Appends an element to the list, automatically retaining ARC-managed custom elements.
  - `list_get(list, index)`: Retrieves a borrowed element; assigning or returning an ARC element creates the required retain.
  - `list_len(list)`: Returns the current number of elements in the list.
  - **Supported AOT list types:** `List[Int]` and `List[Custom]`.
  - **AOT ownership mode:** lists are released automatically at scope exit. Do not call `list_free` in Cranelift AOT code; it is rejected to prevent double release. The legacy C compatibility backend still exposes it.
- **JSON Parsing**:
  - `json_parse("json_string")`: Parses a JSON string and returns a node handle (`Int`).
  - `json_get_int(node, "key")`: Retrieves an integer property value.
  - `json_get_str(node, "key")`: Retrieves a string property value.
  - `json_get_obj(node, "key")`: Retrieves a nested JSON object node handle (`Int`).
  - `json_free(node)`: Recursively frees the parsed JSON tree memory.
- **Networking**:
  - `net_connect(host, port) -> Int`: Opens a TCP client connection and returns a socket handle.
  - `net_listen(port) -> Int`: Binds a TCP listener on the given port and returns a listener handle.
  - `net_accept(listener) -> Int`: Accepts one inbound client from a listener handle.
  - `net_send(socket, data) -> Int`: Compatibility API with complete-write semantics; returns all bytes or `-1`.
  - `net_send_all(socket, data) -> Int`: Safely retries partial native socket writes; returns all bytes or `-1`.
  - `net_set_timeout(socket, milliseconds) -> Int`: Applies native read/write deadlines; returns `1` or `0`.
  - `net_recv(socket, max_bytes) -> String`: Receives up to `max_bytes` and returns the payload as a string.
  - `net_close(handle) -> Void`: Closes a socket or listener handle.
  - Uses Winsock/POSIX sockets directly—never cURL. Full scope and roadmap: [`documentation/Networking.md`](documentation/Networking.md).

Example server:

```lpp
def main():
    listener := net_listen(9000)
    client := net_accept(listener)
    request := net_recv(client, 1024)
    print_str(request)
    net_send(client, "hello from lpp server")
    net_close(client)
    net_close(listener)
```

Example client:

```lpp
def main():
    socket := net_connect("127.0.0.1", 9000)
    net_send(socket, "ping")
    response := net_recv(socket, 1024)
    print_str(response)
    net_close(socket)
```

### Compiler Architecture & Backends

L++ is designed as a multi-tier compilation pipeline:
1. **Cranelift AOT Backend (Default / Native):** Converts L++ AST into Mid-level IR (MIR), performs an ARC insertion pass, and uses Cranelift to emit native machine code object files (`.o`). These are linked against the lean C runtime library ([lpp_runtime.c](C:/Users/khati/lpp/lpp_runtime.c)) using `link.exe`, `cl.exe`, `cc`, `gcc`, or `clang` depending on the host toolchain.
2. **C Transpiler:** Transpiles L++ directly into optimized C code, which can be compiled with standard GCC/Clang compilers.

**Performance Characteristics:**
- **Compile Time:** L++ compiles source to native machine code in **~3.0 ms** (Frontend + MIR + Cranelift AOT). Total compile time including linking is ~100–390 ms.
- **Execution Speed:** Native L++ executables run at optimized C/Rust speeds (e.g., recursive Fibonacci(35) takes ~64 ms, matching optimized C and running ~20x faster than Python).
- **Executable Size:** Native executables are extremely compact (~138 KB), requiring no bulky runtime VM or heavy standard library.

### Standard Library & Built-ins Status

| Category   | Current Support                 |
| ---------- | ------------------------------- |
| Console    | `print(...)`, `print_str(...)`  |
| Input      | `input()`, `parse_int(...)`     |
| Files      | `read_file`, `write_file`       |
| JSON       | Full (`json_parse`, `json_get_int`, `json_get_str`, `json_get_obj`, `json_free`) |
| Networking | Native TCP sockets; complete writes and deadlines (`net_connect`, `net_listen`, `net_accept`, `net_send_all`, `net_set_timeout`, `net_recv`, `net_close`) |
| Threads    | Parsed; direct AOT transfer is intentionally rejected pending ownership work |
| Lists      | ARC-managed `List[Int]` and `List[Custom]` in the verified AOT subset |
| Strings    | Basic output and literals; direct ELF supports `.rodata` literals |
| Structs    | ARC destructors, aliases, returns, and nested ownership in the verified subset |

---

## 9. Planned Features (Roadmap)

- Modules & Imports
- Generics & Interfaces
- Pattern Matching
- Package Manager
- Standard Library Expansion (`io`, `math`, `string`, `collections`)
- Async Runtime
- Rule 6 (Required Aliasing)

---

## 10. Toolchain & Installation

L++ provides a simple and premium toolchain wrapper to install and run the compiler globally.

### Global Installation

To install the L++ compiler globally on your system:

1. Open PowerShell and run the installer script from the project root:
   ```powershell
   .\install.ps1
   ```
2. The installer will:
   - Build the compiler binary in release mode (`lpp-compiler.exe`).
   - Create a global install directory at `%USERPROFILE%\.lpp`.
   - Copy the pre-compiled C runtime library (`lpp_runtime.obj`) to `%USERPROFILE%\.lpp\lib`.
   - Generate a global CLI wrapper (`lpp.bat`) at `%USERPROFILE%\.lpp\bin`.
   - Add the binary directory to your user `PATH` environment variable.
3. Restart your terminal or IDE, and you can now use the global `lpp` command directly!

### Using the Global Command

```cmd
# Show compiler version
lpp -v

# Show help menu
lpp -h

# Compile a L++ file into a native executable
lpp main.lpp
```

The global compiler automatically compiles the `.lpp` file to a native object file, invokes `link.exe` (auto-detecting your MSVC compiler environment), links it with the L++ runtime, and outputs a native executable (`main.exe`) while cleaning up any intermediate files!

### Local Development Runner

For quick local tests during development without installing globally, use the local runner script:

```powershell
# Compile, link, and run a L++ program instantly:
.\run.ps1 tests\fib.lpp
```
