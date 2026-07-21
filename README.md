<p align="center">
  <img src="assets/lpp-logo.svg" width="190" alt="L++ four-pillar prism logo">
</p>

<h1 align="center">L++</h1>

<p align="center"><strong>Readable like Python. Ownership-aware like Rust. Fast iteration like Go. Native by default.</strong></p>

<p align="center">
  <a href="benchmarks/king20/stable/v1/latest.md">King20 Stable</a> ·
  <a href="documentation/Native_Linker_Roadmap.md">Native Linker</a> ·
  <a href="Doc.md">Language Guide</a> ·
  <a href="documentation/CURRENT_CAPABILITIES.md">Current Capabilities</a> ·
  <a href="linguist/UPSTREAM_LINGUIST_PR.md">Linguist Package</a>
</p>

> **Status — July 21, 2026:** L++ is an active experimental native language toolchain. The supported Linux x86-64 and Windows x86-64 native subsets feature ownership-aware MIR, ARC destructors, closure capsules, `List[T]` dynamic lists, `Map[K, V]` hash tables, binary buffer operations (`buf_*`), process execution (`command_*`), directory manipulation (`dir_*`), native networking (`net_*`), Cranelift AOT, self-hosting package manager (`lpp-pm`), and a tri-format direct linker path (`lpp-link`). Unsupported features are deliberately caught and rejected rather than silently compiled with unsafe semantics.

---

## Why L++ exists

L++ is built around a difficult four-way design goal:

```text
                 Python-like readability
                         ▲
                         │
      Rust-inspired ◄────┼────► Go-like iteration speed
      ownership safety    │
                         ▼
                 Native executable performance
```

The language tries to make the safe path the pleasant path:

```lpp
struct Box:
    value: Int

def identity(value: Box) -> Box:
    return value

def main():
    original := Box()
    returned := identity(original)
    lpp_print_int(returned.value)
```

There are no user-visible `Arc`, `Rc`, raw pointers, or manual frees in this example. L++ lowers the program into explicit internal ownership operations:

```text
AllocateArc → Borrow → Retain → Move → ReturnOwned → Release
```

## The four pillars

| Pillar | L++ approach |
|---|---|
| **Readable** | Significant whitespace, `def`, `struct`, inferred locals, explicit public signatures. |
| **Safe by construction** | Ownership-aware MIR, ARC, borrow/return contracts, generated destructors, cycle rejection. |
| **Fast to iterate** | Small Rust compiler pipeline, explicit types, phase timing, fast Cranelift object emission. |
| **Native** | Cranelift AOT, PIC objects, direct ELF / PE linking via `lpp-link`. |

## Verified ownership model

The current AOT ownership contract covers:

```text
✓ ARC allocation and destructor callbacks
✓ owned returns and borrowed parameter returns
✓ direct aliases and field aliases
✓ branch-safe release insertion
✓ nested struct destructor chains
✓ closure capsules and closure environments
✓ List[T] and Map[K, V] element and entry ownership
✓ binary buffer memory allocation, read/write, string conversions & CRC32
✓ strong ownership-cycle rejection
```

The compiler rejects direct or indirect strong ownership cycles such as:

```lpp
struct Node:
    next: Node
```

until explicit `Weak`, arena, or cycle-collection semantics are available.

Read the detailed audit: [`documentation/Cranelift_Ownership_Audit_2026-07-19.md`](documentation/Cranelift_Ownership_Audit_2026-07-19.md). The safety boundary and graduation criteria are in [`documentation/Safety_Mission.md`](documentation/Safety_Mission.md).

## Build paths

### Host-link fallback

Portable development path:

```text
L++ source → Cranelift object → packaged runtime object → host linker → executable
```

Phase 1 packages `lpp_runtime.o` / `lpp_runtime.obj`, so installed builds no longer recompile the full C runtime for every project build.

### Direct Linker (`lpp-link`)

Native direct link path bypassing host C compilers:

```text
L++ source → Cranelift object + freestanding runtime → lpp-link → static executable
```

Use it via CLI or package builds:

```bash
LPP_LINKER=direct lpp build
```

Supported Direct Targets:

```text
- Linux x86-64 ELF (.text, .rodata, GOT imports, freestanding ARC/lists)
- Windows x86-64 PE COFF (.text, .rdata, .data, .idata, .reloc with /DLPP_FREESTANDING)
- macOS Mach-O direct object emitter (experimental)
```

Networking, files, threads, JSON, and process execution feature native runtime bindings. Networking uses native OS sockets (Winsock2 / POSIX sockets)—never cURL—and its current API is documented in [`documentation/Networking.md`](documentation/Networking.md). The linker roadmap is in [`documentation/Native_Linker_Roadmap.md`](documentation/Native_Linker_Roadmap.md).

## King20 benchmark standards

| Track | Purpose | Policy |
|---|---|---|
| [**King20 Stable v1**](benchmarks/king20/stable/v1/README.md) | Historical correctness and performance baseline | Frozen forever; cut `v2` instead of editing it. |
| [**King20 Experimental**](benchmarks/king20/experimental/README.md) | New ownership features, regressions, and optimization experiments | Evolves with the toolchain. |
| [**King20 Direct ELF**](benchmarks/king20/direct_elf_latest.md) | Host-link-free Linux validation | Must preserve exact stdout and exit status. |

Run them:

```bash
python3 benchmarks/king20/run.py --suite stable
python3 benchmarks/king20/run.py --suite experimental
python3 benchmarks/king20/run_direct_elf.py
```

## Getting started

### Install a release — Rust not required

The default installers download a matching release bundle containing `lpp`, `lpp-link`, and packaged runtime objects. End users do **not** need Rust/Cargo for normal installation.

```bash
# Linux x86-64
curl -fsSL https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/install.sh | sh

# Windows PowerShell
irm https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/install.ps1 | iex
```

To build locally from a source checkout instead:

```bash
LPP_FROM_SOURCE=1 ./install.sh
# PowerShell: $env:LPP_FROM_SOURCE=1; .\install.ps1
```

### Command model: files vs packages

| Intent | Command | Result |
|---|---|---|
| Check one source file | `lpp check calc.lpp` | Diagnostics only; no artifacts |
| Emit C source | `lpp emit calc.lpp` | `calc.c` next to source |
| Emit C + AOT object | `lpp emit calc.lpp --aot` | `calc.c` and `calc.o` |
| Build a package | `lpp build` | Executable from `lpp.toml` package |
| Run a package | `lpp run` | Build then run package executable |

### Create a project

```bash
lpp new testproj
cd testproj
lpp build
lpp run
```

For complete language syntax, semantics, standard library function signatures, and current boundaries, see [`Doc.md`](Doc.md) and [`documentation/CURRENT_CAPABILITIES.md`](documentation/CURRENT_CAPABILITIES.md).
