# Rewrite correctness baseline (the new progress bar)

**v1/LegacyEngine is retired** — too buggy, never production-ready, and no
longer a reference. So "parity with v1" is dead as a measure (`docs/
parity-audit-stage1.md`, `scripts/materialize_v1_oracle.py`, and
`compatibility/v1.2.0/` are obsolete and will be retired). The rewrite is now
driven purely for **correctness and feature-completeness**.

## Current score (expected-outcome harness)

```
total=196  scored=178  skipped=18
  run:    PASS=165  COMPFAIL=0  RUNFAIL=0  ASSERTFAIL=0  TIMEOUT=0
  reject: XFAIL-OK=13  WRONGACCEPT=0
correct: 178/178 = 100.0%   (pass 165 + correctly-rejected 13)
```

Literal 196/196 is the wrong target: 18 files are not compile-fixable in this
headless native harness (GUI/webview needing a window server, the wasm-target
rejection suite, and library modules with no `main`) and are **skipped**
(excluded from the denominator); negative tests MUST be rejected (a clean
rejection = correct). The honest metric is **correct / scored**.

The previous 6 `WRONGACCEPT` are closed: the type stage now rejects
blocking builtins in async bodies, task handles captured into closures, and
borrowed slice views that are returned, captured, spawned, or used after their
source is reassigned. The previous 5 normal-program RUNFAILs are also closed:
calculator callback typing/codegen, turbofish/generic methods, overlapping
generic impl dispatch, the enum-match fixture, and the FFI smoke path now all
compile and run under the rewrite engine.

## The oracle: `scripts/corpus_correctness.sh`

Runs every corpus program (`tests/**.lpp` + `examples/**.lpp`, 196 files)
through the **rewrite** engine (`LPP_ENGINE=rewrite lpp run <f>`) and judges
each on its **expected outcome** via `classify()` → run / reject / skip:

* **run** — must compile, link, execute, exit 0 (honouring assertion markers).
* **reject** (path `*reject*`/`*_rejected*`/`*_bad_*` or a `# corpus-expect:`
  directive) — the compiler MUST refuse it; a nonzero rc = `XFAIL-OK`, a clean
  compile = `WRONGACCEPT` (a bug).
* **skip** (no `def main` → library module; `import gui`/`gui_`/`webview_`/`sdl`
  → display program; `tests/wasm/reject/*` → wasm-target-only restrictions) —
  excluded from the denominator.

No v1 involved.

```
scripts/corpus_correctness.sh run     # full: compile+link+execute
scripts/corpus_correctness.sh check   # fast triage: type-check only
```

> **Update (2026-10-04):** native runtime/lowering coverage now includes the
> network/HTTP, time/RNG/clock, atomic/concurrency, mutex/rwlock/thread, narrow
> integer layout, and prefetch slices needed by the stress corpus. Standalone
> rewrite compiles inject the bundled `stdlib` as a declared dependency package.
> The async/slice negative tests now reject for their intended type-stage safety
> reasons. Generic impl methods are now monomorphized through the instance
> planner, overlapping method candidates resolve most-specific first, callback
> parameters carry closure provenance into codegen, and the remaining enum/FFI
> corpus smoke cases are green. Full check/run harness score: `178/178 = 100.0%`
> with `COMPFAIL=0`, `RUNFAIL=0`, and `WRONGACCEPT=0`.

## Baseline @ HEAD (this commit)

```
total=195   PASS=157   clean-correct = 78.5%   (COMPFAIL=13 RUNFAIL=29)

Recent wins: trait-typed parameters desugar to trait-bounded generics, so
dynamic-dispatch-by-specialization works (test_dyn_dispatch; see
docs/rewrite/METHOD_DISPATCH.md); String+scalar `+` concatenation across
codegen/verifier/interpreter; unsigned-compare semantic_result fix (test_integer_correctness);
glob-import semantics — `import module` now brings all of the module's
definitions into unqualified scope (test_import, test_dotted, test_full_project);
slice support — `slice()` yields `Slice[T]`, `view[i]` indexing on list- and
string-slices, `len`/`slice_get` overload onto the slice runtime, and
`slice_get` on a StrSlice returns a 1-char Str (wasm/cases/slices,
str_slice_zero_copy, test_slices_stress). All done without extending the frozen
v1 ABI (schema test stays at 518 builtins / 354 symbols).
```

> **Update (UFCS method dispatch):** implemented method-call syntax
> `receiver.method(args)` for free-function UFCS and concrete `impl` methods.
> This greened `test_methods` and `test_traits` (corpus 140→144). Still open in
> this cluster: `test_traits_stress` (trait-bound dispatch on a generic type
> parameter `[T: Sizer]`), `generic_trait_impls` (overlapping generic impls +
> monomorphized rename), and `test_dyn_dispatch` (trait-typed parameters /
> trait objects) — all need the full trait/instance selection path. See
> `docs/rewrite/METHOD_DISPATCH.md`.

Failures, by the **stage** that rejects them (accurate; the script's raw
COMPFAIL/RUNFAIL split under-counts because non-E-coded stage errors land in
RUNFAIL):

| Stage | Count | What's missing |
| --- | --- | --- |
| codegen **E5003** UnrepresentableBuiltin | 7 | a builtin needs (a) membership in a lowering family (`FAMILY_A`/`vec_*`/slice), (b) a runtime symbol, (c) an `import_signature` entry. Slices so far: alias+`len` (98→104); math+string `int_pow`/`sin`/`cos`/`str_repeat`/`str_split` (104→106); IO `input`+`parse_int` (106→109); **IO file-ops this commit** (109→114) — ported the deterministic filesystem + subprocess family to the Rust runtime: `read_file`/`write_file`/`append_file`/`delete_file`/`file_exists`/`file_size`/`file_copy`/`file_move`/`dir_create`/`dir_remove`/`path_exists`/`command_output`, each added to `FAMILY_A` + `import_signature` (cranelift). This greened `io_demo`, `test_new_builtins`, `filesystem_v2`; it also flipped two **negative** tests to PASS under the native harness — `wasm/reject/reject_file` (a wasm-WASI rejection: `read_file` legitimately works natively, and `--target wasm32` still rejects it) and `async_blocking_rejected` (its intended blocking-in-async rejection is a separate unimplemented stage; it had only ever "rejected" via the missing builtin). Both are the harness's known reason-blind judging (plan item 6), not a native-correctness regression. **LIST slice this commit** (114→116) — ported the `List[T]` mutation/order/search family to the Rust runtime: `list_insert`/`list_remove`/`list_swap`/`list_reverse`/`list_truncate`/`list_reserve`/`list_capacity`/`list_clear`/`list_sort`/`list_sort_desc`/`list_sort_u`/`list_index_of`/`list_binary_search`/`list_extend`, each added to `FAMILY_A` + `import_signature` (cranelift). This greened `test_list_operations_stress` (8/8) and `test_list_sorting_stress` (6/6). Uncovered and **fixed a separate pre-existing codegen bug**: the string `+` operator was lowered as integer `iadd` of the two heap pointers (`lower_binary`), silently corrupting the result; it now routes through `lpp_str_concat` (borrows both operands, returns a fresh owned ARC string), mirroring the existing string-`==` path — see the pre-scan import + `Rvalue::Binary` call-site in `lower.rs`. Without that fix the list files exited 0 with no output (a hollow pass); with it they emit real assertions. The remaining 7 lack a runtime symbol: net (`net_*`), rng (`rng_new`), atomics/concurrency (`atomic_new`), plus `command_exec`/`env_set` (their negative tests). Cranelift done first; WASM + LLVM still to mirror. |
| **MIR build** MirBuildError | 27 | `Unsupported(Field)` (the `arc_*` field-alias cases), `MissingReturn`, `InvalidSpawnTarget`, `GenericTypeMaterialization`, `Unresolved`, `MissingAggregateFact` |
| **type stage** ShadowTypeError | 23 | `Mismatch`, `ArityMismatch`, `SpawnCaptureMutation` — inference/checking gaps |
| codegen **E5001** UnsupportedConstruct | 8 | `tuple` lowering (incl. the 2 former panics, now clean E5001); `vector` list-element (now a clean reject); "non-bool branch condition" |
| dependency graph MissingModule | 7 | GUI/network examples importing modules not on the path |
| link (collect2/ld) | 4 | unresolved symbols at link |
| codegen E5004/E5005 | 2 | generics `DuplicateDefinition` / object emission |
| frontend E1110/E1111 | 2 | (both are negative tests — see below) |

**Negative tests (13)** — `reject_*`, `*_rejected`, `*_bad_*` are *meant* to be
rejected, so a non-zero exit is correct **iff the rejection reason is right**.
Several currently reject via E5003 (builtin) rather than their intended reason
(e.g. `wasm/reject/reject_command` should reject command use, not a missing
builtin) — these need reason-aware judging.

**Compiler panics: 0 (DONE)** — at this HEAD there were 2: `list_vector_rejected`
(`unreachable!` in `element_class` on a vector list-element) and
`tuple_struct_managed` (`unreachable!` in aggregate `type_size_align` on a tuple
field). Both now emit clean typed `E5001`: a tuple is laid out as a flat record
of its elements, and an unstoreable list element is rejected in the pre-scan.
(`str_slice_zero_copy` was already cleared by the zero-init parity work.) A crash
is the worst failure mode, so this was priority 1.

## Work plan (priority order)

1. ~~**Compiler panics**~~ — **DONE (0)**: tuple aggregate layout + vector
   list-element rejection now emit clean `E5001` instead of `unreachable!`.
2. **E5003 builtin-codegen (7 left)** — port each missing builtin: implement
   its runtime symbol + register it in a lowering family. Done so far: string
   (`char_at`/`chr`/`ord`/`str_substr`; 95→98), alias+`len` (98→104),
   math+string (`int_pow`/`sin`/`cos`/`str_repeat`/`str_split`; 104→106), IO
   `input`+`parse_int` (106→109), IO file-ops (`read_file`/`write_file`/
   `append_file`/`delete_file`/`file_exists`/`file_size`/`file_copy`/`file_move`/
   `dir_create`/`dir_remove`/`path_exists`/`command_output`; 109→114). Next: list
   (`list_insert`/`list_sort`/`list_remove`/`list_sort_desc`/`reverse`/`swap`/
   `truncate`/`binary_search`/`index_of`/`extend`/`sort_u` + the other
   stress-file ops), then net/system. Cranelift first, then WASM + LLVM.
3. **MIR build (27)** — `Unsupported(Field)`/arc field-alias, `MissingReturn`,
   generic materialization, spawn targets.
4. **Type stage (23)** — `Mismatch`/`ArityMismatch` inference + spawn-capture.
5. **E5001 tuples (6)**, dependency-graph (7), link (4), generics (2).
6. **Refine the harness** — stage-aware buckets + negative-test (reason-aware)
   judging, so the auto-baseline matches the accurate table above.

Each closed gap is a real language feature that works, measured by the harness —
not a match against a dead engine.
