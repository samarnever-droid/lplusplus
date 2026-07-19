# L++ Native Linker Roadmap

## Goal

Remove the **user-visible host C linker dependency** from the normal L++ AOT build path while preserving portable native executables.

Current path:

```text
L++ source → Cranelift object → host C compiler/linker + lpp_runtime.c → executable
```

Target path:

```text
L++ source → Cranelift object + L++ runtime objects → lpp-link → executable
```

## Why this is a major project

A production-quality native linker must support platform executable formats and startup ABIs:

| Platform | Format | Required work |
|---|---|---|
| Linux | ELF | program headers, dynamic section, relocations, PLT/GOT, `_start`, libc/pthread imports |
| Windows | COFF/PE | import tables, relocations, CRT entry point, subsystem metadata, DLL imports |
| macOS | Mach-O | load commands, dyld bindings, code signatures where required, system-framework imports |

Cranelift emits relocatable object files; it intentionally does not solve all executable-linking responsibilities.

## Delivery strategy

### Phase 0 — Measure the current problem

King 20 and scalability reports now record compiler, AOT, and host-link time separately. Cross-language comparison also reports L++ AOT compile and host-link time separately.

### Phase 1 — Bundled runtime objects

Build and package platform runtime objects during release:

```text
lpp_runtime_linux_x86_64.o
lpp_runtime_windows_x86_64.obj
lpp_runtime_macos_arm64.o
```

This removes runtime C compilation from normal user builds, but still uses a system linker.

### Phase 2 — `lpp-link` ELF MVP

Support Linux x86-64 ELF first:

```text
Cranelift object + bundled runtime object
    ↓
lpp-link
    ↓
ELF executable
```

Initial scope:

- executable ELF headers and load segments
- symbol resolution for L++ objects
- static section layout and relocations
- minimal `_start` / `main` bridge
- explicit libc, pthread, and dynamic loader imports

### Phase 3 — Runtime migration

Move core runtime functionality from `lpp_runtime.c` to native runtime objects:

```text
ARC
List
closure destructors
basic I/O
```

Network, threads, file I/O, and JSON can remain platform-shim imports until their native object implementations are ready.

### Phase 4 — COFF/PE

Implement Windows support only after ELF correctness and repeatable tests are stable.

### Phase 5 — Mach-O

Implement macOS support only after the Linux and Windows artifact formats are stable.

## Correct success criteria

The linker milestone is complete only when:

```text
[ ] `lpp build` produces a native executable without invoking cc/gcc/clang
[ ] King 20 runs through lpp-link
[ ] output and exit status match host-linked AOT executables
[ ] relocations are tested
[ ] system library imports are explicit and verified
[ ] Linux ELF is stable before COFF/PE and Mach-O expansion
```

## Non-goals

- Do not claim cross-platform direct linking before platform tests exist.
- Do not replace `cc` with a hidden `ld` command and call the dependency removed.
- Do not optimize linker time before executable correctness, relocation correctness, and startup ABI correctness are proven.
