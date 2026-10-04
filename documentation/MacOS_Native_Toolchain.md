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

A real direct executable writer requires:

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

## Current v0.1.3 status note — 2026-07-20

This document is historical/design context. For current public capability claims,
platform boundaries, filesystem APIs, package cache layout, and known missing
features, see [Current Capabilities](CURRENT_CAPABILITIES.md).

Current rules:

```text
- Do not claim fixed compile-time, binary-size, or C/Rust parity numbers.
- Do not claim language-wide Rust-equivalent safety.
- Host-linked AOT is the compatibility path for filesystem and networking work.
- Linux direct ELF remains a verified subset; filesystem/networking are not direct-link features yet.
- macOS ARM64 static direct output is rejected; dynamic libSystem imports are required.
- L++ package outputs/cache are LppData/build/release and LppData/cache.
```
