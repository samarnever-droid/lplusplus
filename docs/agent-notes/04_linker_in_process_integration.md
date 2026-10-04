# Wiring the in-process linker into the compile pipeline

_Goal: make `lpp-linker` (the pure-Rust ELF/PE/Mach-O linker) the actual linker
the rewrite driver uses, instead of shelling out to `cc`. "No external tools" is
the project's identity; before this change it was aspirational (a `TODO` comment)._

## Starting state
- `crates/lpp-driver/src/compile.rs::link_executable` shelled out to `cc`
  (`-llpp_runtime -Wl,-rpath ... -lm`). The in-process `lpp-linker` crate was
  **not even a dependency** of the driver.
- `--linker direct` on the rewrite path was silently ignored (still used `cc`).
- The direct linker *could* produce a structurally valid dynamic ELF, but the
  resulting executable **segfaulted**.

## Bugs found & fixed (ELF, x86_64)

### 1. `_start` bypassed `__libc_start_main` (crash before main)
`emit_elf_start_stub_x64` synthesized a `_start` that called `main` **directly**,
even for glibc-hosted dynamic images. glibc (and the Rust runtime cdylib that
depends on it) is only fully initialized by `__libc_start_main` — TLS, `errno`,
stdio buffers, `__environ`, the stack guard. Calling `main` directly left libc
half-live → crash.

**Fix:** added a `StartupAbi` enum. Hosted-dynamic images now emit the
`Scrt1.o`-equivalent bootstrap:
`__libc_start_main(main, argc, argv, init=NULL, fini=NULL, rtld_fini, stack_end)`
(glibc ≥ 2.34 runs `init_array` itself, so init/fini are NULL). `main` is passed
via a RIP-relative `lea`; the `call __libc_start_main@plt` is patched
post-layout through the PLT (generalized the old `injected_exit_reloc` into a
`startup_plt_calls: Vec<(&'static str, usize)>`). Freestanding/static images keep
the raw `exit(2)` syscall stub.

### 2. PIE GOT slots had no dynamic relocation (crash on first runtime call)
The codegen calls runtime fns via `R_X86_64_GOTPCREL` (`mov rax,[rip+got];
call *rax`). The linker filled that `.got` slot with the **link-time** PLT-stub
VA but emitted **no `R_X86_64_RELATIVE`** for it. Under PIE/ASLR the load base is
never added, so the slot points into unmapped low memory → `call *0x1070` → SEGV.

**Current resolution:** the driver links **non-PIE (`ET_EXEC`)**, where link-time
VAs are already the final addresses, so no GOT `RELATIVE` relocs are needed. This
is proven working across the whole corpus.
**Next hardening step (tracked):** emit `R_X86_64_RELATIVE` (and
`R_AARCH64_RELATIVE`) for GOT slots + a proper `.rela.dyn`/`DT_RELACOUNT`, then
flip the default back to PIE.

### 3. No `DT_RUNPATH` → produced exes couldn't find `liblpp_runtime.so`
Added an `rpath: Vec<String>` field to `LinkOptions`, a `DT_RUNPATH` (tag 29)
dynamic entry (string interned in `.dynstr`, entry count bumped like
`DT_SONAME`), and `-rpath`/`--rpath` to the `lpp-link` CLI. The driver embeds a
`DT_RUNPATH` into the runtime lib dir, so images run **without**
`LD_LIBRARY_PATH` — matching `cc -Wl,-rpath`.

## Driver integration (`compile.rs`)
- Added `lpp-linker` as a dependency.
- `link_executable` now calls `lpp_linker::link_typed` with
  `dynamic=Force, pie=false, dynamic_linker=<host loader>,
  needed=["liblpp_runtime.so"]` (libc/libm auto-derived from undefined symbols),
  `rpath=[runtime_lib_dir]`.
- **Fallback policy:** genuine user/input errors (missing `main`, malformed
  object, bad args → `LinkErrorKind::{Unresolved,Usage,Io,Malformed,
  RelocationOverflow}`) are reported *cleanly* — no `cc`, no `collect2` dump.
  Only `Internal`/`UnsupportedFormat` (a real in-process limitation) fall back to
  `cc`, with a visible stderr warning so gaps are never silently masked.
  `LPP_LINKER=cc` forces the old path.
- Side benefit: the 4 former no-`main` "link failures" now emit
  `required symbol 'main' ... not found` instead of a `collect2` dump.

## Verification
- `cargo test`: lpp-linker 15 unit + 10 `linker_gate` ok; lpp-driver 2 + 4
  `driver_gate` ok; cranelift/runtime unchanged (green).
- Corpus `scripts/corpus_correctness.sh run`: **PASS=116/195 (59.5%)**, no
  regression, **0 `cc` fallbacks / 0 limitations** — every corpus program that
  reaches the link stage is linked by the in-process linker.
- Standalone proof: `io_demo`, `test_new_builtins`, `pint` built via the driver
  run correctly with `LD_LIBRARY_PATH` unset (resolved via `DT_RUNPATH`).
- Output validated with standard tools (`readelf -d/-r/-l`, `objdump -d`, `file`,
  `ldd`, `LD_DEBUG`).

## Follow-ups (the "better and better" backlog)
1. PIE support: `R_*_RELATIVE` GOT relocs + `.rela.dyn` + `DT_RELACOUNT`, flip
   default to PIE.
2. aarch64 hosted-glibc `_start` (mirror the x86_64 `__libc_start_main` stub).
3. Fuzzing: feed malformed/random objects to `link_typed`; assert clean typed
   errors, never a panic. Cross-check every produced image against `readelf`/`ld`
   validators.
4. Cross-OS proof: exercise the PE and Mach-O writers end-to-end.
5. Debug info: carry `.debug_*` sections / emit a minimal symbol table so
   produced exes are debuggable.
