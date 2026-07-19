<p align="center">
  <img src="assets/lpp-logo.svg" width="190" alt="L++ four-pillar prism logo">
</p>

<h1 align="center">L++</h1>

<p align="center"><strong>Readable like Python. Ownership-aware like Rust. Fast iteration like Go. Native by default.</strong></p>

<p align="center">
  <a href="benchmarks/king20/stable/v1/latest.md">King20 Stable</a> ·
  <a href="documentation/Native_Linker_Roadmap.md">Native Linker</a> ·
  <a href="Doc.md">Language Guide</a> ·
  <a href="linguist/UPSTREAM_LINGUIST_PR.md">Linguist Package</a>
</p>

> **Status — July 19, 2026:** L++ is an active experimental native language toolchain. The supported Linux x86-64 subset has ownership-aware MIR, ARC destructors, closures, `List[Int]`, `List[Custom]`, Cranelift AOT, and a direct ELF linker path. Unsupported features are deliberately rejected rather than silently compiled with unsafe semantics.

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
    print(returned.value)
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
| **Native** | Cranelift AOT, PIC objects, direct ELF linking for the verified Linux subset. |

## Verified ownership model

The current AOT ownership contract covers:

```text
✓ ARC allocation and destructor callbacks
✓ owned returns and borrowed parameter returns
✓ direct aliases and field aliases
✓ branch-safe release insertion
✓ nested struct destructor chains
✓ closure capsules and closure environments
✓ List[Int] and List[Custom] element ownership
✓ strong ownership-cycle rejection
```

The compiler rejects direct or indirect strong ownership cycles such as:

```lpp
struct Node:
    next: Node
```

until explicit `Weak`, arena, or cycle-collection semantics are available.

Read the detailed audit: [`documentation/Cranelift_Ownership_Audit_2026-07-19.md`](documentation/Cranelift_Ownership_Audit_2026-07-19.md).

## Build paths

### Host-link fallback

Portable development path:

```text
L++ source → Cranelift object → packaged runtime object → host linker → executable
```

Phase 1 packages `lpp_runtime.o` / `lpp_runtime.obj`, so installed builds no longer recompile the full C runtime for every project build.

### Direct Linux ELF path

Experimental Linux x86-64 path:

```text
L++ source → Cranelift object + freestanding runtime → lpp-link → static ELF
```

Use it after installation:

```bash
LPP_LINKER=direct lpp build
```

The direct linker currently supports all **20 / 20 King20 Stable** workloads without a host final linker.

It includes:

```text
.text + .rodata merging
internal symbols
GOT runtime imports
Linux startup and exit
freestanding integer/string output
freestanding ARC, closures, and lists
```

Networking, files, threads, JSON, writable data sections, Windows PE, and macOS Mach-O still use the host-link fallback. Networking uses native OS sockets—not cURL—and its current API/roadmap is in [`documentation/Networking.md`](documentation/Networking.md). The linker roadmap is in [`documentation/Native_Linker_Roadmap.md`](documentation/Native_Linker_Roadmap.md).

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

Latest direct-link result:

```text
King20 Stable: 20 / 20 passed through lpp-link
Direct link:   ~1.5–2.1 ms per workload
Host link:     ~200 ms per workload on the benchmark sandbox
```

## Scalability: 10k → 100k LOC

The scalability suite records each compiler phase at 10,000, 50,000, and 100,000 LOC:

```bash
python3 benchmarks/scalability/run.py
```

At 100k LOC on the recorded Linux sandbox:

| Phase | Time |
|---|---:|
| Lexing | 26.405 ms |
| Parsing | 49.233 ms |
| Semantic analysis | 9.601 ms |
| Type checking | 5.816 ms |
| Escape analysis | 691.433 ms |
| MIR generation | 19.834 ms |
| Cranelift AOT | 634.006 ms |
| Compiler total | 783.497 ms |
| Host link | 202.726 ms |

Escape analysis and AOT lowering are the current scaling targets. See [`benchmarks/scalability/latest.md`](benchmarks/scalability/latest.md) for system information and full data.

## Cross-language comparison

Compare equivalent canonical workloads against C, C++, Rust, Go, and Zig:

```bash
python3 benchmarks/comparison/run.py
```

Unavailable toolchains are reported as `SKIP`, never fabricated as results. L++ comparison now records compile, link, total build, and runtime time separately. See [`benchmarks/comparison/README.md`](benchmarks/comparison/README.md).

## Windows: started

Windows currently supports:

```text
✓ Cranelift COFF object generation
✓ lpp_runtime.obj packaging via install.ps1
✓ MSVC host-link fallback
✓ Windows CI build and COFF fallback smoke test
✓ `lpp-link inspect` COFF / x86-64 inspection gate
✓ Runtime-free direct PE MVP in Windows CI
✓ Kernel32 import table / IAT MVP (`WriteFile`, `VirtualAlloc`)
⏳ COFF `.rdata` / `.data` / `.bss` merge and broad AMD64 relocation coverage
⏳ PE base relocations, full ARC/list/closure validation, and King20 direct PE
```

The Windows direct-toolchain plan is in [`documentation/Windows_Native_Toolchain.md`](documentation/Windows_Native_Toolchain.md).

## GitHub language recognition

L++ uses `.lpp` and TextMate scope `source.lpp`. Generated benchmark reports are marked `linguist-generated=true` so they do not distort repository statistics.

- Repository Linguist-readiness PR: merged.
- Upstream GitHub Linguist PR: [#8075](https://github.com/github-linguist/linguist/pull/8075), ready for maintainer review.

Global recognition depends on GitHub Linguist maintainers accepting the upstream language definition. The submission package is documented in [`linguist/UPSTREAM_LINGUIST_PR.md`](linguist/UPSTREAM_LINGUIST_PR.md).

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

Release bundles target Linux x86-64, Windows x86-64, macOS x86-64, and macOS arm64. macOS currently uses the verified clang host-link fallback while direct Mach-O work begins.

### Command model: files vs packages

| Intent | Command | Result |
|---|---|---|
| Check one source file | `lpp check calc.lpp` | Diagnostics only; no artifacts |
| Emit C source | `lpp emit calc.lpp` | `calc.c` next to source |
| Emit C + AOT object | `lpp emit calc.lpp --aot` | `calc.c` and `calc.o` |
| Build a package | `lpp build` | Executable from `lpp.toml` package |
| Run a package | `lpp run` | Build then run package executable |

`lpp calc.lpp` is kept as a legacy source invocation and prints guidance. Prefer `emit` for one-file artifacts and `build`/`run` for project executables.

### Create a project

```bash
lpp new hello-lpp
cd hello-lpp
lpp build
lpp run
```

For syntax, semantics, runtime contracts, and current limitations, read [`Doc.md`](Doc.md).

## Architecture

```text
Source
  ↓
Lexer (significant whitespace)
  ↓
Parser → AST
  ↓
Semantic resolver + symbol table
  ↓
Type checker
  ↓
Escape / alias analysis
  ↓
Ownership-aware MIR
  ↓
ARC insertion + generated destructors
  ↓
Cranelift AOT object ──→ lpp-link ELF (Linux verified subset)
        │
        └──────────────→ packaged runtime + host-link fallback
```

## Project identity

The four-pillar visual identity is available as:

```text
assets/lpp-logo.svg
editors/vscode/lpp-logo.svg
```

The logo is SVG/XML, so it can be reused in documentation, websites, packaging, editors, and future extensions.

---

L++ is ambitious by design. The project prefers a narrow, verified capability over a broad unverified promise: if a feature lacks a correct ownership and runtime contract, the compiler should reject it rather than pretend it is safe.
