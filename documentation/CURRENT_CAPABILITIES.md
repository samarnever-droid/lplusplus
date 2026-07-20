# L++ Current Capabilities — v0.1.3

Last reviewed: 2026-07-20.

This document is the source of truth for public feature claims. Experimental does not mean unavailable; it means its platform, safety, or compatibility boundary is explicitly limited.

## Language

| Capability | Status | Boundary |
|---|---|---|
| Functions, structs, `if`/`else`, `while` | Available | Cranelift AOT and C compatibility paths |
| `for value in list` | Available | Desugars to index-based List iteration |
| `for i in range(end)` / `range(start, end)` | Available | Desugars directly to integer `while` MIR; no range allocation |
| `List[Int]` | Verified AOT | ARC-safe list lifetime |
| `List[Custom]` | Verified AOT | Custom elements retained/released |
| Strings | Basic | literals, input/output, file APIs; rich string operations are incomplete |
| Struct constructor arguments | Not available | Initialize fields after `Struct()` |
| `List[String]`, maps, sets, enums, traits | Not available | Require an owned string/container model |
| `break`, `continue`, async, channels | Not available | Future control-flow/runtime work |

## Ownership and safety

Verified Cranelift AOT behavior includes ARC allocation, move/borrow/return handling, aliasing, field aliases, closure capsules, destructor chains, `List[Int]`, `List[Custom]`, and strong ownership-cycle rejection.

L++ must not be described as language-wide Rust-equivalent safe yet. The verified promise is the documented AOT subset; unsupported ownership behavior should be rejected rather than silently compiled. See [Safety Mission](Safety_Mission.md).

## Filesystem

Host-linked scalar APIs:

```lpp
read_file(path) -> Str
write_file(path, data) -> Int
write_file_atomic(path, data) -> Int
append_file(path, data) -> Int
delete_file(path) -> Int
file_exists(path) -> Bool
file_size(path) -> Int
file_copy(source, destination) -> Int
file_move(source, destination) -> Int
make_dir(path) -> Int
make_dir_all(path) -> Int
remove_dir(path) -> Int
file_is_dir(path) -> Bool
```

Directory listing, `List[String]`, binary buffers, streaming file handles, structured errors, and direct-link filesystem support remain incomplete.

## Networking

Host-linked native TCP APIs exist: connect/listen/accept, complete writes, read/write deadlines, read, and close. A Rust socket runtime foundation provides TCP and connected UDP ABI work. TLS, HTTP, async networking, byte buffers, typed sockets, and `Result` errors are not complete. Networking never uses cURL.

## Build and packages

```text
lpp check <file>
lpp emit <file>
lpp emit <file> --aot
lpp build
lpp run
```

Package build artifacts and cache use:

```text
LppData/build/release/<package>
LppData/cache/<fingerprint>.o
```

The cache hashes source modules, manifest, compiler version, platform, architecture, and AOT optimization profile.

## Native targets

| Target | State |
|---|---|
| Linux x86-64 host-linked AOT | Supported |
| Linux x86-64 direct ELF | Verified direct subset; files/networking/JSON/threads not supported |
| Windows x86-64 | COFF/PE and host-link support; direct PE remains incomplete |
| macOS host link | Supported compatibility path |
| macOS Intel direct Mach-O | Experimental |
| macOS ARM64 static direct Mach-O | Explicitly rejected; dynamic libSystem path required |

## Performance claims

Do not use fixed claims such as “3 ms compile” or “138 KB binaries.” Performance depends on workload, compiler mode, host linker, and target. Use the checked-in benchmark reports under `benchmarks/`.

Recent verified findings:

```text
- 100k straight-line scalar AOT compilation improved substantially through safe MIR propagation and dead-store elimination.
- Struct/List and List Labyrinth runtime workloads are near the C compatibility path in current local measurements.
- Tight scalar loops and recursive/call-heavy programs remain the C-Speed optimization priority.
```
