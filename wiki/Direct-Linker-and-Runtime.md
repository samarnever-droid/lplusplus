# Direct Linker and Runtime

L++ has two ways to produce executables.

## Host linker path

Uses the platform C toolchain:

- Linux/macOS: `cc` or `clang`
- Windows: `cl.exe` / `link.exe`

This path links against the full host runtime and libc/CRT.

## Direct linker path

Uses L++'s custom linker:

```bash
lpp-link input.o runtime.o -o app          # ELF
lpp-link pe input.obj runtime.obj -o app.exe
lpp-link macho input.o runtime.o -o app
```

Direct linker formats:

| Format | Platform | Status |
|---|---|---|
| ELF | Linux | working |
| PE/COFF | Windows | working, freestanding |
| Mach-O | macOS | working baseline |

## Runtime layers

```text
stdlib/*.lpp            pure L++ helpers
runtime/*.c             platform runtime functions
lpp-link                object/executable linker
```

Adding a new language feature normally changes only compiler files.

Adding a new builtin normally changes:

- `src/builtins.rs`
- host runtime C file
- freestanding runtime C file, if direct linker support is required

It normally does **not** require editing `lpp-link`.

## Freestanding runtime

The direct linker uses minimal runtimes such as:

- `runtime/linux_x86_64_min.c`
- `runtime/windows_x86_64_min.c`

These avoid libc/CRT dependencies and are the reason direct-linked binaries can be very small.

## Windows PE linker note

The PE linker has support for COFF sections, imports, relocations, and freestanding Kernel32-based binaries. It should not be modified for normal language features.
