# Direct Linker (lpp-link)

`lpp-link` is a custom linker that produces standalone executables without requiring `gcc`, `clang`, or MSVC.

## Usage

```bash
# Linux ELF (default)
lpp-link input.o runtime.o -o output

# Windows PE
lpp-link pe input.obj runtime.obj -o output.exe

# macOS Mach-O
lpp-link macho input.o runtime.o -o output

# Inspect COFF/ELF object
lpp-link inspect input.obj
```

## Supported Formats

| Format | Platform | Status |
|--------|----------|--------|
| ELF x86-64 | Linux | ✅ Full |
| PE x86-64 | Windows | ✅ Full (15.5KB binaries!) |
| Mach-O x86-64/ARM64 | macOS | ✅ Basic |

## PE Linker Features

The Windows PE linker is the most advanced, supporting:
- Multi-section layout: `.text`, `.rdata`, `.data`, `.idata`, `.reloc`
- Import Address Table (IAT) for KERNEL32.dll
- Base relocations for ASLR
- TLS directory support
- COMDAT section handling
- Raw COFF relocation types (ADDR64, REL32, ADDR32NB, SECREL, SECTION)
- Freestanding executables (no MSVC CRT dependency)

## Binary Size Comparison

| Tool | Binary Size | Dependencies |
|------|------------|-------------|
| **lpp-link PE** | **15.5 KB** | None (freestanding) |
| MSVC link.exe | ~100 KB+ | MSVC CRT |
| **lpp-link ELF** | **~8 KB** | None (freestanding) |
| cc (gcc/clang) | ~20 KB+ | libc |
| Go | ~2,400 KB | Go runtime |

## Architecture

```
Input Objects (.o / .obj)
    │
    ├── Parse COFF/ELF/Mach-O sections
    ├── Merge .text, .rdata, .data, .tls
    ├── Resolve symbols (global_syms)
    ├── Build import tables (IAT for PE)
    ├── Apply relocations
    ├── Generate base relocations (ASLR)
    └── Write executable headers + section data
```

## When to Use

| Scenario | Use lpp-link? |
|----------|--------------|
| Zero-dependency install | ✅ Yes |
| Smallest binary size | ✅ Yes |
| Cross-compilation | ✅ Yes |
| Need full libc (networking, threads) | ❌ Use cc/cl.exe |
| Debug symbols (DWARF/PDB) | ❌ Not yet supported |
