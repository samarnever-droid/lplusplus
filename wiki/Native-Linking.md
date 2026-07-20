# Native Linking

## Linux x86-64

The direct ELF linker is the most mature direct-native path. It supports the verified King20 direct suite, text/rodata, internal symbols, GOT runtime imports, ARC, closures, and supported lists.

## Windows x86-64

COFF generation, inspection, runtime packaging, and direct PE work are in progress. Host-link fallback remains the safe compatibility path when a feature is not represented by the direct PE writer.

## macOS

Host-linking is the supported compatibility path. Intel direct Mach-O experiments and Apple Silicon requirements are documented separately. Production macOS ARM64 rejects the static direct executable style used by the runtime-free MVP; a dynamic Mach-O/libSystem import path is required.

## Networking and direct linkers

Networking uses host linking today. A Rust static network runtime cannot simply be inserted into a minimal direct linker: its archive/object requirements, allocation, symbols, and platform dependencies must be supported deliberately. Correctness takes priority over premature direct-link claims.

## v0.1.3 current-status note

This page is maintained with the project, but current support claims are
centralized in [Current Capabilities](../documentation/CURRENT_CAPABILITIES.md).

```text
Use LppData/build/release and LppData/cache for package artifacts.
Use host-linked AOT for filesystem/networking work.
Do not assume direct ELF supports files, networking, JSON, or threads.
Do not claim language-wide Rust-equivalent safety outside the verified AOT subset.
```
