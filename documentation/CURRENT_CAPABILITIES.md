# L++ Current Capabilities Matrix — v0.1.3

Last reviewed: 2026-07-21.

This document serves as the authoritative source of truth for verified compiler capabilities, toolchain commands, standard library primitives, and platform boundaries in L++.

---

## 1. Core Language & Compiler Pipeline

| Capability | Status | Implementation Boundary |
|---|---|---|
| Functions (`def`) | **Available** | Top-level routines with explicit param and return types; local type inference |
| Variables (`:=`, `mut`, `=`) | **Available** | Immutable by default; lexical shadowing supported across all scopes |
| Structs (`struct`) | **Available** | Value semantics by default; promoted to ARC heap or Arena as required |
| Control Flow (`if`/`else`, `while`) | **Available** | Full branching and conditional loops |
| Range Loops (`for i in range(n)`) | **Available** | Zero-allocation MIR lowering to integer comparison `while` loops |
| List Loops (`for item in list`) | **Available** | Desugars to index-based iteration over `List[T]` |
| Closures (`fn`) | **Available** | Inline and block closures with lexical capture |
| Dynamic Lists (`List[T]`) | **Available** | Supported elements: `Int`, `Float`, `Bool`, `Str`, `CustomStruct` (ARC managed) |
| Struct Constructors | **Available** | Supports both positional field initialization `Point(10, 20)` and zero-argument allocation `Point()` |
| `break` / `continue` | **Available** | Fully supported in `while`, `for i in range(...)`, and `for item in list` |
| Key-Value Maps (`Map[K, V]`) | **Available** | Open-addressing hash table supporting `Int` and `Str` keys with `Int`, `Float`, `Str` values |
| `Set[T]`, Enums, Traits | **Not Available** | Planned container and algebraic type extensions |

---

## 2. Memory Model & Ownership Verification

L++ uses an automated, rule-based **Hybrid Memory Model**:

- **Stack Allocation (Value)**: Scalars (`Int`, `Float`, `Bool`) and non-escaping structs are stack-allocated.
- **Automatic Reference Counting (ARC)**: Structs or elements escaping through returns, closure capsules, or list insertion are heap-allocated with atomic headers (`LppArcHeader`) and automatic `retain`/`release` MIR insertions.
- **Arena Storage**: Self-referential structs (`struct Node: next: Node`) are automatically allocated in arena memory.
- **Cycle Rejection**: Direct and indirect ownership cycles are detected and rejected at compile time to guarantee zero leaks without requiring a tracing garbage collector.

---

## 3. Verified Standard Library Primitives

### 3.1 Console, System & Strings (`lpp_str`)
- **Console**: `print(val)`, `print_str(s)`, `lpp_print_int(n)`, `lpp_print_float(f)`, `input()`, `parse_int(s)`
- **String Primitives**: `str_len(s)`, `str_concat(a, b)`, `str_split(s, delim)`, `str_find(s, sub)`, `str_replace(s, old, new)`, `str_substr(s, start, len)`, `str_trim(s)`

### 3.2 Binary Buffer Primitives (`lpp_buf`)
- **Memory & Disk I/O**: `buf_alloc(sz)`, `buf_free(b)`, `buf_len(b)`, `buf_read(path)`, `buf_write(path, b)`
- **8-bit / 16-bit / 32-bit LE Accessors**: `buf_get8`, `buf_set8`, `buf_get16le`, `buf_set16le`, `buf_get32le`, `buf_set32le`
- **Data Operations**: `buf_copy`, `buf_crc32` (IEEE 802.3 CRC32 checksums), `buf_write_str`, `buf_read_str`

### 3.3 Filesystem & Directories (`lpp_dir`)
- **File APIs**: `read_file`, `write_file`, `append_file`, `delete_file`, `file_exists`, `file_size`, `file_copy`, `file_move`
- **Directory APIs**: `dir_create`, `dir_list`, `dir_remove`, `path_exists`, `path_join`

### 3.4 Process Execution & Environment (`lpp_exec`)
- `command_exec(cmd) -> Int`: Executes process and returns numeric exit code.
- `command_output(cmd) -> String`: Executes process and captures combined stdout/stderr output.
- `env_get(key) -> String`: Reads environment variable value.
- `env_set(key, val) -> Int`: Sets environment variable.

### 3.5 Native Networking (`lpp_net`)
- **TCP Sockets**: `net_dial`, `net_listen`, `net_accept`, `net_accept_timeout`, `net_send`, `net_send_all`, `net_recv`, `net_set_timeout`, `net_set_deadline`, `net_set_keepalive`, `net_close`
- **UDP Sockets**: `net_dial_udp`, `net_listen_udp`, `net_recv_udp`
- **DNS & HTTP**: `net_resolve`, `http_get`, `http_post`

### 3.6 JSON Parsing
- `json_parse`, `json_get_int`, `json_get_str`, `json_get_obj`, `json_free`

### 3.7 Hash Maps (`lpp_map`)
- `map_new()`, `map_put(m, k, v)`, `map_get(m, k)`, `map_has(m, k)`, `map_len(m)`, `map_remove(m, k)`
- Native support across C Transpilation, Cranelift AOT, and direct freestanding `lpp-link` execution.

---

## 4. Package Manager & Toolchain Ecosystem

- **Self-Hosted PM (`lpp-pm`)**:
  - `lpp new <name>`: Bootstraps self-hosted package manager from `pm/src/main.lpp` and scaffolds new package. Path resolution works seamlessly outside repo roots (e.g. `/tmp`).
  - `lpp build`, `lpp run`, `lpp check`, `lpp clean`, `lpp list`, `lpp tree`, `lpp metadata`, `lpp outdated`
- **Compiler Options**:
  - `lpp check <file.lpp>`: Fast semantic/type check pass.
  - `lpp emit <file.lpp> [--aot]`: Emits C transpile artifacts and optional Cranelift AOT object files.
  - `lpp --checkall`: Recursive workspace-wide type verification.
- **Direct Linker (`lpp-link`)**:
  - **Linux x86-64**: Standalone ELF direct linker.
  - **Windows x86-64**: Standalone multi-section PE COFF direct linker (`.text`, `.rdata`, `.data`, `.idata`, `.reloc`), fully compatible with `windows_x86_64_min.c` under `/DLPP_FREESTANDING`.
  - **macOS x86-64/ARM64**: Mach-O direct object emitter.
