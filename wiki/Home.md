# L++

L++ is an experimental compiled language with Python-readable syntax, ownership-aware native compilation, Cranelift AOT code generation, ARC-managed aggregate values, and a direct Linux ELF linker path.

## Status at a glance

| Area | Current state |
|---|---|
| Parser, semantic analysis, type checking | Implemented for the documented language subset |
| Ownership | MIR-aware ARC, move/borrow/return handling, closure capsules, list ownership, cycle rejection |
| Linux x86-64 | Cranelift AOT and direct ELF King20 path verified |
| Windows x86-64 | COFF/PE host-link and direct-link work in progress |
| macOS | Host-link supported; ARM64 direct static output is intentionally rejected by macOS policy |
| Networking | Native socket compatibility runtime plus a Rust static-runtime foundation; host-link migration in progress |

## Important honesty rule

L++ is not yet a complete Rust-equivalent safety guarantee, full Go-equivalent networking stack, or finished cross-platform direct linker. Read [[Roadmap]] before relying on an experimental path.

## Navigation

- New users: [[Getting-Started]]
- Language users: [[Language-Guide]] and [[Ownership-and-ARC]]
- Package/build users: [[Compiler-and-Builds]]
- Network work: [[Networking]]
- Systems contributors: [[Native-Linking]] and [[Architecture]]
- Contributors: [[Contributing]]
