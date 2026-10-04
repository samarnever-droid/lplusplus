# Cross-Format Linker Gate (PE / Mach-O)

This document describes the hermetic PE + Mach-O proof layer for the `lpp-linker`
crate, and how it fits the overall "proof pyramid" for cross-format output.

## What it proves

The L++ linker (`lpp_linker`) is format-generic: it parses input objects with the
`object` crate and can emit ELF, **PE (Windows)**, and **Mach-O (macOS)** images.
The ELF path is exercised end-to-end by the driver on this Linux host. PE and
Mach-O cannot be *executed* here (no Wine / no macOS), so we prove the next-best
guarantee that is verifiable in-sandbox: **the emitted images are structurally
well-formed and satisfy the invariants a real loader depends on.**

## Files

### `crates/lpp-linker/tests/cross_format_gate.rs`
Integration test, 7 cases. It is fully **hermetic and host-independent**: it does
not shell out to any toolchain to *produce* inputs. Instead it synthesizes minimal
x86-64 input objects in-process with `object::write`, links them through
`lpp_linker::link_typed`, then re-parses the output with `object::read` and asserts
loader invariants.

Helpers:
- `minimal_object(format)` — a `.text` section holding `xor eax,eax; ret`
  (`31 c0 c3`) plus a defined global `main` (Linkage scope, in-section).
- `calling_object(format, import)` — `call rel32; ret` with an **undefined** symbol
  (Dynamic scope, Undefined section) and a `RelocationFlags::Generic { Relative,
  X86Branch, 32 }` reloc at the operand (addend −4). Used to force an import.

Tests:
| Test | Asserts |
|------|---------|
| `pe_image_is_well_formed` | Output parses as COFF/PE, arch x86-64, entry > 0, has a text section, starts with `MZ`. |
| `pe_import_populates_the_iat` | Synthesized `call ExitProcess` → output `imports()` contains `ExitProcess` bound to `KERNEL32.dll` (auto-classified). |
| `pe_objdump_agrees` | **External validator**: runs `objdump -f`; requires `pei-x86-64` + a non-zero start address. Skips cleanly if `objdump` is absent. |
| `macho_image_is_well_formed` | Output parses as Mach-O, arch x86-64, entry > 0, has a text section, magic `0xFEEDFACF`. |
| `macho_import_binds_against_libsystem` | Synthesized `call exit` → the undefined dynamic import survives into the linked image (routed to `libSystem`). |
| `malformed_input_is_a_clean_error_pe` | Garbage bytes declared as PE input → `link_typed` returns `Err`, no panic. |
| `malformed_input_is_a_clean_error_macho` | Same, for Mach-O. |

### `crates/lpp-linker/examples/emit_cross.rs`
Runnable demonstration (`cargo run -p lpp-linker --example emit_cross`). Emits
`/tmp/xf/hello.exe` and `/tmp/xf/hello.macho` for manual inspection. Verified
output on this host:

```
$ file /tmp/xf/hello.exe /tmp/xf/hello.macho
hello.exe:   PE32+ executable for MS Windows 6.00 (console), x86-64
hello.macho: Mach-O 64-bit x86_64 executable, flags:<NOUNDEFS|DYLDLINK|TWOLEVEL|PIE>

$ objdump -f /tmp/xf/hello.exe
file format pei-x86-64 ; architecture i386:x86-64 ; EXEC_P, D_PAGED
start address 0x0000000140001000
```

Both are recognized by the platform-neutral `file(1)` utility and by GNU
`objdump`, tools that know nothing about our linker — independent corroboration
of the in-test assertions.

## Build wiring
`crates/lpp-linker/Cargo.toml` gains `object = { version = "0.36", features =
["read","write"] }` under **`[dev-dependencies]` only**. The production dependency
stays read-only default features; the `write` capability is confined to tests and
examples and never ships in the library.

## Proof pyramid — where this sits
1. **Layer 1 — hermetic structural (DONE, this gate):** synthesize → link → re-parse
   → assert invariants; plus `file`/`objdump` cross-check. Runs anywhere, no toolchain.
2. **Layer 2 — differential vs `lld`/`lld-link`/`ld64` (open):** requires an LLVM
   install; compare our image structure against a reference linker's.
3. **Layer 3 — execution (gold standard, open):** run the PE under Wine and the
   Mach-O on a macOS runner in CI.

Codegen remains ELF-only for the L++→machine path (there is no L++→COFF/Mach-O
source lowering yet); this gate exercises the linker's format writers directly.
