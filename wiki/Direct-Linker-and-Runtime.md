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

## Runtime cache

The freestanding runtime is compiled once from C source and cached as a native object file. Subsequent builds reuse the cached object — no C compiler needed after the first build.

### Multi-arch cache layout

```text
LppData/cache/
    linux-x86_64/
        lpp_runtime_min.o
        runtime.hash
    linux-aarch64/
        lpp_runtime_min.o
        runtime.hash
    windows-x86_64/
        lpp_runtime_min.obj
        runtime.hash
    macos-arm64/
        lpp_runtime_min.o
        runtime.hash
```

Each target architecture gets its own cache directory, preventing cross-platform overwrites.

### Hash-based invalidation

Cache validity is checked using a **content hash** of the runtime C source file, not timestamps. This is reliable across:

- Git checkouts (which reset timestamps)
- ZIP extraction
- File copies between machines
- Different filesystems

The hash is stored in `runtime.hash` beside the cached object. When the source content changes, the hash misses and the runtime is recompiled automatically.

### Release installs

Users who install L++ from a release package receive a pre-compiled runtime object in `lib/`. They never need a C compiler — `lpp` and `lpp-link` are fully self-contained.

## Windows PE linker note

The PE linker has support for COFF sections, imports, relocations, and freestanding Kernel32-based binaries. It should not be modified for normal language features.


## Inspect objects

Use `inspect` to debug object files before linking:

```bash
lpp-link inspect file.o
lpp-link inspect file.obj
```

This helps diagnose unresolved symbols, COFF section layout, and relocation problems.
