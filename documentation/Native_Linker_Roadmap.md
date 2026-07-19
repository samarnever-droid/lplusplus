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

### Phase 1 — Bundled runtime objects — implemented

Installers now package a platform runtime object:

```text
Linux:   ~/.lpp/lib/lpp_runtime.o
Windows: ~/.lpp/lib/lpp_runtime.obj
```

Installed `lpp build` prefers this object and links it with the generated Cranelift object. It no longer recompiles `lpp_runtime.c` for each user project build. The source runtime remains a development fallback.

Verified with a temporary installed-layout integration test:

```text
installed lpp → lpp new → lpp build → lpp run
```

This still uses a host linker; it removes only repeated runtime C compilation.

### Phase 2 — `lpp-link` ELF MVP — started

A new Rust binary, `lpp-link`, now emits a real Linux x86-64 ELF executable without invoking `cc`, `gcc`, `clang`, or `ld`.

Current validated scope:

```text
multiple x86-64 ELF objects
+ merged .text sections
+ internal PC-relative calls
+ GOTPCREL runtime imports
+ generated Linux _start syscall exit stub
+ freestanding syscall runtime (integer/string output)
= static ELF executable without host final link
```

The integration test verifies both:

```text
runtime-free source → Cranelift object → lpp-link → ELF executable → exit 0

fib(35) source + lpp_runtime_min.o → lpp-link → ELF executable → 9227465
```

Next Phase 2 increments are:

- merge packaged full runtime objects
- support `.rodata`, `.data`, `.bss`, and relocations beyond internal calls/GOT
- extend the freestanding runtime for ARC, lists, and closures
- add explicit dynamic libc/pthread imports for networking/files/threads
- preserve the host-link fallback until King 20 runs through lpp-link

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
