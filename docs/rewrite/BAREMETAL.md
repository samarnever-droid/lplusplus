# L++ on bare metal — what it would take

_Answering: "if L++ has to run on bare metal, what has to be done?"_

"Bare metal" = no OS, no libc, no `std`: a kernel, bootloader, unikernel, or
microcontroller. Code owns the machine — there is no `malloc`, no `stdout`, no
threads, no files, no dynamic loader, no crt0 `main`.

## TL;DR

The compiler's **front and middle end are already portable** — parser, HIR,
name resolution, type checker, MIR, and the whole **ownership/ARC analysis**
(SCC cycle detection, Frame / Owned / Shared placement) are pure and never
touch the OS. Bare-metal work is concentrated in exactly three places:

1. **A new gated target** (`x86_64-unknown-none` / `aarch64-unknown-none` /
   `thumbv*-none-eabi`) that rejects every OS-dependent builtin — reusing the
   same per-target allow-set machinery the `wasm` target already uses to reject
   file/simd/spawn.
2. **A `#![no_std]` runtime** built on `core` + `alloc`: replace libc
   `calloc`/`free` with a `#[global_allocator]`, replace `printf`/`puts` with a
   board output hook, and add a `#[panic_handler]`.
3. **Freestanding linking**: static link (no `ld-linux`, no libc), a linker
   script (memory map), a `_start`/startup shim, and the `compiler-builtins`
   `mem*`/soft-float intrinsics Cranelift/LLVM call.

Nothing in the type system, ownership model, or MIR changes.

## Why it doesn't work today (grounded in the tree)

* **Runtime is hosted.** `crates/lpp-runtime/src/io.rs` uses
  `libc::{printf, puts, fflush, write}`; `arc.rs` allocates with
  `libc::{calloc, free}`. That is the C reference ABI — correct for a hosted
  build, unusable with no libc.
* **Linking is dynamic + OS.** `crates/lpp-driver/src/compile.rs` links the
  object against the runtime **cdylib** `liblpp_runtime.so` through the host
  **dynamic linker** (`/lib64/ld-linux-x86-64.so.2`) plus `libm`. Bare metal
  has no loader and no shared objects.
* **Targets are hosted only.** `crates/lpp-codegen-api/src/target.rs` defines
  `X86_64`, `Aarch64`, `Wasm32Wasi`. There is no freestanding target, so the
  OS-dependent builtin families (file, net, subprocess, env, clock, RNG,
  spawn/threads, GUI) are never gated out.

## The concrete plan

### 1. Add a freestanding target + gate the builtin surface
* Add `Target::BareMetal { .. }` (or `NoneElf`) to `target.rs` with the triples
  above; wire it into each backend's `targets()`.
* Gate builtins by target the way `wasm` already does: bare metal **rejects**
  the entire OS surface — `file_*`, `net_*`, subprocess/`command_*`, `env_*`,
  wall-clock time, OS RNG, `spawn`/tasks, GUI/webview. These become
  `UnrepresentableBuiltin` on this target (a *feature*, not a bug: the harness
  already knows how to score target-specific rejections — see the
  `tests/wasm/reject/*` handling).
* Left standing: all pure compute — arithmetic, bit ops, strings, lists, maps,
  slices, tuples, ARC — everything that only needs a heap and CPU.

### 2. Split the runtime into `hosted` vs `no_std`
* Make `lpp-runtime` `#![cfg_attr(not(feature = "hosted"), no_std)]` with
  `extern crate alloc`. `default = ["hosted"]` keeps today's libc build; a
  `bare` feature drops libc.
* **Allocator.** Provide a `#[global_allocator]` — a linked-list/bump/buddy heap
  over a static region or a board-supplied RAM range. The ARC core keeps its
  header layout; only `calloc`/`free` become `alloc::alloc_zeroed`/`dealloc`.
  This is exactly where L++'s Owned/Shared/Frame placement lands.
* **Output.** `print`/`print_str` call a weak, board-overridable
  `lpp_putbytes(ptr, len)` sink — UART/serial, a framebuffer, or ARM
  semihosting — instead of `puts`. No formatting depends on libc (`itoa`/`ftoa`
  in-crate).
* **Panic.** Add `#[panic_handler]`; build `panic = "abort"` (no unwinding, no
  landing pads). On panic: halt/loop or trigger reset. Destructors do not run on
  panic — documented, acceptable for freestanding.

### 3. Freestanding link + startup + intrinsics
* Link **statically**: L++ object + runtime **staticlib** (`.a`) with
  `-nostdlib -static` and a **linker script** describing the memory map
  (FLASH/RAM, `.text/.rodata/.data/.bss/.stack`, entry `_start`).
* **Startup shim** (`_start`): set the stack pointer, zero `.bss`, copy `.data`
  from FLASH to RAM, then call the generated L++ entry; on return, halt/reset.
* **Compiler intrinsics.** Cranelift and LLVM emit calls to
  `memcpy/memset/memmove` and integer/float helpers. Link
  `compiler-builtins` (or hand-written `mem*`) so those resolve without libc.
* Teach `lpp-driver` a bare-metal link mode: drive `rust-lld` with
  `-T board.ld -nostdlib`, instead of the in-process hosted-cdylib path.

## The one real constraint: ISA coverage
* **Big cores are fine on Cranelift.** `x86_64-unknown-none` and
  `aarch64-unknown-none` (a kernel, a Raspberry Pi bare-metal image, a
  unikernel) are covered by the existing Cranelift backend — just a new triple
  and freestanding flags.
* **Microcontrollers need the LLVM backend.** Cranelift has **no 32-bit ARM
  Thumb (Cortex-M) backend**. For `thumbv7em-none-eabi` etc. route through
  `crates/lpp-codegen-llvm`, which reaches those targets via LLVM (with
  soft-float where there's no FPU). So: Cranelift for bare-metal-on-big-cores;
  LLVM for MCUs.

## Also deferred on this target
* **Concurrency.** `spawn`/tasks assume OS threads or a scheduler. Bare metal
  either rejects `spawn` (target gate) or later ships a cooperative
  single-core executor / interrupt-driven runtime.
* **Float without an FPU.** Soft-float ABI + float libcalls from
  `compiler-builtins` on MCUs; native on x86_64/aarch64.

## Why L++ is actually a *good* fit for bare metal
The ownership model was built for this without knowing it: **non-escaping
values live in a Frame arena (no heap at all)**, acyclic values are freed
**deterministically** (recursive free, no GC, no pauses), and **ARC is paid
only where a reference cycle is structurally possible** (detected statically by
Tarjan SCC in `crates/lpp-ownership`). That is precisely the memory discipline
embedded and kernel code wants: predictable, allocation-light, pause-free.

## Phasing / cost
1. `no_std` runtime + global allocator + `putbytes` + panic handler — the bulk.
2. Bare-metal target variant + builtin gating — small, mirrors `wasm`.
3. Static link mode + linker script + `_start` + `compiler-builtins` — medium,
   board-specific.
4. First milestone: an `x86_64-unknown-none` "hello + arithmetic + lists/maps"
   image printing over serial — no files, net, or spawn. Then LLVM/Cortex-M.
