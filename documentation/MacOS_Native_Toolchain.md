# macOS Native Toolchain Roadmap

## Current macOS support

```text
Cranelift Mach-O object emission     supported
clang host-link fallback             validated in CI
release bundles                      planned for Intel and Apple Silicon
Direct Mach-O executable emitter     not implemented yet
```

## M0 — Host fallback and release packaging

macOS CI validates:

```text
cargo test
Cranelift AOT Mach-O object
clang runtime object
clang link
correct executable output
```

The release workflow packages:

```text
lpp-macos-x86_64.tar.gz
lpp-macos-arm64.tar.gz
```

Normal macOS users can install a release bundle without Rust once the matching release asset is published.

## M1 — Mach-O object inspection

`lpp-link inspect` already uses the shared object parser. The next direct-link step is to classify Mach-O sections, symbols, relocations, and architecture-specific object details under macOS CI.

## M2 — Direct Mach-O MVP

Intel x86_64 can execute a narrow static direct Mach-O MVP. Apple Silicon cannot: production macOS rejects static `MH_EXECUTE` arm64 binaries and kills them instead of launching them. `lpp-link macho-arm64` therefore rejects this mode explicitly until the dynamic import path exists.

A real ARM64 direct executable writer requires:

```text
Mach header 64
LC_SEGMENT_64 commands
__TEXT / __text
__TEXT / __const
__LINKEDIT
symbol table and string table
entry point command
rebase/bind information
```

The first direct target is a runtime-free console executable on one architecture at a time.

## M3 — Darwin runtime

The Linux direct runtime uses syscalls; macOS needs Darwin-compatible runtime and loader behavior. Later work includes:

```text
libSystem imports
write output path
allocation API
ARC runtime
closure/list runtime
King20 direct Mach-O gate
```

## Gate

```text
King20 Stable
20 / 20
through lpp-link Mach-O
on macOS x86-64 and arm64
```

Until then, macOS uses the verified clang host-link fallback.
