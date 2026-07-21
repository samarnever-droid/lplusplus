# L++ Complete Language & Toolchain Specification

L++ is an experimental native programming language designed to combine the **readability of Python**, the **memory safety and control of Rust**, and the **build/iteration speed of Go**.

L++ features a multi-tier compilation pipeline: an ahead-of-time (AOT) Cranelift compiler producing native object files (`.o`/`.obj`), a custom tri-format direct linker (`lpp-link` for Linux ELF, Windows PE, and macOS Mach-O), a self-hosting package manager (`lpp-pm`), and an auxiliary C transpiler for debugging and cross-platform bootstrapping.

---

## 1. Syntax Reference & Language Basics

### 1.1 Significant Whitespace & Comments
L++ uses 4-space indentation and colons (`:`) to define blocks. Statements are terminated by newlines.
Line comments start with `#`.

```lpp
# This is a single-line comment
def main():
    print("Hello, L++!")
```

### 1.2 Variables, Mutability & Shadowing
Variables in L++ are **immutable by default**.
- **`:=` Declaration**: Defines a new variable in the current lexical scope.
- **`mut` Keyword**: Required to declare a variable as mutable.
- **`=` Assignment**: Mutates an existing `mut` variable.
- **Shadowing**: Re-declaring a name with `:=` in the same or nested scope creates a distinct lexical binding without mutating the previous binding.

```lpp
def demo_variables():
    # Immutable binding
    x := 10
    # x = 20  <-- COMPILE ERROR: x is immutable

    # Mutable binding
    mut y := 5
    y = 15    # OK: y was declared as mut

    # Lexical shadowing
    msg := "initial"
    msg := "shadowed" # OK: creates a new distinct scope binding
```

### 1.3 Data Types
L++ supports native primitives, custom structs, and parameterized lists (`List[T]`):

| Type | Syntax | Description |
|---|---|---|
| Integer | `Int` | 64-bit signed scalar integer (`int64_t`) |
| Float | `Float` | 64-bit double-precision scalar float (`double`) |
| Boolean | `Bool` | Boolean scalar (`true` or `false`) |
| String | `String` or `Str` | NUL-terminated UTF-8 byte string |
| Void | `Void` | Absence of return value |
| Custom Struct | `typename` | Heap/stack-allocated user type |
| Dynamic List | `List[T]` | Parameterized heap list (`T` = `Int`, `Float`, `Bool`, `Str`, `Custom`) |

---

## 2. Control Flow

### 2.1 Branching (`if` / `else`)
Branching evaluates boolean conditions:

```lpp
def check_score(score: Int) -> Void:
    if score >= 90:
        print_str("Grade: A")
    else:
        if score >= 70:
            print_str("Grade: B")
        else:
            print_str("Grade: C")
```

### 2.2 Loops (`while` and `for`)
L++ provides `while` loops, list iteration `for x in list:`, and integer range iteration `for i in range(n)` / `for i in range(start, end)`. `break` and `continue` control-flow statements are supported across all loop forms.

```lpp
def loop_demo():
    # Standard while loop with break & continue
    mut i := 0
    while i < 10:
        if i == 5:
            break
        i = i + 1

    # Range loop with continue
    mut sum := 0
    for k in range(0, 10):
        if k % 2 == 0:
            continue
        sum = sum + k

    # List iteration loop
    items := [10, 20, 30, 40]
    for item in items:
        if item == 30:
            break
        lpp_print_int(item)
```

---

## 3. Functions, Structs & Closures

### 3.1 Functions
Top-level functions are declared using `def`. Function signatures must explicitly declare parameter types and return types. Function body local variables are automatically type-inferred.

```lpp
def multiply(a: Int, b: Int) -> Int:
    return a * b

def greet(name: String):
    print_str(str_concat("Hello, ", name))
```

### 3.2 Structs
Structs define custom composite data structures. Structs can be instantiated with 0 arguments (empty initialization) or with positional constructor arguments corresponding to field definitions:

```lpp
struct Point:
    x: Int
    y: Int

def create_point() -> Point:
    # Direct positional constructor initialization
    p := Point(10, 20)
    return p
```

### 3.3 Closures
Closures are created using the `fn` keyword with automatic parameter and return type inference.

```lpp
def closure_demo():
    factor := 3
    # Anonymous closure capturing immutable scalar 'factor'
    triple := fn(x) -> x * factor
    result := triple(10) # 30
```

---

## 4. Memory Management & Escape Analysis

L++ employs a **Hybrid Memory Model** that combines stack allocation with Automatic Reference Counting (ARC) and Arena storage without exposing borrow-checker syntax or requiring a Tracing Garbage Collector (GC).

### 4.1 Storage Classes
1. **Value (Stack)**: Zero-cost, stack-allocated storage. The default for scalars and non-escaping structs.
2. **Managed Heap (ARC)**: Objects whose lifetimes escape their initial frame are allocated with an `LppArcHeader` prefix containing an atomic reference count. The compiler emits `retain` and `release` operations into MIR basic blocks and attaches auto-generated recursive destructor chains (`lpp_drop_<struct>`) for cleaning up fields.
3. **Arena**: Self-referential or recursive structs (e.g., linked list nodes) are detected during semantic analysis and allocated in contiguous arena blocks.

### 4.2 Escape Analyzer Rules
The Escape Analyzer checks five active escape conditions:
- **Rule 1 (Returned Reference)**: Returning a custom struct or object instance escapes its stack frame and promotes it to Managed Heap (ARC). Returning scalar values (`Int`, `Float`, `Bool`) copies the primitive and does not promote the parent object.
- **Rule 2 (Closure Capture)**: Capturing a struct or container inside an escaping closure promotes the captured storage to Managed Heap. Immutable primitives are copied onto the closure stack capsule.
- **Rule 3 (Container Storage)**: Inserting an object into a `List` promotes the element object to Managed Heap.
- **Rule 4 (Concurrency Boundary)**: Thread transfers require explicit ownership transfer contracts. Unsafe thread captures are caught and rejected at compile time.
- **Rule 5 (Self-Referential Types)**: Struct definitions containing recursive references to themselves are automatically allocated via Arena memory.
- **Cycle Rejection**: Direct or indirect strong ownership cycles (e.g., doubly linked nodes where `A.next = B` and `B.prev = A`) are detected at compile time and rejected to guarantee zero memory leaks without a tracing GC.

---

## 5. Standard Library & Built-ins Reference

### 5.1 Console & System I/O
```lpp
print_str("text")            # Prints a NUL-terminated string to stdout
lpp_print_int(123)           # Prints an integer with a trailing newline
lpp_print_float(3.14159)     # Prints a float with a trailing newline
input() -> String            # Reads one line from stdin
parse_int(str) -> Int        # Parses string digits into a 64-bit integer
```

### 5.2 String Primitives (`lpp_str`)
```lpp
str_len(s) -> Int                            # Returns string length (strlen)
str_concat(a, b) -> String                  # Concatenates two strings
str_split(s, delim_char_code) -> List[Str]   # Splits string by delimiter
str_find(haystack, needle) -> Int            # Returns index or -1
str_replace(s, old_str, new_str) -> String   # Replaces substrings
str_substr(s, start, length) -> String       # Extracts substring slice
str_trim(s) -> String                        # Strips whitespace
```

### 5.3 Binary Buffer Library (`lpp_buf`)
Binary buffers store raw byte sequences with an 8-byte length prefix:
```lpp
buf_alloc(size) -> Int                       # Allocates binary buffer handle
buf_free(buf) -> Void                        # Frees binary buffer
buf_len(buf) -> Int                          # Returns buffer length
buf_get8(buf, offset) -> Int                 # Reads 1 byte (0-255)
buf_set8(buf, offset, val) -> Void           # Writes 1 byte
buf_get16le(buf, offset) -> Int              # Reads 16-bit little-endian integer
buf_set16le(buf, offset, val) -> Void        # Writes 16-bit little-endian integer
buf_get32le(buf, offset) -> Int              # Reads 32-bit little-endian integer
buf_set32le(buf, offset, val) -> Void        # Writes 32-bit little-endian integer
buf_copy(dst, dst_off, src, src_off, len)    # Copies byte ranges
buf_read(path) -> Int                        # Reads entire file into buffer
buf_write(path, buf) -> Int                  # Writes buffer to disk file
buf_crc32(buf, offset, len) -> Int           # Calculates IEEE 802.3 CRC32 checksum
buf_write_str(buf, offset, str) -> Int       # Writes UTF-8 string bytes into buffer
buf_read_str(buf, offset, len) -> String     # Reads buffer section as String
```

### 5.4 Filesystem & Directory APIs (`lpp_dir`)
```lpp
read_file(path) -> String                    # Reads complete UTF-8 text file
write_file(path, content) -> Int             # Overwrites text file (0 = success)
append_file(path, content) -> Int            # Appends text content
delete_file(path) -> Int                     # Removes file from disk
file_exists(path) -> Bool                    # Returns true if path exists
file_size(path) -> Int                       # Returns file size in bytes
file_copy(src, dst) -> Int                   # Copies file
file_move(src, dst) -> Int                   # Moves/renames file
dir_create(path) -> Int                      # Creates directory
dir_list(path) -> List[Str]                  # Lists directory entries
dir_remove(path) -> Int                      # Removes directory recursively
path_exists(path) -> Int                     # Checks if filesystem path exists
path_join(base, child) -> String             # Combines path components safely
```

### 5.5 Process & Environment Control (`lpp_exec`)
```lpp
command_exec(cmd_string) -> Int              # Runs process and returns exit code
command_output(cmd_string) -> String         # Runs process and returns stdout/stderr
env_get(var_name) -> String                  # Returns environment variable value
env_set(var_name, var_value) -> Int          # Sets environment variable
```

### 5.6 JSON Processing
```lpp
json_parse(json_str) -> Int                  # Parses JSON string -> handle
json_get_int(handle, key) -> Int             # Reads integer property
json_get_str(handle, key) -> String          # Reads string property
json_get_obj(handle, key) -> Int             # Reads child object handle
json_free(handle) -> Void                    # Recursively frees JSON node tree
```

### 5.7 Native Sockets & Networking (`lpp_net`)
```lpp
net_dial(host, port, timeout_ms) -> Int      # TCP client connection handle
net_dial_udp(host, port, timeout_ms) -> Int  # UDP socket connection handle
net_listen(port) -> Int                      # TCP server listener handle
net_listen_udp(port) -> Int                  # UDP server listener handle
net_accept(listener) -> Int                  # Accepts inbound TCP client socket
net_accept_timeout(listener, timeout_ms)     # Inbound TCP accept with deadline
net_send_all(socket, payload_str) -> Int     # Retries partial writes until done
net_recv(socket, max_bytes) -> String        # Receives TCP data stream payload
net_recv_udp(socket, max_bytes) -> String    # Receives UDP packet string
net_set_timeout(socket, ms) -> Int           # Sets socket read/write timeouts
net_set_deadline(socket, r_ms, w_ms) -> Int  # Sets individual deadlines
net_set_keepalive(socket, enable, ...) -> Int# Configures TCP keepalive probes
net_resolve(hostname) -> String              # Resolves DNS host name to IPv4
net_close(handle) -> Void                    # Closes socket or listener handle
http_get(url, timeout_ms) -> String          # Performs native HTTP GET request
http_post(url, body, content_type, ms) -> Str# Performs native HTTP POST request
```

---

## 6. Package Manager (`lpp-pm`) & Compiler Toolchain

### 6.1 Package Directory Layout
An L++ package created via `lpp new <name>` uses this standard directory structure:

```text
my_project/
├── lpp.toml         # Package manifest
├── lpp.lock         # Locked dependency hash tree
├── .gitignore
└── src/
    └── main.lpp     # Entry point
```

### 6.2 CLI Command Matrix
The `lpp` binary manages both source-level actions and package-level workflows:

```bash
# Package Manager Commands
lpp new <project_name>    # Scaffold a new package directory
lpp init <project_name>   # Initialize package in current directory
lpp build                 # Compile package into native binary under LppData/build/release/
lpp run                   # Compile and execute package binary
lpp check                 # Semantically check project files without emitting binaries
lpp clean                 # Remove target directories and build artifacts
lpp list                  # List direct dependencies in lpp.toml
lpp tree                  # Print lockfile dependency tree
lpp metadata              # Output package manifest summary

# Single File Commands
lpp check <file.lpp>      # Type-check a single L++ source file
lpp emit <file.lpp>       # Emit transpiled C source
lpp emit <file.lpp> --aot # Emit transpiled C source and Cranelift .o object file

# Global Inspection & Verification
lpp --checkall            # Recursively check all .lpp files in workspace
lpp --version             # Display compiler version
```

### 6.3 Direct Linker (`lpp-link`)
`lpp-link` is L++'s custom, direct tri-format linker that bypasses host C compiler linking:

```bash
# Direct link for Linux ELF:
lpp-link program.o lpp_runtime_min.o -o executable

# Direct link for Windows PE:
lpp-link pe program.obj lpp_runtime_min.obj -o executable.exe

# Direct link for macOS Mach-O:
lpp-link macho program.o lpp_runtime_min.o -o executable

# Inspect binary object sections and relocations:
lpp-link inspect object.o
```

---

## 7. Current Technical Boundaries & Missing Features

To ensure predictability, the compiler rejects unsupported configurations at compile time rather than generating invalid machine code:

1. **Control Flow**: `break` and `continue` keywords are currently missing.
2. **Data Structures**: Hash maps (`Map[K,V]`), sets (`Set[T]`), fixed tuples, and algebraic data types (`enum`/`match`) are not yet implemented.
3. **Methods & Polymorphism**: Interfaces, traits, class methods, and virtual dispatch are missing; all routines are top-level functions.
4. **Constructors**: Struct instances do not accept positional initialization arguments (`Point()` followed by `p.x = ...`).
5. **Float Modulo**: Floating-point modulo (`float % float`) is disabled in Cranelift AOT pending runtime `fmod` integration.
6. **AOT Concurrency Transfer**: `spawn` expressions are transpiled in the C path but disabled in direct AOT until thread-safe atomic closure captures are finalized.
