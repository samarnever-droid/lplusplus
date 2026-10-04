# Rewrite ↔ v1 language-feature parity audit (cutover stage 1)

Date: 2026-09-27 · Engine commit: `ee27039` (`LPP_ENGINE=rewrite`)

## Method

Now that the rewrite engine is selectable (`LPP_ENGINE=rewrite lpp …`), parity is
measured by running **both engines over the same corpus** and comparing. The
operation is `check` (compile source → object: frontend → HIR → typed HIR → MIR →
Cranelift codegen, no link/run). Comparing v1 against the rewrite on the *same*
file controls for negative tests and non-entry modules: a file is a **parity gap**
only when **v1 succeeds and the rewrite fails**.

```
v1: lpp check <file>            (LegacyEngine)
rw: LPP_ENGINE=rewrite lpp check <file>   (RewriteEngine)
```

## Result — `tests/` corpus (181 `.lpp` files)

| Outcome | Count | Share |
|---|---|---|
| **Parity** (v1 OK, rewrite OK) | 86 | 47.5% |
| **Gap** (v1 OK, rewrite FAIL) | 82 | 45.3% |
| Both fail (negative / unsupported by both) | 13 | 7.2% |

`examples/` (14 curated feature demos): only `control_flow_demo.lpp` reaches
parity; the other 12 are gaps (`escape_demo` fails under both).

## Gap categories (priority order)

| # | Category | Count | Failing stage / error |
|---|---|---|---|
| 1 | **type-inference** | 28 | `type stage: ShadowTypeError` — the rewrite's shadow inference rejects programs v1 accepts (Mismatch / ArityMismatch families) |
| 2 | **builtin-codegen** | 23 | `codegen: E5003 UnrepresentableBuiltin` — 17 distinct `BuiltinId`s |
| 3 | **mir-other** | 10 | `MIR build: MirBuildError` ( assorted kinds, incl. `InvalidSpawnTarget` for threads ) |
| 4 | **mir-generics** | 10 | `MIR build: … GenericTypeMaterialization` — generic monomorphization |
| 5 | **codegen-unsupported** | 8 | `E5001 Unsupported` (6), `E5005 ObjectEmit` (1), `E5004 IrVerify` (1) |
| 6 | **module-resolution** | 2 | `dependency graph: MissingModule` — cross-package/stdlib imports |

## Why builtins fail (E5003 mechanism)

`BuiltinLowering::from_builtin(id)` (lpp-codegen-api/src/builtin.rs) returns
`Some` only when the ABI registry `descriptor.symbol` is **non-empty** — i.e. the
builtin maps to a single runtime import. v1 builtins with an **empty symbol**
(e.g. `print`, lowered as a *sequence* of runtime calls) are not runtime-lowerable
as one import; a backend must then handle them **by name** within its own
documented subset (cranelift's `lower.rs` name list: `print`, `print_str`,
`print_int`, `print_float`, `print_bool`, `str_concat`, `str_len`, …) or reject
with `E5003`. The gap is therefore two-fold:

1. **Symbol-lowerable builtins** missing from the generated descriptor coverage.
2. **Sequence/by-name builtins** not in each backend's by-name subset.

The 17 distinct failing `BuiltinId`s (0-based positional index into
`abi/builtins.toml`'s `[[builtin]]` list — base confirmed against the cranelift
constants `PRINT_STR=from_raw(14)` and `LIST_NEW=from_raw(46)`):

```
20 22 26 45 93 145 147 171 175 181 276 334 366 449 489 497 507
```

> Caveat: `builtins.toml` carries duplicate-name entries (v1 overloads
> `input`/`lpp_input`, `file_size`/`lpp_fd_size`, …), so a positional name lookup
> can misalign on those. Authoritative names must be read from
> `lpp_runtime_abi::generated`'s descriptor table (`BuiltinId::descriptor().name`)
> when each builtin is fixed — not from the positional guess.

## Corrected completion picture

- **Infrastructure** (pipeline stages, three backends' cores, runtime, driver
  contract, PM/keel): high — the whole source→exe path works for programs within
  the supported subset.
- **Language-feature parity** (can the rewrite compile v1's real programs?):
  **~47%** on the `tests/` corpus. **This is the gating metric** for promoting the
  rewrite from opt-in (`LPP_ENGINE=rewrite`) to the default engine.

## Prioritized remaining work (data-driven)

1. **Type-inference parity (28 gaps)** — largest single lever. Drill the
   `ShadowTypeError` kinds; likely a small set of inference rules where the
   rewrite is stricter than v1 (default params, numeric coercion, arity).
2. **Builtin-codegen parity (23 gaps / 17 builtins)** — expand descriptor
   coverage + each backend's by-name subset (I/O, net, string, list, process,
   env, math, time, atomic, rng families).
3. **MIR generics (10) + mir-other (10)** — generic type materialization and
   spawn/thread targets.
4. **LLVM backend parity (~40%)** — managed data, closures, slices, SIMD, enums
   (overlaps the above for the LLVM target).
5. **codegen-unsupported (8) + module-resolution (2)** — narrower constructs and
   cross-package import resolution.
6. **WASM SIMD slice 3, runtime staticlib packaging, Phase 6 arenas** — as before.

Re-run this audit after each slice; the parity share (currently 47.5%) is the
cutover-to-default progress bar.
