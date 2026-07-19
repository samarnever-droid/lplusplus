# Windows Native Toolchain Roadmap

L++ now has a Linux x86-64 direct ELF linker path. Windows is the next platform target, but it requires a different executable format and startup model.

## Current Windows support

```text
Cranelift COFF object emission     supported
lpp_runtime.obj packaging          supported by install.ps1
MSVC host-link fallback            supported
Direct PE executable emission      not implemented yet
```

## Phase W0 — CI and artifact validation

- Build `lpp` and `lpp-link` on `windows-latest`.
- Run Rust unit tests on Windows.
- Compile `lpp_runtime.c` to `lpp_runtime.obj` with MSVC.
- Build and run a small Cranelift COFF program through the host-link fallback.

## Phase W1 — COFF object inspection — started

`lpp-link inspect <object.o>` now uses the shared object parser to report:

```text
COFF format
x86-64 architecture
section names, sizes, and kinds
defined / undefined symbol counts
relocation count
relocation-kind classification
```

Windows CI compiles a Cranelift COFF object, runs this inspection command, verifies `Coff` and `X86_64`, then confirms the existing MSVC host-link fallback still runs the executable correctly.

The next W1 step is COFF section merging and AMD64 relocation application. Direct PE output remains intentionally disabled until those object-level operations are tested.

## Phase W2 — PE executable emitter — started

`lpp-link pe <program.obj> -o <program.exe>` now starts the direct PE path for a narrow, runtime-free x86-64 COFF input.

Current PE MVP output includes:

```text
DOS header
PE signature
COFF file header
PE32+ optional header
.text section
.idata import section
console subsystem metadata
entry point at L++ main
internal relative relocation application
Kernel32 import table / IAT generation
```

Windows CI compiles the freestanding Windows runtime object, emits a COFF program containing `print(7)`, invokes `lpp-link pe`, runs the resulting `.exe`, and requires exact output `7`.

The direct runtime MVP resolves these Kernel32 APIs:

```text
GetStdHandle
WriteFile
VirtualAlloc
VirtualFree
```

Still required before complete Windows direct builds:

```text
COFF .rdata / .data / .bss merge
full AMD64 relocation coverage
PE base relocations
ARC/list/closure runtime validation across King20
file, networking, JSON, and thread imports
King20 20 / 20 direct PE gate
```

## Phase W3 — Windows direct runtime

The Linux freestanding runtime uses syscalls. Windows needs a different strategy:

```text
kernel32 imports
WriteFile for output
VirtualAlloc / VirtualFree for ARC allocation
process startup / exit ABI
import table generation
```

The direct PE linker must generate a valid import directory for the required Win32 APIs.

## Phase W4 — King 20 Windows direct-link gate — started

Windows CI now contains a direct PE King20 gate that compiles the freestanding Windows runtime, builds all 20 Stable inputs to COFF, links each with `lpp-link pe`, runs each `.exe`, and checks exact stdout.

The completion gate remains:

```text
King20 Stable
20 / 20
through lpp-link PE
on Windows x86-64
```

Until that CI gate is green, Windows continues to use the packaged `lpp_runtime.obj` plus MSVC linker fallback for normal user builds.

## Design rule

L++ will not claim direct Windows linking until PE images, import tables, runtime allocation, process exit behavior, and King20 correctness are all verified on real Windows runners.
