# L++ Rewrite Specification & Architecture Manual (PPP.md)

---

## 1. Rewrite Crates & Pipeline Architecture

The L++ compiler is decomposed into a strict, unidirectional 15-crate pipeline. No stage depends on legacy code or circular dependencies.

```
Source (.lpp)
  │
  ▼
lpp-frontend ────► lpp-hir ────► lpp-types ────► lpp-mir ────► lpp-ownership ────► lpp-passes
(Lexer/Parser)   (Lower/Scope)  (Inference)    (CFG/SSA)     (Safety/ARC)        (Opt/Const)
                                                                                       │
┌──────────────────────────────────────────────────────────────────────────────────────┘
│
▼
lpp-codegen-api ──► [ Backends: lpp-codegen-cranelift | lpp-codegen-wasm | lpp-codegen-llvm ]
                          │                       │                     │
                          ▼                       ▼                     ▼
                     Native Object (.o/.obj)    WASM Module (.wasm)   LLVM Object (.o)
                          │                                                 │
                          ▼                                                 ▼
                     lpp-linker ◄───────────────────────────────────────────┘
                     (Direct ELF / PE / Mach-O Linker)
                          │
                          ▼
                     Executable Binary
```

### Crate Taxonomy & Responsibilities

| Crate | Responsibility | Key Input | Key Output |
|---|---|---|---|
| `lpp-common` | Universal diagnostics, `Span`, source interner, error reporting. | Source text | Diagnostic bags |
| `lpp-frontend` | Lexer (indent/dedent tokens), recursive-descent parser, typed AST. | Raw `.lpp` text | `AstPackage`, `AstItem` |
| `lpp-hir` | High-level IR, module graph, flat/nested namespace resolution, symbols. | `AstPackage` | `HirPackage`, `HirItem` |
| `lpp-types` | Bidirectional type inference, unification, trait solver, type interner. | `HirPackage` | `TypeAssignments`, `TypeInterner` |
| `lpp-mir` | Mid-level IR CFG, basic blocks, 3-address instructions, SSA values. | Typed HIR | `MirProgram`, `BasicBlock` |
| `lpp-ownership` | Ownership graph, cell placement (Frame/Owned/Shared), balance proofs. | Validated MIR | `OwnershipPlan`, `ProofReport` |
| `lpp-passes` | Deterministic MIR optimization: constprop, constfold, branchfold. | Raw `MirProgram` | Optimized `MirProgram` |
| `lpp-runtime-abi`| ABI constants, 24-byte layout definitions, symbol registries. | - | ABI struct definitions |
| `lpp-runtime` | Pure-Rust native runtime: ARC core, task reactor, list/string/net engines. | Rust/C ABI | `liblpp_runtime.{a,so,dylib,dll}` |
| `lpp-codegen-api` | Target triples, machine types (`I32`/`I64`/`Ptr`), lowering contracts. | - | `Backend` trait, `Target` |
| `lpp-codegen-cranelift` | Native AOT codegen (x86_64, aarch64) emitting ELF, PE, Mach-O objects. | Optimized MIR | Native `.o` / `.obj` |
| `lpp-codegen-wasm` | Pure-Rust WebAssembly binary emitter targeting `wasm32-wasip1`. | Optimized MIR | Binary `.wasm` image |
| `lpp-codegen-llvm` | Textual LLVM-IR generation with clang fallback. | Optimized MIR | Native `.o` |
| `lpp-linker` | Zero-external-dependency tri-format direct linker (ELF, PE, Mach-O). | Objects + libs | Native Executables |
| `lpp-driver` | End-to-end driver, CLI dispatcher, multi-backend routing. | CLI arguments | Completed compilation |
| `keel` | Cargo-equivalent workspace/project manager, lockfile, cache manager. | `lpp.json` | Build DAG, packages |
| `lpp-pm` | Storage core: content-addressed blob store, in-memory KV index. | Blobs/Registries | Resolved packages |

---

## 2. Memory Safety & Ownership Architecture

L++ achieves complete memory safety **without garbage collection**, without a tracing runtime, and without manual `free()` calls.

### 2.1 The 24-Byte ARC Header Layout

Every heap-allocated object (strings, structs, lists, enums, maps, tasks) begins with an invariant 24-byte header:

```
Byte Offset:   0           8          16         24
              ┌───────────┬───────────┬───────────┬───────────────────────┐
              │ Refcount  │ Drop Fn   │ Magic /   │ Payload Data          │
              │ (i64)     │ (fn ptr)  │ Sentinel  │ (Struct fields,       │
              │           │           │ ("ARC1")  │ string bytes, etc.)   │
              └───────────┴───────────┴───────────┴───────────────────────┘
```

- **`rc` (+0, 8 bytes)**: Signed 64-bit reference counter. Mutated via atomic operations for shared cells, or non-atomic for exclusive owned cells.
- **`drop` (+8, 8 bytes)**: Function pointer to the generated type destructor (e.g., `lpp_drop_s12`), or index into a funcref dispatch table.
- **`magic` (+16, 8 bytes)**: 8-byte safety canary: `0x41524331` (`"ARC1"`). If `magic == ARC_IMMORTAL` (`i64::MAX`), the object is statically allocated (string literals, immortal singletons); retain and release calls are no-ops.
- **`payload` (+24)**: Object fields begin strictly at offset +24. Pointers to L++ objects reference offset +24 directly, allowing zero-offset access to fields while accessing metadata via negative indexing (`ptr[-24]`, `ptr[-16]`, `ptr[-8]`).

### 2.2 Three-Tier Placement Engine

The `lpp-ownership` crate classifies every local variable and temporary into one of three storage classes:

1. **`Frame` (Stack Local)**:
   - Value does not escape the current activation record.
   - Zero heap allocation, zero reference count traffic.
   - Deallocated instantly on stack unwinding/block exit.
2. **`Owned` (Exclusive Heap)**:
   - Value is heap-allocated but owned by a single deterministic path.
   - Retain/release calls use non-atomic CPU instructions.
   - Transferred by Move semantics: assigning moves ownership, eliminating refcount increments.
3. **`Shared` (Concurrent / Aliased Heap)**:
   - Value is accessible across threads, tasks, or multiple aliased variables.
   - Retain uses `atomic_add(1)`, release uses `atomic_sub(1)`.
   - When `rc` drops to 0, destructor at `drop` is invoked and the buffer is freed.

### 2.3 Static Cycle Breaking (`E4403`)

- **Containment Graph Analysis**: During MIR analysis, `lpp-ownership` builds a directed graph of type containment ($T_1 \to T_2$ if $T_1$ owns a field of type $T_2$).
- **Cycle Rejection**: If an owning cycle is detected ($A \to B \to A$), the compiler rejects the program with diagnostic `E4403: Owning reference cycle detected`.
- **Resolution**: Cyclic graphs must use weak generations, integer keys, or explicit graph node tables rather than direct owning pointers.

### 2.4 Ownership Balance Proof (Phase 4E)

Before emission, every basic block and edge in the MIR CFG is evaluated against the **Ownership Balance Law**:
$$\sum \text{Retains}(x) - \sum \text{Releases}(x) = 0 \quad \text{along every path from entry to exit}$$
Any path with a lingering reference is flagged as a compile-time leak; any path with an extra release is flagged as a use-after-free or double-free.

---

## 3. The Reasoning Engine & Invariant Verifiers

The compiler enforces strict verification boundaries between all pipeline stages. No pass trusts the output of another pass.

```
[Pass 1: AST] ──► (Verify AST) ──► [Pass 2: HIR] ──► (Verify HIR) ──► [Pass 3: MIR]
                                                                            │
      ┌─────────────────────────────────────────────────────────────────────┘
      ▼
(Verify CORE_MIR_INVARIANTS)
      │
      ▼
[Pass 4: Definite Initialization] ──► (Verify All Locals Initialized)
      │
      ▼
[Pass 5: MIR Optimization] ──► (Verify Invariants Preserved) ──► [Codegen]
```

### Core Verified Invariants

1. **`CORE_MIR_INVARIANTS`**:
   - Every basic block terminates with exactly one explicit terminator (`Goto`, `Switch`, `Return`, `Unreachable`).
   - All instruction operands reference existing locals or valid constants.
   - SSA temporaries are assigned exactly once before use.
2. **Definite Initialization (`DefiniteInitializationLimits`)**:
   - Variables cannot be read along any control-flow path where they might be uninitialized.
   - In branches (`if`/`else`), if a variable is initialized in one branch, it must be initialized in the other or marked optional.
3. **Type Equivalence**:
   - Target and source types in assignments and calls must match strictly down to pointer width and calling convention.

---

## 4. The Direct Linker (`lpp-linker`)

`lpp-linker` is an in-process, zero-dependency link engine that emits native executables without invoking `gcc`, `clang`, `ld`, or `link.exe`.

### 4.1 Tri-Format Native Emission

| Target OS | Binary Format | Features Implemented |
|---|---|---|
| **Linux** | **ELF64** | `ET_EXEC`, 64-bit ELF headers, program headers (`PT_LOAD`, `PT_INTERP`, `PT_DYNAMIC`), PLT/GOT resolution for `libc`/`libm`, static PIE support. |
| **Windows** | **PE32+ (COFF)** | MS-DOS header/stub, PE signature, Optional Header (`IMAGE_NT_HEADERS64`), Section Headers (`.text`, `.rdata`, `.data`), Import Directory Table (`KERNEL32.dll`, `MSVCRT.dll`, `ws2_32.dll`), Base Relocation table (`.reloc`). |
| **macOS** | **Mach-O 64** | `MH_EXECUTE`, Mach-O load commands (`LC_SEGMENT_64`, `LC_DYLD_INFO_ONLY`, `LC_LOAD_DYLINKER`), symbol table, ad-hoc codesigning hash generation. |

### 4.2 Linker Selection Strategy

- **`direct` (default)**: Uses `lpp-linker` in-process. Link time is 5–15ms.
- **`host` / `cc` (fallback)**: Automatically selected if platform relocations require system frameworks (e.g. specialized macOS SDK imports or Windows MSVC toolchains).
- Can be explicitly requested via `--linker direct` or `--linker host`.

---

## 5. Keel: The Build & Package Manager

Keel is the build orchestration tool for L++ (the Cargo analog to the L++ compiler).

### 5.1 Command Interface

```bash
keel init [name]            # Initialize a new package in the current directory
keel new <name>             # Create a new package in a new directory
keel build [--release]      # Compile project and dependencies
keel run [args]             # Build and execute the entry point
keel test                   # Execute all unit and integration test suites
keel add <pkg>[@version]    # Add dependency to lpp.json and fetch
keel remove <pkg>           # Remove dependency from lpp.json
keel update                 # Update dependencies within semantic bounds
keel tree                   # Render dependency graph tree
keel publish                # Verify, sign, and publish package to registry
keel doctor                 # Diagnostic tool for compiler, linker, and tools
```

### 5.2 Project Layout

```
my_project/
├── lpp.json                # Project manifest
├── lpp.lock                # Cryptographic lockfile (SHA-256)
├── src/
│   └── main.lpp            # Application entry point
├── tests/
│   └── test_logic.lpp      # Integration tests
└── target/                 # Build cache and outputs
```

### 5.3 Manifest (`lpp.json`) Specification

```json
{
  "name": "my_project",
  "version": "1.0.0",
  "authors": ["Samar Kotwal <samarkotwal@lplusplus.bond>"],
  "license": "MIT",
  "dependencies": {
    "lppsqlite": "1.2.0",
    "compresslpp": "^0.5.0"
  }
}
```

---

## 6. Decentralized Package Registry

The L++ package registry is **100% decentralized and git-backed**. There is no central server, no API token storage, and no credit card requirement.

### 6.1 Architectural Model

- **Canonical Repository**: Git remote (default: `git@github.com:samarnever-droid/llppregistry.git`).
- **Authority**: Git commit and push access. Deploy keys, SSH keys, or signed commits form the sole publisher credentials.
- **Content-Addressed**: Every package artifact is named by its SHA-256 hash.

```
llppregistry/
├── index/                          # Sparse package index
│   └── lp/
│       └── ps/
│           └── lppsqlite           # JSON metadata versions
├── blob/                           # Content-addressed packages
│   └── e3b0c44298fc1c14...tar.gz   # Files named strictly by SHA-256
└── registry/
    └── index.json                  # Aggregated public catalog
```

### 6.2 HTTP Read-Only Mirror

- **URL**: `https://registry.lplusplus.bond`
- **Role**: Read-only cache and web interface powered by Cloudflare Workers. It reflects git state and cannot accept writes. Publishing happens strictly through `keel publish` via git.

---

## 7. Complete Language Specification & Syntax

### 7.1 Lexical Conventions

- **Whitespace**: 4 spaces indentation per level. Tabs are forbidden.
- **Colons**: `:` terminates block headers (`def`, `if`, `while`, `for`, `struct`, `enum`).
- **Comments**: `#` begins a line comment.

### 7.2 Variable Declarations & Mutability

```lpp
# Immutable variable declaration
x := 10

# Mutable variable declaration
mut y := 20
y = y + 5           # OK: mutated

# Lexical shadowing (re-binding)
name := "Alice"
name := 42          # OK: shadows previous binding with new type
```

### 7.3 Data Types

| Type | Representation | Example |
|---|---|---|
| `Int` | 64-bit signed integer | `42`, `-100`, `0x1F` |
| `Float` | 64-bit IEEE-754 float | `3.14159`, `-0.01` |
| `Bool` | 1-byte boolean | `true`, `false` |
| `Str` / `String`| Immutable UTF-8 string | `"Hello, World!"` |
| `Char` | 32-bit Unicode scalar | `'A'`, `'🚀'` |
| `Void` | Unit return | Return nothing |
| `List[T]` | Dynamic heap array | `[1, 2, 3]` |
| `Slice[T]` | Borrowed contiguous slice | `items[1:4]` |
| `Map[K, V]`| Hash table | `{"key": 100}` |
| `(A, B)` | Structural tuple | `(1, "test", true)` |

### 7.4 Control Flow

```lpp
# If-Elif-Else
if score >= 90:
    print("Grade: A")
elif score >= 75:
    print("Grade: B")
else:
    print("Grade: C")

# While Loop
mut counter := 0
while counter < 5:
    counter = counter + 1

# Range Loop
for i in range(0, 10):
    if i == 5:
        continue
    print_int(i)

# List Iteration
items := ["apple", "banana", "cherry"]
for item in items:
    print_str(item)
```

### 7.5 Functions & Closures

```lpp
# Standard typed function
def add(a: Int, b: Int) -> Int:
    return a + b

# Default parameters
def greet(name: Str, greeting: Str = "Hello") -> Void:
    print(greeting + ", " + name)

# Variadic arguments
def sum_all(*numbers: List[Int]) -> Int:
    mut total := 0
    for n in numbers:
        total = total + n
    return total

# First-class closures
multiplier := fn(x: Int) -> Int: x * 2
result := multiplier(10)
```

### 7.6 Structs & Methods

```lpp
struct Vector2:
    x: Float
    y: Float

    def length_squared(self) -> Float:
        return self.x * self.x + self.y * self.y

    def scale(mut self, factor: Float) -> Void:
        self.x = self.x * factor
        self.y = self.y * factor

# Construction
pos := Vector2(x=3.0, y=4.0)
dist := pos.length_squared()
```

### 7.7 Enums & Pattern Matching

```lpp
enum Option:
    Some(Int)
    None

def process(val: Option) -> Int:
    match val:
        Option.Some(n) => return n * 2
        Option.None    => return 0
```

### 7.8 Asynchronous Tasks & Concurrency

```lpp
async def fetch_data(url: Str) -> Str:
    # Asynchronous task body
    return "response"

def main() -> Int:
    # Spawn task on background worker pool
    handle := spawn fetch_data("https://lplusplus.bond")
    
    # Await completion
    result := await handle
    print_str(result)
    return 0
```

### 7.9 Builtin I/O & System Primitives

| Function | Signature | Description |
|---|---|---|
| `print(val)` | Polymorphic | Prints any value followed by newline |
| `print_int(i)` | `(Int) -> Void` | Fast non-allocating integer printer |
| `print_float(f)` | `(Float) -> Void` | Fast non-allocating float printer |
| `print_str(s)` | `(Str) -> Void` | Prints string with trailing newline |
| `write_str(s)` | `(Str) -> Void` | Writes string without trailing newline |
| `eprint_str(s)` | `(Str) -> Void` | Writes string to stderr |
| `exit(code)` | `(Int) -> Void` | Terminates process with exit code |
| `panic(msg)` | `(Str) -> Void` | Aborts execution with error message and stack trace |
