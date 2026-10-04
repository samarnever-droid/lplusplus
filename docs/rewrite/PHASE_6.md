# Rewrite Phase 6 — linker and runtime

## Rewrite roadmap (Phases 0–7)

1. **Phase 0 — secure and freeze** — complete.
2. **Phase 1 — workspace and common contracts** — complete.
3. **Phase 2 — frontend and modules** — complete.
4. **Phase 3 — HIR, types, generics, and traits** — complete.
5. **Phase 4 — MIR and ownership** — complete.
6. **Phase 5 — backends** — active: `lpp-codegen-api` plus the Cranelift
   backend (5A contract, 5B scalars/CFG/calls done; 5C aggregates next).
7. **Phase 6 — linker and runtime** — **active: 6A (linker crate) and
   6A.2 (typed error boundary) and all of 6B (the runtime crate — ARC,
   lists, slices, numeric, string, IO, tasks, tuple, vec) are done; the
   full 95-symbol c_shim surface is ported and differentially verified.
   Phase 7 (CLI/PM/LSP + driver cutover) is next.**
8. **Phase 7 — CLI, PM, LSP, and cutover** — planned.

The root compiler remains routed through `LegacyEngine` until the Phase 7
cutover. Phase 6 starts by relocating the production direct linker out of
the legacy monolith so the rewrite crates own the object→executable
boundary, with the proven engine behavior preserved byte-for-byte.

## Approved Phase 6 architecture

The stage boundary continues from Phase 5:

```text
verified, deterministic object files (Phase 5 exit)
  -> lpp-linker contract (6A: relocatable objects + archives -> executable)
  -> native ELF / PE / Mach-O image, entry resolved, relocations applied
```

One-way dependencies: `lpp-linker` depends on **nothing in the rewrite
crate tree** (it consumes object bytes, not MIR) — only the already
approved `object 0.36` crate (the same version the root compiler pins in
`Cargo.lock`; no new external crates). It may never import `lpp-mir`,
`lpp-types`, or any backend; Phase 7 wires the driver to it.

**Non-negotiable rules:**

- **Behavior-preserving port (6A).** The production linker (v1
  `src/linker.rs`, ~6.9k LOC: ELF/PE/Mach-O writers, archives, TLS,
  dynamic tables, PE base relocations, AArch64 patching) is proven
  byte-level in every v1 build. 6A moves it verbatim into the crate;
  no layout math changes in this slice.
- **Typed boundary.** The engine already carries a `LinkError`
  (message + unresolved symbols) at its error sites. The modern entry
  `link_typed` returns it directly with a `LinkReport`; 6A.2 completes
  the boundary by threading `LinkError` through the internal
  `Result<_, String>` signatures so the unresolved list survives to
  callers (still zero behavior change).
- **Deterministic output.** Same inputs and options produce
  byte-identical images (existing `build_id` is content-derived, not
  time-derived); asserted by the gates.
- **Gates must be hermetic** where possible: hand-crafted relocatable
  objects link and **execute** (real exit code), so the gate needs no
  external toolchain. A `cc`-based multi-object/relocation test is
  guarded and skips cleanly when no C compiler is present.

## Phase 6 slices

### 6A — `lpp-linker` crate (engine relocation) — done

New crate `crates/lpp-linker` (dependencies: `object = "0.36"` only).

**Scope**

- The full v1 direct-linker engine ported verbatim into
  `lpp_linker::core` (private module): ELF/PE/Mach-O output writers,
  object + static-archive input via `object`, symbol resolution and
  merging, relocation patching (x86-64 + AArch64), TLS variants,
  PE checksum and base relocations, response files, `inspect`, format
  sniffing, and the `link_cli` surface.
- **Public API = the proven v1 surface, unchanged:**
  `write_elf{,_with_options}`, `write_pe{,_with_options}`,
  `write_macho{,_with_options}`, `link_direct`, `link_with_options`,
  `link_cli`, `inspect_object`, `sniff_format`, `expand_response_files`,
  `usage`, `LinkOptions`, `LinkError`, `OutputFormat`, `Machine`,
  `PeSubsystem`, `DynamicMode`, `LPP_FREESTANDING`.
- **Modern entry (for the Phase 7 driver):**
  `link_typed(inputs, output, options) -> Result<LinkReport, LinkError>`
  with `LinkReport { format, output, output_size, object_count }`.
- **Root monolith shrinks:** `src/linker.rs` becomes a one-line
  re-export shim (`pub use lpp_linker::*;`); the `lpp-link` binary and
  every `pm.rs`/`legacy_driver.rs` call site keep working unchanged.

**Gates** (`crates/lpp-linker/tests/linker_gate.rs`)

1. **Hermetic end-to-end:** a hand-crafted x86-64 relocatable ELF object
   (self-contained `_start` that exits 42) links and **executes** with
   exit code 42 — no lpp, no cc required.
2. **Determinism:** two links of the same input are byte-identical.
3. **Clean errors:** garbage bytes, missing input, and empty input
   fail with a non-empty message, never panic.
4. **Response files:** `@file` expansion (lines, comments, quoted
   tokens) and `link_cli --help`/`version` behavior.
5. **Guarded integration:** with a C compiler present, two real `cc -c`
   objects (function + call site, PC32/PLT32 relocations) link with
   entry `main` and execute with exit code 42.
6. The 13 ported unit tests (align/identity math, ELF hash, SHA-256,
   PE checksum, AArch64 encodings) stay green inside the crate.

### 6A.2 — typed error boundary — done

Thread `LinkError` (message + unresolved symbols) through the internal
signatures so `unresolved` survives to `link_typed` callers; add a
`LinkErrorKind` category (Io / Malformed / UnsupportedFormat /
Unresolved / RelocationOverflow / Usage / Internal). Internal engine
signatures now carry `Result<_, LinkError>` end to end; the proven v1
public surface (`write_*`, `link_with_options`, `link_cli`, …) keeps
its `Result<_, String>` signatures as thin adapters over the new
typed `*_t` entries, so error text is byte-identical. `link_typed`
calls `link_with_options_t` and returns the engine's `LinkError`
unchanged (kind + unresolved list). Gates: the existing gate suite
plus `removed_symbol_survives_as_typed_error` (multi-object link whose
only error is a removed symbol, asserting the exact unresolved list
and the `Unresolved` kind) and `error_kinds_are_classified`
(garbage → Malformed, missing input → Io, empty input → Usage). Zero
behavior change.

### 6B — runtime crate — core complete (full 95-symbol c_shim surface ported)

`lpp-runtime` (or the equivalent in-crate representation of
`lpp_runtime.c` + `lpp_runtime_min.c`): the ARC/arena/list/task runtime
as first-class, testable code — symbol census, layout constants, and
the ABI that 5C/5C2 objects link against. The v1 C runtime remains the
behavioral reference; the crate must emit a symbol-for-symbol identical
table.

**6B.1 — layout + ARC core — done.** New crate `crates/lpp-runtime`
(`rlib` + `staticlib` + `cdylib`), zero deps beyond `libc`:

- `layout.rs` — the header constants, verified against the C struct with
  `offsetof`: `LppArcHeader` is magic@0, refcount@4, generation@8,
  4 bytes padding@12, destructor@16, size 24. The destructor offset is
  16, not 12 — the C compiler 8-aligns the pointer (a wrong 12 here put
  an unaligned 8-byte field at offset 12 and corrupted the header).
- `arc.rs` — the ARC primitives re-implemented with identical ABI and
  memory ordering: `lpp_arc_alloc[_with_destructor]`, `lpp_arc_retain`,
  `lpp_arc_release`, the relaxed thread-local fast path
  (`lpp_arc_retain_local` / `lpp_arc_release_local`), the weak-generation
  protocol (`lpp_weak_generation` / `lpp_weak_get`, generation bumped
  BEFORE free), the immortal empty string (`lpp_empty_str`, sentinel
  refcount), and `lpp_closure_destroy`. Same allocator as C (libc
  `calloc`/`free`) so object identity/recycling matches. The generation
  counter stays internal (C keeps it `static`; objects reach generations
  only through `lpp_weak_generation`), so it is deliberately not part of
  the exported ABI.
- Gates (`tests/runtime_gate.rs` + in-crate `arc::tests`): behavioral
  (retain/release accounting, destructor-exactly-once, weak reject after
  free, local fast path, foreign/NULL tolerance, immortal empty string,
  closure env release); **golden-fingerprint differential** — the same
  ARC scenario run against the v1 C reference and the Rust runtime must
  print the identical fingerprint (both pinned to
  `A genpos=1 live=1 drops=1 dead=1 B drops=1 emptypos=1`); and a
  **symbol census** proving the Rust `staticlib` exports every 6B.1 ABI
  symbol and that each is also defined by the C reference object.
- Two real bugs found and fixed while landing this: (1) the allocator
  initialized the header through the *base* pointer instead of the
  payload, writing 24 bytes before the allocation (heap corruption /
  SIGSEGV) and leaving the real header zeroed (generation read 0,
  destructor never fired); (2) the destructor offset (above).

**6B.2 — lists + slices — done.** `panic.rs` (the C-style banner +
`exit(101)` path, internal — generated objects reach it only through the
runtime's own bounds checks), `list.rs` (`List[T]`: value and ARC-owning
lists, push/get/set for int/float/bool/arc, len/pop/free/reserve/
capacity/clear; the `LppList` payload is `#[repr(C)]`-identical to C —
data@0, len@8, cap@16, retain@24, drop@32, size 40 — and its ARC
destructor frees the inner array while the ARC free releases the outer
block, matching the C ownership split; `set` retains-before-drops for
self-assignment), and `slice.rs` (numeric `Slice[T]`: init/len/get/
get_float/get_bool over a borrowed weak-generation-guarded window;
`LppSlice` is base@0, start@8, length@16, generation@24, kind@32, size
40). Gates: a list/slice/ARC-list **golden-fingerprint differential**
(Rust and the v1 C reference print the identical scenario string,
including the ARC-owning list dropping its element exactly once on
free) and the symbol census extended to all 35 ABI symbols (10 ARC +
20 list + 5 slice), each proven exported by the Rust staticlib AND
defined by the C reference object.

**6B.3a — numeric builtins — done.** `numeric.rs`: the 39 pure-numeric
primitives from `runtime/lpp_int.c` + the math/`abs`/`min`/`max` helpers,
ABI-identical (all `i64`/`f64`, same edge cases — unsigned shifts clamp
the count, `div_u`/`rem_u` panic on zero, `*_checked` panic on overflow,
`*_wrap` wrap, `clz`/`ctz` of zero are 64, truncate/bswap/rotate
reproduce the C unsigned-cast semantics, floats match libm). Gate: an
FNV-fold **golden differential** over all 53 representative calls
(including `i64::MIN`/`MAX`, shift-count clamps, unsigned compares of
`-1`) — the Rust runtime and the v1 C reference produce the identical
64-bit fingerprint; census extended to 63 ABI symbols.

**6B.3b/c — string + IO builtins — done.** `string.rs`: the 18 string
primitives from `runtime/lpp_str.c` + `runtime/lpp_int.c` (concat, find,
replace, trim, contains, starts_with, ends_with, upper, lower, eq, len,
int/float/bool/u64-to-str, str-to-int/u64, u64-to-hex), each returning a
NUL-terminated ARC `char *` (immortal empty string on alloc failure, as
C does) with float formatting via libc `snprintf("%g")` so it is
byte-identical to the C reference; plus the two string-slice reads
(`lpp_str_slice_get`/`_to_str`) in `slice.rs`. `io.rs`: the 6 console
primitives (print_int/float/bool/str, write_str, eprint_str) matching
the C formats exactly (`%lld\n`, `%f\n`, `%d\n`, `puts`, raw write,
stderr). Gates: an FNV-fold **golden differential** over all 20 string
calls (Rust == the v1 C reference fingerprint) and an **IO differential**
that captures the C scenario's stdout/stderr and the Rust runtime's
(fd-redirected) stdout/stderr and asserts they are byte-identical;
census extended to 89 ABI symbols, with the shim-only `lpp_eprint_str`
(absent from `lpp_runtime.c`) verified against `c_shim.c`.

**6B.3d — tasks + tuple + vec checksum — done (completes the c_shim
surface).** `task.rs`: the single-threaded task runtime (new/poll/await/
destroy) with the identical atomic state machine (0 pending / 1 running /
2 complete, acq_rel CAS, idempotent double-poll, run-to-completion
executor), the `LppTask` payload `#[repr(C)]`-identical to C (code@0,
environment@8, result@16, state@24, result_managed@28; size 32), owning
its environment and — when `result_managed` — one reference to its
result, both released by the ARC destructor. `tuple.rs`: the structural
tuple (`lpp_tuple_alloc` + the 4-slot managed-mask/packed-offsets
destructor), `LppTuplePrefix` repr(C)-identical (size 16). `numeric.rs`:
`lpp_vec_i64_checksum` (the scalar reference for the SIMD intrinsic —
vectorization does not change the value). Gate: an FNV-fold **golden
differential** over a managed scenario (int-result task, managed-result
task with caller retain + release-on-destroy drop accounting, a tuple
releasing its managed child, and six vec-checksum values) — Rust == the
v1 C reference fingerprint; census extended to the **full 95-symbol
`c_shim.c` surface** (verified: zero c_shim symbols missing from the
Rust staticlib).

**6B core is complete** — the entire ABI that 5C/5C2 objects link
against (`c_shim.c`'s 95 symbols) is now first-class, differentially
verified Rust. Remaining optional work: arena regions (internal to the
full runtime; not part of the c_shim link surface) and the freestanding
`lpp_runtime_min.c` variants (no-libc builds).

**6B.3e — drop-in link capstone — done.** The end-to-end proof that the
Rust runtime *replaces* the C runtime rather than merely matching it in
isolation: the 5C gate compiles each proven corpus
(`AGGREGATE_CORPUS`, `ARC_STRESS_CORPUS`) to a single object and links
that **same object** two ways — against `c_shim.c` (the v1 C reference)
and against the Rust `lpp-runtime` **cdylib** (`liblpp_runtime.so`,
built on demand by the gate) — asserting byte-identical stdout, a clean
exit, and the success markers from each. The cdylib is self-contained
(107 exported `lpp_` symbols, zero undefined std symbols), so the C
object needs nothing but `-L target/debug -llpp_runtime -Wl,-rpath,…`.
Gate: `rust_runtime_is_a_drop_in_for_the_c_shim` in `phase5c_gate.rs`.

## Status

- **6A — done** (this document's slice above; commit follows).
- **6A.2 — done** (typed boundary; `LinkErrorKind` + `*_t` entries;
  unresolved list survives to `link_typed`).
- **6B.1 — done** (layout + ARC core; golden-fingerprint differential
  against the v1 C reference + symbol census).
- **6B.2 — done** (panic path + `List[T]` + numeric `Slice[T]`;
  list/slice golden differential + 35-symbol census).
- **6B.3a — done** (39 numeric builtins; FNV golden differential over
  53 calls + 63-symbol census).
- **6B.3b/c — done** (18 string + 2 string-slice + 6 IO builtins;
  string golden differential + IO stdout/stderr differential +
  89-symbol census incl. shim-only eprint_str).
- **6B.3d — done** (tasks, tuple, vec-checksum; managed golden
  differential + the full 95-symbol c_shim census — zero missing).
- **6B.3e — done** (drop-in link capstone: the same compiled object
  links against `c_shim.c` *and* the Rust cdylib with byte-identical
  stdout — `rust_runtime_is_a_drop_in_for_the_c_shim`).
- **6B core complete + drop-in proven.** Optional remainder: arena
  regions (internal) + freestanding min-runtime variants.
