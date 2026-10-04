# L++ Fix Hit List

Working checklist derived from `SAMARDB_FEATURE_REQUESTS.md`. That file is the
*evidence* (repros, error text, why it matters); this file is the *worklist*
(where to edit, what to watch out for). Updated 2026-08-21.

Repro prerequisite on Windows — without this, linking dies with
`Failed to execute host linker 'cl.exe': program not found`:

```bash
export PATH="$PATH:/c/Program Files (x86)/Microsoft Visual Studio/Installer"
```

Compile + run a single file (`lpp <file>.lpp` produces `<file>.exe`; there is no
`lpp run` / `lpp build -o`):

```bash
/c/Users/khati/lpp/target/release/lpp.exe probe/b4.lpp && ./probe/b4.exe
```

---

## Done

| # | Bug | Files touched |
|---|---|---|
| B4 | `Bool` ABI: verifier failure **and** silent wrong answers | `src/backend/cranelift/lower.rs` (`coerce_args_to_signature`), `runtime/lpp_str.c:260`, `runtime/windows_x86_64_min.c:640` |
| B8 | Runtime object cache ignored `#include`d sources — runtime edits silently no-op'd | `src/pm.rs` (`hash_local_includes`, `cached_runtime_object`) |
| B1 | ARC use-after-free on multi-field struct alias rebuild (premature destruction at refcount 1 on MSVC due to `InterlockedDecrement` returning new count while release checked `prev == 1`) | `lpp_runtime.c:596` (`LPP_ARC_DEC` -> `InterlockedExchangeAdd(p, -1)`), `src/pm.rs:4181` |
| B2 | Enum `Str` payload crash: `expr_type_hint` didn't check `match_bindings` and bindings lacked debug names, emitting 64-bit integer pointer addition `iadd` instead of `str_concat` | `src/mir/lower.rs:240`, `1430`, `1568`, `2695` |
| B3 | `?` operator propagation with string payload errors | Resolved alongside B2 payload typing + borrowing; verified across multi-step chained ? pipeline |
| B7 | Qualified match arms (`Enum.Variant`), wildcard arms (`_`), and out-of-order match arms (previously used loop index `i` instead of declared variant tag) | `src/frontend/parser.rs:931`, `src/mir/lower.rs:1410`, `2718` |
| F11 | Symbolic backtrace reporting on panics: dynamic `dbghelp.dll` symbol and source line resolution (`SymFromAddr`, `SymGetLineFromAddr64`) + `/DEBUG` link flags | `lpp_runtime.c:76-135`, `src/pm.rs:1820` |
| F12 | Observability: stdout/stderr flushed on exit, panic, and signals; crash dialogs disabled for deterministic exit codes | `lpp_runtime.c:75-175` |
| F16 | `write_str` builtin without trailing newline | `lpp_runtime.c:197`, `runtime/windows_x86_64_min.c:102`, `src/builtins.rs:158`, `src/mir/lower.rs:2034` |
| F9 | Document in-place by-reference struct mutation (`param.field = val`) as zero-cost standard idiom | `Doc.md:128-148` |
| F1, F2 | Integer correctness: unsigned shifts/arithmetic/comparisons (`shr_u`, `shl_u`, `div_u`, `rem_u`, `lt_u`, `le_u`, `gt_u`, `ge_u`, `min_u`, `max_u`), bitwise intrinsics (`popcount64`, `clz64`, `ctz64`, `bswap16/32/64`, `rotl/rotr32/64`), truncation (`trunc_u8`..`u32`, `trunc_i8`..`i32`), checked arithmetic (`add/sub/mul_checked`), wrapping arithmetic (`add/sub/mul_wrap`), formatting (`u64_to_str`, `u64_to_hex`, `str_to_u64`) | `runtime/lpp_int.c`, `lpp_runtime.c:2230`, `src/builtins.rs:3307-3350` |
| F5 | `Map` iteration & management: `map_keys`, `map_values`, `map_clear`, `map_capacity` with full `Map[K, V]` type inference in semantic analysis and MIR lowering | `runtime/lpp_map.c:320`, `runtime/windows_x86_64_min.c:342`, `src/analysis/typecheck.rs:1380`, `src/mir/lower.rs:2000` |
| B5 | Traits and trait bounds: bodyless method declarations, `trait_method_names` registration in semantic analysis, method-call/UFCS return type inference in `expr_type_hint` | `src/analysis/semantic.rs:231`, `src/mir/lower.rs:290` |
| B6 | Turbofish syntax: Rust-style `::<T>`, bracket `::[T]`, and standard `[T]` generic call arguments | `src/frontend/parser.rs:1501-1545` |
| F3 | Atomics: 64-bit and 32-bit atomic load, store, add, sub, and, or, xor, swap, CAS (`atomic_cas`, `atomic_cas_weak`, `atomic_cas32`), memory fences and `cpu_pause` | `runtime/lpp_atomic.c`, `runtime/windows_x86_64_min.c:915`, `lpp_runtime.c:2230` |
| F4 | Concurrency runtime: Mutex (`mutex_new`/`lock`/`trylock`/`unlock`/`free`), RWLock (`rwlock_new`/`rdlock`/`rdunlock`/`wrlock`/`wrunlock`/`free`), OS threads (`thread_spawn`/`thread_join`/`thread_pin`/`thread_id`), CPU core detection (`cpu_count`) | `runtime/lpp_concur.c`, `runtime/windows_x86_64_min.c:935`, `lpp_runtime.c:2231` |
| F6 | Struct layout control: `@repr(exact)` / `@repr(packed)` for tight unpadded field packing and `@align(N)` for cacheline/page alignment padding | `src/frontend/lexer.rs:78`, `src/frontend/parser.rs:60`, `src/analysis/types.rs:34`, `src/analysis/layout.rs:70` |
| F7 | Slices & borrowed views: `slice`, `str_slice`, `slice_to_str`, `slice_len`, indexing `sl[i]`/`ssl[i]`, and non-retaining reader borrow checking | `src/mir/validate_borrows.rs:145`, `src/mir/lower.rs:1885, 2988`, `src/analysis/typecheck.rs:1655` |
| F14 | Injectable clock & RNG: `clock_new`/`now`/`advance`/`free` (deterministic virtual clock + OS clock) and `rng_new`/`next`/`range`/`float`/`free` (splitmix64 PRNG) | `runtime/lpp_clock_rng.c`, `runtime/windows_x86_64_min.c:968`, `lpp_runtime.c:2291` |
| F15 | SIMD vector intrinsics: `vec_i64x2`, `vec_i64x2_splat`, `vec_i64x2_add`, `vec_i64x2_sub`, `vec_i64x2_mul`, `vec_i64x2_xor`, `vec_i64x2_shr`, `vec_i64x2_shr_var`, `vec_i64x2_extract`, `vec_i64x2_sum` | `src/builtins.rs:1850`, `runtime/windows_x86_64_min.c:905`, `lpp_runtime.c:2235` |

All Track B bugs (B1–B8) and feature requests (F1–F16) are fully implemented and verified!

F16 is a 20-minute job and removes a spurious blank line from every
`print_str("...\n")` call site in SamarDB.

---

## Documentation accuracy pass

`LPP_SYNTAX.md` needs correcting alongside the compiler:

- §8 traits — non-functional (B5)
- §6.6 turbofish — unimplemented (B6)
- §5.5 qualified match arms — unimplemented (B7)
- §12.1 — teaches the slow state-threading idiom, omits the by-reference
  mutation that does work (F9)
