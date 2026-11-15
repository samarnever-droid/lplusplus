# Current capabilities

**Last updated for v0.3.0 (`v0.3-dev`);** remaining historical gate counts below reflect their dated snapshots unless marked current. This file is a compact index; current roadmap status is tracked in [ROADMAP.md](../ROADMAP.md) and `v0.3.0` release notes in [V0.3.0.md](release/V0.3.0.md).

## Compiler

| Capability | Status |
|---|---|
| Python-readable indentation syntax | Available |
| Functions, defaults, loops, break/continue, ranges | Available |
| Structs, enums, exhaustive match, `?`, f-strings | Available |
| Generics and turbofish | Available and monomorphized |
| Traits with static/dynamic dispatch | Available |
| Principled `unsafe` & Kernel Primitives | Available: `unsafe` blocks/functions, `Ptr[T]`, `RawPtr`, casts, volatile MMIO, port I/O, CPU control registers, `--deny-unsafe` (`E4501`), `--multiboot2`, `--image-base` |
| Bare-metal targets | Available: `x86_64-unknown-none`, `aarch64-unknown-none` + static ELF kernel linking (`lpp-link`) |
| FFI / `extern "C"` | Cranelift supports direct calls for the documented scalar/`Str` subset, including imported declarations across modules and host-library linking. The system SQLite and zlib binding smokes pass. See [`EXTERNAL_C_ABI.md`](EXTERNAL_C_ABI.md). |
| Native optimization | Cranelift respects `-O0`–`-Oz`; LLVM backend now supports `-O0`–`-O3`, `-Os`/`-Oz`, managed ARC, kernel primitives, and SIMD. `--lto` enables the bounded whole-program MIR inliner; `--pgo-generate`/`--pgo-use` profile function entries on Cranelift. |
| Closures and thread spawn | Available |
| Fixed structural tuples | Available up to arity 16, managed elements, destructuring; Cranelift and LLVM full parity |
| Typed variadics | Available: final owned rest `List[T]`; extern C varargs rejected |
| Borrowed slices | Available: zero-copy `StrSlice` / `Slice[T]`, conservative non-escape rules |
| Async/await | Single-thread run-to-completion executor (`async def`, `.await`, `spawn`) |
| Collections & builtins | `List[T]`, `Map[K, V]`, `Set[T]`, `Deque[T]`, strings, buffers, files, networking, JSON, 100+ builtins |
| DWARF debugging & backtraces | Available: `-g` / `--debug` (Cranelift & LLVM), `lpp debug` CLI, runtime stack backtraces |
| Developer CLI & LSP | Available: `lpp fmt`, `lpp repl`, `lpp test`, `lpp debug`, `lpp-lsp` |
| Package tooling | Keel PM (`Keel.toml`, `Keel.lock`), Git registry (`llppregistry`), and `lpp migrate` / `keel migrate` for legacy `lpp.toml` |
| `c2lpp` package | Phase 1 C-header/native-package generation complete; Phase 2 audited scalar translation started |

## Ownership

- MIR and `lpp-ownership` are the single source of escape/storage truth.
- `Frame < Owned < Shared` is the escape lattice.
- Non-escaping `Frame` POD structs and tuples lower onto the stack (`StackSlot` in Cranelift, `alloca` in LLVM) with zero heap allocations and zero ARC retain/release traffic.
- Escaping `Owned` POD structs, enums, and tuples pass a null destructor to `lpp_arc_alloc_with_destructor`, skipping the 8-byte destructor prefix and indirect destructor call at teardown.
- Self-referential structs and cycle-capable aggregates are classified `Shared` and collected on Cranelift and LLVM native targets via Bacon-Rajan trial-deletion in `lpp-runtime`. WASM recycles freed acyclic blocks but does not yet collect unreachable cycles. (See `docs/CYCLE_BREAKING.md` and `ROADMAP.md`.)
- Closures use ARC environments; non-escaping closure capsules can be stack-resident.
- Tuples up to arity 16 and async tasks are ARC aggregates with runtime child/environment teardown (or stack-placed when `Frame` POD).
- Slice view records are stack-resident and borrowed; the MIR validator prevents first-tier escapes and the runtime validates bounds/generation at each access.

## Backends and linkers

| Combination | Status |
|---|---|
| Cranelift + host linker | Production/default (`x86_64`, `aarch64`); stack-placed `Frame` POD, destructor-free POD `Owned`, cycle collector, DWARF `-g` |
| Cranelift + direct ELF/PE/Mach-O linker | Production for host native targets and static bare-metal `*-unknown-none` ELF kernels |
| LLVM + host/direct linker | Full MIR/ARC/kernel/SIMD parity (`x86_64`, `aarch64`), `-O0`–`-O3`, DWARF `-g`, stack `Frame` POD & null-dtor `Owned` POD |
| WASM | Experimental (`wasm32-wasip1`); ARC size-class free-list reuse passes bounded acyclic churn; WASI filesystem builtins lowered via WASI preopens; cycle collection missing |
| Windows LLVM execution | Needs Windows CI validation |

Select the optional backend explicitly:

```sh
lpp app.lpp --backend llvm --linker direct
```

There is no Turbo mode in the current repository. Cranelift remains the fast
compile-time default; LLVM is the optional optimization backend.

## Explicit vectors

The common API supports `VectorI64x2` construction, splat, add, subtract,
multiply, XOR, constant right shift, lane extraction, and horizontal sum. The
`vec_i64_checksum` runtime intrinsic now auto-dispatches to a four-lane
256-bit AVX2 kernel on capable x86-64 CPUs and falls back to a scalar reference
elsewhere. This is a workload-specific SIMD intrinsic, not general loop
auto-vectorization or a source-level `VectorI64x4` type; see
[`benchmarks/simd/results.md`](../benchmarks/simd/results.md).

## MIR inlining and PGO

`--lto` opts into the whole-program MIR inliner: it only substitutes direct,
one-block, tiny, pure scalar callees and avoids effects, calls, stores, and
managed values. It operates after L++ modules have been merged into MIR; it is
not object-file, runtime, or foreign-library LTO.

PGO generation is available for Cranelift builds targeting the compiler host:

```sh
lpp app.lpp -O3 --pgo-generate -o app-instrumented
LPP_PGO_PROFILE=app.pgo ./app-instrumented
lpp app.lpp -O3 --pgo-use app.pgo -o app-pgo
```

If `LPP_PGO_PROFILE` is unset, the runtime writes `lpp.pgo` in the process
working directory. The versioned profile stores MIR function-entry counts and
a fingerprint of the pre-optimization program; `--pgo-use` rejects malformed or
mismatched profiles. PGO selects hot callees for the same bounded MIR inliner;
it does not collect edge/value profiles or guide backend machine-code passes.
The runtime uses relaxed atomic counters in a dense table (IDs below 1,048,576)
and flushes it when source `main` returns normally. Profile generation adds
per-entry instrumentation overhead; see the benchmark for a measured example.

## Verified gates (`v0.3-dev`)

- `bash scripts/fast_gate.sh`: **PASS** — `cargo fmt --check`, workspace
  Clippy, **872** workspace tests (0 failed, including the `v03_*` production
  suites across all 14 core crates), **37** bindgen source modules across 4 entry
  graphs, focused bindgen ASan/LSan/UBSan regression, and **58/58** AOT parity.
- `bash scripts/corpus_correctness.sh run` (`193/193` = `100.0%`, 180 run + 13 expected rejects; 18 skipped) and
  `python3 scripts/check_compatibility_freeze.py` (`112/112` unchanged): **PASS**.

## Not yet claimed

- Project-wide “100% feature freeze”; async I/O reactor and static `Send`/`Sync` enforcement remain open.
- WASM cycle collection or bounded memory for cyclic WASM workloads.
- General lifetime-parameterized slices, mutable slices, or slice thread send.
- Unsafe native C variadic ABI or tuples beyond arity 16.
- Windows LLVM runtime execution.
- General source-level automatic vectorization of arbitrary loops or a source-level wider SIMD type.
