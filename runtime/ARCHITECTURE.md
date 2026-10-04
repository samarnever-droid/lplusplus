# L++ Runtime Architecture — Scalable Design

## Problem

Adding a new builtin currently requires editing 3 files:
1. `src/builtins.rs` — declare the builtin in the compiler
2. `lpp_runtime.c` — implement it for the host linker path (uses libc)
3. `runtime/windows_x86_64_min.c` — implement it for Windows freestanding
4. (future) `runtime/linux_x86_64_min.c` — implement it for Linux freestanding

The **linker (`lpp-link`) never needs editing** for new builtins. It just links
whatever symbols the object files contain.

## Solution: Layered Runtime

```
Layer 3: stdlib/        ← Pure L++ standard library (str_upper, math, etc.)
                          NO C code. NO linker changes. Just .lpp files.

Layer 2: runtime/       ← Platform runtime in C (one file per platform)
         unified.c        OS-specific I/O, networking, process calls.
                          Compiled differently per target:
                          - cc -DLPP_HOST    → uses libc (host linker path)
                          - cc -DLPP_WIN_FS  → freestanding Windows (Kernel32 only)
                          - cc -DLPP_LIN_FS  → freestanding Linux (syscalls only)

Layer 1: lpp-link       ← Direct linker. NEVER edited for new builtins.
                          Only edited for new linking MODES (shared libs, etc.)
```

## Adding a new builtin — decision tree

```
Is it pure computation (no OS calls)?
  YES → Write it in L++ as stdlib/math.lpp or stdlib/strings.lpp
        No C code needed. No linker changes.

Does it need OS calls (file, network, process)?
  YES → Add C function to runtime/unified.c with #ifdef guards:
        #ifdef LPP_HOST
          // Use libc: fopen, socket, etc.
        #elif defined(LPP_WIN_FREESTANDING)
          // Use Kernel32: CreateFile, WSASocket, etc.
        #elif defined(LPP_LIN_FREESTANDING)
          // Use syscalls: SYS_open, SYS_socket, etc.
        #endif

Does it need a new linking mode?
  YES → Edit lpp-link (rare: dynamic libs, plugins, etc.)
```

## File layout

```
stdlib/
  strings.lpp      ← str_upper, str_lower, str_pad, str_repeat
  math.lpp         ← abs, min, max, clamp, sqrt (wraps builtin)
  collections.lpp  ← stack, queue, deque (built on List[T])
  fmt.lpp          ← string formatting, interpolation

runtime/
  unified.c        ← ALL platform runtime in one file with #ifdef
  lpp_net.c        ← Networking (optional, larger)
  ARCHITECTURE.md  ← This file
```
