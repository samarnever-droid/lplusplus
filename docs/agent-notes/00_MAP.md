# L++ rewrite — working map (agent notes)

> My own orientation notes for the `rewrite/phase-1-workspace-contracts` branch.
> The repo is dense (1322 tracked files); this is the subset that matters for the
> **E5003 builtin-codegen** work described in `docs/rewrite/CORRECTNESS.md`.

## The two trees
- **`src/`** — the *legacy v1* single-crate compiler. Retired as a correctness
  reference (see CORRECTNESS.md). `ARCHITECTURE.md` still describes this tree.
  The root package `lpp` (`src/main.rs`) is still the **driver binary** — it
  dispatches to the rewrite engine when `LPP_ENGINE=rewrite` is set.
- **`crates/`** — the *rewrite* workspace. This is where all real work happens.

## Rewrite crate responsibilities (`crates/`)
| Crate | What it does |
| --- | --- |
| `lpp-common` | shared error/diagnostic primitives |
| `lpp-frontend` | lexer + parser → AST |
| `lpp-hir` | HIR (arena-based) |
| `lpp-types` | `TypeInterner`, `BuiltinId` + **builtin name→descriptor resolution** (`src/builtins.rs`) |
| `lpp-runtime-abi` | the **ABI registry**: `abi/builtins.toml` (source of truth) → generated `abi/generated/builtins.rs` (`BUILTINS: &[BuiltinAbi]`). `BuiltinId(n)` indexes this array. |
| `lpp-mir` | MIR + interpreter (`interpret/eval.rs`, the ARC oracle) |
| `lpp-ownership` / `lpp-passes` | ARC/ownership plan, MIR passes |
| `lpp-codegen-api` | backend trait, `CodegenError`/`CodegenErrorKind` (E5001/E5003/…), `BuiltinLowering` |
| `lpp-codegen-cranelift` | **default backend**. `src/lower.rs` is where builtins get lowered; `FAMILY_A/C`, `import_signature`, and the pre-scan census live here. |
| `lpp-codegen-llvm` / `-wasm` | other backends (pure-Rust emitters; no system LLVM needed) |
| `lpp-codegen-gates` | cross-backend gate tests |
| `lpp-linker` | zero-dependency PE/ELF/Mach-O linker |
| `lpp-runtime` | **the Rust runtime** (`liblpp_runtime.{so,a}`). All `lpp_*` runtime symbols live here now (NOT `lpp_runtime.c`, which is the v1 behavioural reference only). |
| `lpp-driver` | orchestrates compile→object→link→run. `compile.rs` links the generated object against the **Rust runtime cdylib** `liblpp_runtime.so` in `target/debug`. |
| `keel` | package-manager / CLI surface + its gates |

## The correctness oracle
- `scripts/corpus_correctness.sh run` compiles+links+runs every `tests/**.lpp`
  + `examples/**.lpp` (195 files) through `target/debug/lpp` with
  `LPP_ENGINE=rewrite`, judged on exit 0 + internal assertion markers.
- Baseline @ this branch tip: **total=195 PASS=109 (55.9%)**, E5003=14.
- Build prerequisites for the harness:
  `cargo build --bin lpp && cargo build -p lpp-runtime`.

## How a builtin becomes representable (the 3-part port)
A builtin call hits `E5003 UnrepresentableBuiltin` unless all three exist:
1. **Lowering family** — its name is in `FAMILY_A` (plain runtime-symbol call),
   `FAMILY_C` (slice), or handled as `vec_*`/`print`/`list_*` in
   `crates/lpp-codegen-cranelift/src/lower.rs`.
2. **`import_signature` arm** — `"lpp_<name>" => (&[params], ret)` in the same
   file (the import set is a *closed* match; a missing arm is `unreachable!`).
3. **Runtime symbol** — a `#[unsafe(no_mangle)] pub unsafe extern "C" fn lpp_<name>`
   in `crates/lpp-runtime/src/*.rs`, exported by `liblpp_runtime.so`.

The ABI *descriptor* (name/symbol/params/result/semantic_result) already exists
in the generated table for every registry builtin, so porting usually needs no
change to `abi/`.

### Gate coupling (important)
`crates/lpp-codegen-cranelift/tests/phase5c2_gate.rs` keeps its **own** copy of
`FAMILY_A_NAMES` and asserts a registry census (`family A ids == 110`, total
518). That copy is intentionally **decoupled** from `lower.rs::FAMILY_A` — prior
slices (char_at/input/…) added names to `lower.rs` only and left the gate at
110. So a pure lowering-family addition needs **no gate edit**; the census still
partitions the 518-entry registry the same way.
