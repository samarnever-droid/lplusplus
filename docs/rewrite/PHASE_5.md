# Rewrite Phase 5 — backends

## Rewrite roadmap (Phases 0–7)

1. **Phase 0 — secure and freeze** — complete.
2. **Phase 1 — workspace and common contracts** — complete.
3. **Phase 2 — frontend and modules** — complete.
4. **Phase 3 — HIR, types, generics, and traits** — complete.
5. **Phase 4 — MIR and ownership** — complete: typed CFG, verification,
   pass management, ownership plan, ARC/move-out, and the optimizer with
   its safety exit (4A–4F).
6. **Phase 5 — backends** — active: `lpp-codegen-api` plus the
   Cranelift (5A–5C2), WASM (5D + 5D2a + 5D2b slices 1–2), and LLVM
   (5E slice 1 + 5E2a float output) backends that translate optimized, revalidated MIR
   into verified machine objects; 5F safety exit done. Next: 5D2b
   slice 3 (async/`Await`/`Spawn`/tasks).
7. **Phase 6 — linker and runtime** — active: 6A (`lpp-linker` crate,
   the proven v1 direct-linker engine relocated into `crates/` with a
   typed facade and hermetic gates) is done; 6A.2 (typed error
   boundary) and 6B (runtime crate) are planned.
8. **Phase 7 — CLI, PM, LSP, and cutover** — planned.

The root compiler remains routed through `LegacyEngine`. Rewrite codegen
is shadow-only: it emits objects that are differentially tested against
the v1 engine and the Phase 4 execution oracle; production does not
route through rewrite codegen until the Phase 7 cutover.

## Approved Phase 5 architecture

The stage boundary is:

```text
optimized and revalidated MIR (Phase 4 exit)
  -> lpp-codegen-api contract (targets, machine types, builtin lowering)
  -> backend-specific lowering (Cranelift / LLVM / WASM)
  -> verified, deterministic object files
  -> Phase 6 linker contract
```

Dependencies remain one-way: `lpp-codegen-* -> lpp-mir / lpp-passes /
lpp-ownership / lpp-runtime-abi / lpp-types`. MIR, passes, and ownership
must never import a backend. The v1 `src/backend/` tree (13,929 LOC:
wasm 9,606 / cranelift 2,611 / llvm 1,712) is the behavioral reference,
not a source to copy into the rewrite crates.

**Non-negotiable backend rules** (from the Phase 4 contracts and
`OPTIMIZATION_STRATEGY.md`):

- **No ownership-blind codegen.** Heap layouts and ARC traffic come from
  the 4D ownership plan and the 4E consume-vs-borrow table; a backend
  never invents a layout.
- **No builtin dispatch by string matching.** Every runtime call lowers
  from the checked-in generated table (`lpp-runtime-abi::generated`,
  `BuiltinId::descriptor`); the symbol, parameter and result ABI types
  come from the registry.
- **Deterministic output.** Functions lower in `MirFunctionId` order;
  data symbols in declaration order; object bytes are
  compile-to-compile identical for the same input (asserted by test).
- **Typed rejection.** Anything a slice does not lower is a typed
  `CodegenError` with an `E5xxx` code at the exact function — never a
  silent fallback and never a miscompile.
- **Budget-bounded.** Machine-level refinements honor
  `OptimizationBudget`; a backend may refine but never change language
  semantics.
- **v1 runtime ABI preserved.** Generated objects link against the v1
  freestanding runtime unchanged; symbol names, calling conventions,
  and constant layouts (including the 24-byte string header) match the
  v1 cranelift backend.

## Phase 5 slices

### 5A — codegen contract and ABI lowering — approved contract

New crate `lpp-codegen-api` (dependencies: `lpp-mir`, `lpp-types`,
`lpp-runtime-abi`): the backend-neutral contract every Phase 5
backend implements.

**Scope**

- `Target` — the targets rewrite codegen addresses in Phase 5:
  `X86_64` and `Aarch64` for the Cranelift backend (the ISA set
  cranelift 0.113 ships by default). Each target carries its
  deterministic triple and pointer width. `Wasm32Wasi` (Phase 5's first
  32-bit target) is added to the lattice by **5D**; the Cranelift
  backend's `targets()` is unchanged by it.
- `MachineType` — the machine scalar lattice: `I8`, `I32`, `I64`,
  `F64`, `I64X2`.
- The registry has two type views, and the contract keeps them apart
  (implementation evidence from the checked-in generated table):
  - `machine_type_for_abi` on `generated::AbiType` (5 variants: the
    *machine* ABI a builtin symbol actually takes):
    `Bool -> I8`, `I32 -> I32`, `I64 -> I64`, `F64 -> F64`,
    `Void -> None`. Total and compile-checked; this is the only table
    a backend may use to build import signatures.
  - `semantic_machine_type` on the semantic view (9 variants:
    `Any`, `Bool`, `F64`, `I32`, `I64`, `Str`, `StrSlice`,
    `VectorI64x2`, `Void`): `Any/Str/StrSlice -> I64`, `Bool -> I8`,
    `I32 -> I32`, `I64 -> I64`, `F64 -> F64`,
    `VectorI64x2 -> I64X2`, `Void -> None`.
  - The two tables are **not** inverses of each other. v1 evidence:
    all 35 vector sites in the generated table (13 results, 22
    parameters) carry machine `I64`, not `I64X2` — v1 lowers
    `VectorI64x2` as a boxed pointer — and the machine arity exceeds
    the semantic arity where lowering adds detail (`slice`: semantic
    3 -> machine 5; `vec_i64x2`: semantic 2 -> machine 4).
    `I64X2` stays in the lattice as the 5C register-vector choice;
    5B emits no vector code.
- `BuiltinLowering` — for one `BuiltinId`, the symbol plus the machine
  parameter and result types derived from
  `BuiltinId::descriptor()`. A builtin whose registry symbol is empty
  (e.g. `print`, which v1 lowers as a sequence of calls, not a single
  runtime import) is reported **not runtime-lowerable**; backends
  either handle it by name in their own subset or reject it with
  `E5003`.
- `CodegenOptions { target: Target }`.
- `Backend` trait: `name()`, `targets()`, and
  `compile_module(program: &MirProgram, types: &TypeInterner,
  options: &CodegenOptions) -> Result<CompiledModule, CodegenError>`.
- `CompiledModule { target, object: Vec<u8>, exported_symbols:
  BTreeSet<String>, imported_symbols: BTreeSet<String>,
  entry: Option<String> }` — the object bytes plus a deterministic
  symbol census (so tests can assert ABI shape without parsing ELF).
- `CodegenError { function: Option<MirFunctionId>, kind }` with the
  `E5xxx` table:
  - `E5001` `UnsupportedConstruct` — a MIR shape the slice does not
    lower (aggregates, stores, closures, async, …).
  - `E5002` `UnsupportedTarget` — the target is not in
    `targets()`.
  - `E5003` `UnrepresentableBuiltin` — a builtin the slice does not
    lower (or a runtime-lowerable builtin is required but unavailable).
  - `E5004` `IrVerificationFailed` — the backend's own IR rejected the
    lowered function.
  - `E5005` `ObjectEmissionFailed` — the object emitter failed.
  - `E5006` `AbiMismatch` — a registry descriptor contradicts the
    backend's expectation (defence in depth at the boundary).

**Gates**

1. ABI parity: for all 518 builtins in the generated table,
   `BuiltinLowering` matches the *machine* descriptor exactly (symbol,
   arity, machine types); the semantic mapping is total; the census of
   semantic-vs-machine divergences (boxed vectors, arity expansion) is
   asserted with exact counts so table drift fails loudly; builtins
   with an empty symbol are exactly the non-runtime-lowerable set.
2. Determinism contract: the census uses ordered collections and the
   module document requires byte-identical objects across repeated
   compiles (enforced by 5B's tests).
3. Compile-fail: every `E5xxx` code is reachable and asserted with
   its exact code and (where applicable) exact function.
4. No new dependencies outside the lockfile; strict warnings;
   production unchanged.

### 5B — Cranelift: scalars, CFG, and calls — approved contract

New crate `lpp-codegen-cranelift` (dependencies: `lpp-codegen-api`,
`lpp-mir`, `lpp-types`, plus `cranelift-codegen`,
`cranelift-frontend`, `cranelift-module`, `cranelift-object` at
0.113 — the exact versions v1 already pins in `Cargo.lock`).
`CraneliftBackend` implements `Backend` for `X86_64` and `Aarch64`
and emits relocatable ELF objects via cranelift-object.

**Lowering subset (the 5B surface)**

- **Types.** `Int -> I64`, `Float -> F64`, `Bool -> I8`, `Str -> I64`
  (matching the v1 cranelift backend exactly), `Char -> I32` (the
  rewrite defines this mapping; v1 predates `Char`). `Void` -> no
  return value.
- **Locals.** Every MIR local is a cranelift variable
  (`declare_var`); the 4B definite-initialization invariant is what
  makes this sound — a variable is only read where a def dominates.
- **Scalars.** The full 4F operator set: wrapping integer arithmetic
  (`iadd`/`isub`/`imul`/`sdiv`/`srem`), bitwise and shifts, signed
  comparisons; IEEE-754 float arithmetic including `frem` and `fcmp`;
  bool logical and equality; integer `Negate`/`Not`. The folds
  established in 4F already removed the constant cases; the lowering
  matches the interpreter's runtime semantics for the rest.
  Integer division by zero is not software-trapped — identical to the
  v1 cranelift backend, which emits the hardware divide.
- **String constants.** Emitted as the v1 runtime layout: the
  24-byte header (`LPP_ARC_MAGIC 0x4152_4331` in the first two
  little-endian words, sixteen zero bytes), the UTF-8 payload, a null
  terminator, 16-byte alignment, handed out as `base + 24`. This is
  byte-for-byte the layout the v1 cranelift backend emits, so the
  same object links against either v1 runtime (host or freestanding).
  Data is **deduplicated per `MirStringId`** (one symbol per literal):
  the interpreter interns string constants per program, so pointer
  identity of equal literals must match between object and oracle.
- **Builtin subset.** `print_str` (`lpp_print_str`, `I64 -> Void`) is
  the only builtin call in 5B. Any other `Rvalue::Builtin` is
  `E5003` at the exact instruction's function. Float `Modulo` lowers
  through a `fmod` import (`F64, F64 -> F64`) — cranelift 0.113 has
  no `frem` opcode and the v1 backend uses the same libcall
  convention; the differential harness links `libm` (the v1
  freestanding runtime provides the symbol itself).
- **Calls and returns.** Direct calls to user functions with
  parameter passing per the callee's declared parameter types
  (`iconst`/`fconst`/`bconst` for constant arguments, variable loads
  for copies); `Return` per the return type; `Unreachable` to
  cranelift's `unreachable`.
- **Scalar slot mutation.** Implementation evidence: every L++
  `=` statement lowers to `InstructionKind::Store`, and augmented
  assignment (`i = i + 1`) desugars through `Rvalue::Load` — so the
  5B slice includes **bare-local** places only: `Load`/`Store` with an
  empty projection list against a local of a slice value type lower to
  a cranelift variable read/write. Projected places (fields, indices,
  downcasts) remain `E5001` (5C).
- **CFG.** `Goto -> br`, `Branch -> brif` (the I8 condition is
  `bcast` to the cranelift bool type), `Return`, `Unreachable`.
- **Entry.** A source-level `main` lowers to the internal `lpp_main`
  export, plus the generated C-ABI `main(void) -> int` wrapper that
  calls it and returns 0 — identical to the v1 convention, so the
  system linker finds the entry.
- **Module.** cranelift-object with the target triple from
  `CodegenOptions`, object name `lpp_module` (v1-compatible), default
  libcall names, cranelift settings with `opt_level = "speed"` (the
  v1 AOT default).

**Not in 5B (typed rejection)** — aggregates, tuples, structs,
enums, lists and maps, stores and loads on *projected* places
(fields, indices, downcasts), closures, async/task,
`SwitchEnum`: `E5001` at the exact function. (Bare-local scalar
stores/loads are in 5B — see "Scalar slot mutation".) These are 5C
(Cranelift aggregate/ARC surface) and later.

**Gates**

1. **Differential execution.** For the 5B scalar corpus (wrapping
   overflow, div/rem, shifts, signed comparisons, IEEE NaN/inf,
   branches, loops, multi-function calls, char, `print_str`): the
   rewrite object is linked with the C runtime symbols it imports,
   executed, and its stdout plus exit status compared against the
   Phase 4 execution oracle (`execute_mir_with_stats`), which is the
   v1-proven reference.
2. **Determinism.** Two compiles of the same program produce
   byte-identical objects and identical symbol censuses; function
   order is `MirFunctionId` order.
3. **IR validity.** Every corpus function survives cranelift's own
   verification (object emission verifies on `define_function`); a
   broken lowering fails with `E5004`/`E5005`, never silently.
4. **Compile-fail.** A list program, a struct program, a closure
   program, and an async program each fail with the exact `E5001`/
   `E5003` code at the exact function.
5. **Regression.** Every existing workspace test stays green; strict
   all-target warnings; production stays on `LegacyEngine` with
   rewrite codegen shadow-only.

### 5C — Cranelift aggregate data surface — approved contract

Scope narrowing (implementation evidence): the original 5C bullet
bundled six concerns. The first three — heap layouts, ARC runtime
calls, `SwitchEnum` — form the **aggregate data surface** and are this
slice. Closure cells, tasks/await/`Spawn`, and the full 518-builtin
table form the **function-value surface** and move to **5C2** (below).
Rationale: each half is independently gateable against the Phase 4
oracle, and the data surface is a prerequisite for none of the
function-value surface's machinery beyond what 5C already adds.

Extended surface over 5B in `lpp-codegen-cranelift`:

- **Value representation (v1 runtime ABI, unchanged from the v1
  cranelift backend).** Every managed value is an `I64` heap pointer
  with a 24-byte ARC header in front of its payload:
  `magic u32 @0, refcount i32 @4, generation i32 @8, map_size u32
  @12, destructor ptr @16` (the freestanding `LppArcHeader`).
  `Str`, `List[_]`, structs and enums all lower to this pointer ABI,
  so `Call`/`Return` need no new machinery over 5B.
- **Struct layout.** Fields in declaration order with native
  alignment from offset 0 — the v1 `struct_layout` rule (Int/Float/
  pointer `8:8`, Char `4:4`, Bool `1:1`). Allocation is
  `lpp_arc_alloc_with_destructor(size, dtor)` (zero-initialized
  payload, refcount 1).
- **Enum layout (rewrite-defined; v1 predates enums).** An `I64`
  variant-ordinal tag at offset 0, then the active variant's fields
  laid out from offset 8 with native alignment; allocation size is
  the maximum over all variants, 8-aligned. `ConstructVariant`
  writes the tag; `SwitchEnum` reads it.
- **List layout (runtime-managed).** The v1 `LppList` node
  (40 bytes) via `lpp_list_new` (scalar elements) or
  `lpp_list_new_arc` (elements of a managed type). Elements are raw
  slots: `lpp_list_push{,_arc,_float,_bool}`,
  `lpp_list_get{,_arc,_float,_bool}`, `lpp_list_set{,_arc,_float,
  _bool}`, `lpp_list_len`. All are registry entries
  (`BuiltinId::descriptor`), so the symbols come from the table, not
  from string matching.
- **ARC traffic (mirrors the interpreter's consume-vs-borrow rules,
  4E; the 4D plan classifies what is shared).** Retain points:
  `Rvalue::Use(Copy)` of a managed local (new owner), `Rvalue::Load`
  of a managed place (field/element reads gain a reference), and a
  `Store` from a `Copy` operand into a bare local or a struct field.
  ARC-list element stores retain inside `lpp_list_set_arc`
  (runtime-internal) instead. Release points: cell death on
  reassignment of a bare managed local, replacement of a managed
  struct field, and — at every `Return` — a release pass over all
  managed locals of the function in declaration order. Destructors:
  one generated, exported function per constructed nominal aggregate
  (`lpp_drop_s{raw}` / `lpp_drop_e{raw}`, in `MirAggregateId`
  order), releasing managed fields in declaration order (enums
  dispatch on the tag); the v1 runtime invokes it when a node's
  refcount reaches zero, so nested structures free depth-first.
- **Null-on-move (implementation evidence).** A moved-out slot — a
  call argument, a returned value, a `ConstructStruct`/
  `ConstructVariant`/`List` element that was a `Copy` of a managed
  local — is written with null at the move site. `lpp_arc_release`
  of null is a no-op (v1 runtime behavior), which makes every
  release point unconditional and statically decidable even though
  the interpreter models the same moves dynamically (`frame.take`).
  Read-after-move is impossible: the 4B verifier rejects it.
- **String constants.** 5B's immortal two-word-magic header is
  simultaneously a valid host-runtime magic and the freestanding
  immortal sentinel, so retain/release on literal data symbols are
  no-ops under both v1 runtimes; no ARC code is emitted for them.
- **`SwitchEnum`.** The subject is a plain read (no retain — the
  interpreter's rule); the tag is the `I64` at offset 0; lowering is
  a dense `brif` cascade over the variant ordinals to the
  terminator's targets (targets may alias for wildcard arms;
  out-of-range tags cannot occur in verified MIR).
- **`Downcast` in a `Load` projection** lowers to a no-op: the
  dispatch that justifies it is the dominating `SwitchEnum` (the 4B
  builder emits downcast projections only in match arms). A `Store`
  through a `Downcast` or `TupleField` is `E5001` (the interpreter
  rejects it as `InvalidAggregateProjection`).
- **Builtin subset.** `print_str` (5B) plus `list_new` (both
  element modes, chosen by the destination's element type). Every
  other `Rvalue::Builtin` is `E5003` at the exact instruction's
  function (5C2 lifts this to the full table).
- **`Rvalue::ListLen`** lowers to `lpp_list_len`.
- **`Rvalue::Tuple` / `TupleField`** remain `E5001`: the rewrite
  frontend emits no tuples (checked-in evidence: no tuple expression
  in `lpp-frontend`), and 5C keeps the typed rejection rather than
  inventing a syntax.

**Not in 5C (typed rejection)** — closures and their calls
(`MakeClosure`, closure callees), async functions, `Await`, `Spawn`,
slices, tasks, maps, function values, and every builtin outside the
subset: `E5001`/`E5003` at the exact function. These are 5C2.

**Gates**

1. **Differential execution (aggregate corpus).** Structs (construction,
   field read/write, nesting, reassignment, mixed-width layout,
   struct-in-struct, struct with managed fields), enums (dense match
   dispatch, payload binding, wildcard arms, match on parameter and
   local), lists (literals of every element class, element
   read/write, `ListLen`, reassignment, managed-element lists,
   nested lists, list-in-struct), and cross-function passing of all
   three: object stdout equals the Phase 4 oracle
   (`execute_mir_with_stats`), no `fail_*` marker, `all_ok` present.
2. **Differential execution (ARC stress corpus).** Aliasing,
   self-assignment, field/element swaps, move-then-reassign,
   cross-container stores, deep nesting: object stdout equals the
   **ARC-mode** oracle (`execute_mir_arc` with the 4D
   `pinned_types()`), whose end-of-run balance proof validates the
   interpreter's own ARC model. Two corpus-domain notes from the
   implementation evidence: (a) ARC-mode construction, list
   elements, and call arguments **move** their local sources
   (use-after-move is `E4303`), so the corpus never reads a local
   after transferring it; (b) the 5C slice has no closure
   captures, and the 4D plan graph carries no struct-field edges, so
   no 5C cycle type is statically pinnable (verified:
   `pinned_types() = ∅` for a struct self-cycle; such a program
   fails the oracle's balance proof with `E4304`) — cyclic
   aggregation through function values remains 5C2.
3. **Determinism.** Two compiles of each corpus produce byte-identical
   objects and identical symbol censuses; destructors in
   `MirAggregateId` order; imports declared only when used.
4. **Symbol census.** The object is a real ELF image with entry
   `main`; exports include `lpp_main`, every user function, and every
   generated destructor; imports exactly the runtime symbols the
   corpus uses.
5. **Compile-fail.** A closure program and an async program each fail
   with the exact `E5001` at the exact function; a non-slice builtin
   (`print_int`) fails with the exact `E5003`; all 5B compile-fails
   stay green. Every existing workspace test stays green; strict
   all-target warnings; production stays on `LegacyEngine`.

### 5C2 — Cranelift function-value surface — approved contract

Scope: lifts the 5C function-value rejections — closures
(`MakeClosure`, capsules and by-reference capture cells), function
values (`Operand::Function`), async functions / `Await` / `Spawn` and
task thunks, the slice builtin family, and the full 518-builtin table
with a registry-derived policy (including the SIMD operations 5B
deferred).

**Value representation (v1 runtime ABI, per the v1 cranelift backend
and `runtime/linux_x86_64_min.c`):**

- **Closure capsule.** A 16-byte ARC node `[code @0, env @8]`,
  allocated `lpp_arc_alloc_with_destructor(16, &lpp_closure_destroy)`;
  `lpp_closure_destroy` releases the env reference (env is `NULL` for
  zero-capture closures). `MakeClosure { function, captures }` with
  zero captures still allocates the capsule (the oracle allocates a
  managed closure node), keeping the balance identical.
- **Closure env.** An `N*8`-byte ARC node of capture slots,
  allocated `lpp_arc_alloc_with_destructor(N*8, &lpp_drop_c{n})`;
  `lpp_drop_c{n}` is a generated per-closure-function destructor that
  releases the managed capture slots, numbered in `MirFunctionId`
  order over closure functions.
- **Task env.** `lpp_tuple_alloc(size, managed_mask, packed_offsets)`:
  16-byte prefix (mask + packed 16-bit offsets), value slots from
  offset 16, destroyed by `lpp_tuple_destroy` (the shim's own
  implementation, with no slot-count limit).
- **Task node.** `lpp_task_new(code, environment, managed)`: ARC node
  `{ code, environment, result, state, result_managed }` whose
  destructor releases the environment and, once resolved and managed,
  the result. `lpp_task_poll` runs `code(environment)` exactly once
  (state 0→1→2; polling a running task exits 101).
  `lpp_task_await` polls and retains a managed result — the task keeps
  its share, the awaiter gains one (the oracle's repeated-await
  semantics). `lpp_task_destroy` releases.

**Closure lowering.** A closure function compiles as `lpp_c{n}`
(`n` = the `MirFunctionId`, in function-id order; closures are
unnamed in the MIR) taking the **env pointer** as its first
parameter. The builder emits each capture as a `kind=Capture`
parameter local, so the ABI parameters are the user parameters
only: on entry the capture slots are loaded from the env into the
capture locals as **views — no retain** (the env slot is the
reference owner, the local is a view).

Captures split on whether the closure **writes** the captured
local. A pre-scan of the function body (over every contained
closure, at any nesting depth, before any local is materialized)
marks each enclosing local that a closure assigns to; a local that
is a parameter of any enclosing function is never marked — the call
ABI passes parameters by value, so that capture stays a one-way
value capture. A marked local lowers as a **by-reference capture
cell**: a single-element list of the captured type in the
enclosing frame, and the closure's capture local is that same cell.
A read of the local lowers to a load of element 0 (`lpp_list_get*`,
retaining when the element is managed); a store lowers to a set of
element 0 (`lpp_list_set*`), which releases the old element and
stores the new one in place — there is **no env-slot writeback**,
the env slot keeps holding the cell pointer for the life of the
capsule. The enclosing frame and the closure share the cell, so the
closure's write is visible wherever the local is read. Unmarked
captures stay **value captures**: the env slot holds the value
(retained when managed) and the capture local is a plain view.

A managed return value of a capture view retains for the caller;
the frame-exit release skips the capture views (they hold no
reference — the env slot owns the cell or the value) and never
releases the env (the capsule owns it). `MakeClosure` allocates the
env (the capture slots in the closure's capture-parameter order; a
cell slot and a managed value slot are retained) and the capsule
`[&lpp_c{n}, env]`; a zero-capture closure has a `NULL` env. A
closure call loads `env` from the capsule and calls through the
capsule's code pointer with signature `(env, args...) -> I64`
(every closure of the callee's function type shares that shape),
unboxing the result for the return type.

**Function values.** `Use(Operand::Function)` materializes a
capsule of the same 16-byte ARC shape as a closure:
`[code, NULL]` with `code = func_addr(lpp_{fn})` for a sync
function and `func_addr(__lpp_task_thunk{fn})` for an async one —
a function value is therefore an ARC node like a closure (the
oracle's bare function reference carries no ARC traffic of its
own, and the capsule's single reference is released by the
holding frame's exit pass, so the object stays self-balanced).
A call through a sync function value loads `code` from the
capsule and is a `call_indirect` with the function's own
signature; a call through an async function value builds the task
env tuple and `lpp_task_new(code, env, managed)`. A function-value
local is provenance-classed in the pre-scan (closure vs
function value; the two capsule call ABIs differ by the leading
`env` parameter) and a local mixing both origins is a typed
rejection.

**Task lowering.** An async call `f(args)` builds the env tuple
(managed arguments retained) and `lpp_task_new(&__lpp_task_thunk{f},
env, managed_flag)`; the handle is an `I64` local.
`__lpp_task_thunk{f}(env)` loads the argument slots, retains the
managed ones, calls the async function, and boxes the result to
`I64` (bool uextend, float bit-cast — the v1 thunk rule).
`x.await` lowers to `lpp_task_await(x)`. `spawn(closure)` loads the
closure's code pointer and env from the capsule, builds the env tuple
`[closure_env_ptr]` (retained), `lpp_task_new(code, env,
managed_flag)`, then `lpp_task_poll` and `lpp_task_destroy` — eager,
before the statement completes (validated programs only spawn
zero-parameter closures, matching the oracle's arity rule).
The task code for a spawned closure is
`__lpp_closure_thunk{n}` — a generated `(env) -> I64` trampoline
that invokes the closure function and boxes the result (void →
`0`, bool uextend, float bit-cast) — because `lpp_task_new`'s
code pointer requires the `int64_t (*)(void*)` task-code shape
while the closure function itself returns its user result type;
the capsule's code pointer is that thunk, so direct calls of a
zero-parameter closure also go through it and unbox at the call
site (a closure with at least one user parameter is never
spawnable and keeps `lpp_c{n}` as its capsule code, called
directly with `(env, args...)`). `spawn(f)` of a zero-parameter
function value uses the plain task thunk with an empty env. An **async entry** `main` is auto-drained in
the `lpp_main` wrapper: empty env tuple, `lpp_task_new`,
`lpp_task_await`, `lpp_task_destroy` (the execution boundary holds no
references). 4D `Callable` nodes carry edges to their capture types,
so closure-capture cycles are statically pinned; the ARC oracle's
balance proof tolerates the pinned types, and the object retains the
cycle with plain ARC (no special-case code).

**Builtin table (registry-derived over all 518 `BuiltinId`s).** Four
disjoint families (96 covered: 71 + 18 + 7, by name at this commit —
the gate recomputes from the registry per `BuiltinId`, since a few
names carry two ids), asserted exhaustively by the gate:

- **Family A — oracle-supported (71 source names, dual `lpp_`
  spellings included).** Lowered as an import of the descriptor's
  v1 symbol, typed per `SemanticAbiType`; argument semantics mirror
  the oracle: value arguments (`list_push@1`, `list_set@2`,
  `print@0`, `write_str@0`) transfer ownership, receivers and indices
  are plain reads (no retain).
- **Family B — SIMD (18 `vec_*` entries: the `vec_i64x2_*` and
  `vec_u8x16_*` operators, `vec_movemask8`, `vec_i64_checksum`).**
  Native Cranelift 128-bit instructions (the 5B deferral lifted).
- **Family C — slice handles (7 entries: `slice`, `str_slice`,
  `slice_len`, `slice_get`, `slice_get_bool`, `slice_to_str`,
  `str_slice_to_str`).** Import the shim's slice runtime (views over
  list storage). Not oracle-verifiable — the interpreter reports
  `UnsupportedBuiltin` for this family — so the corpus is
  self-checking with hand-computed stdout.
- **Family D — everything else** (webview, files, threads, `input`,
  arena/task/tuple/closure runtime internals): typed rejection
  `E5003` at the exact function.

**Gates** (five tests in `tests/phase5c2_gate.rs`)

1. **Differential (closures + tasks).** Counter closures, closures in
   lists, capture mutation through by-reference cells (an unmanaged
   `Int` and a managed struct), nested captures, managed captures, a
   closure-capture cycle (4D `pinned_types()` non-empty),
   async/await chains, double await, spawn of a capturing closure,
   async `main` auto-drain: object stdout equals the **ARC-mode**
   oracle (`execute_mir_arc` with the 4D `pinned_types()`), no
   `fail_*` marker, `all_ok` present.
   Corpus-domain note from the implementation evidence: the ARC
   oracle's `drop_reference` is a **no-op for pinned nodes** (pinned
   types are immortal; their children are never released), and
   `pinned_types()` is program-wide. If a non-cyclic closure shared
   its function type with a pinned cycle closure, its capsule would
   be pinned, never freed, and leak its unpinned managed captures.
   The corpus therefore gives the cycle closure a function type no
   other closure in the program shares.
2. **Differential (builtin families).** The Family A surface
   (print family, string ops, conversions, bit ops, checked
   arithmetic, float ops, list family): object stdout equals the
   ARC-mode oracle.
3. **Self-check (slices + SIMD).** Family B and C corpora with exact
   hand-computed stdout (`vec_i64x2` arithmetic/comparison/extract,
   `u8x16` splat/equality/movemask on the
   `test_simd_probe_only.lpp` values; slice views, `slice_len`,
   `slice_get`, `slice_to_str`).
4. **Determinism + census.** Two compiles produce byte-identical
   objects; exports include `lpp_main`, every user function, every
   closure function (`lpp_c{n}`), every generated destructor
   (aggregate drops in `MirAggregateId` order, closure env drops
   `lpp_drop_c{n}` in `MirFunctionId` order), and the used
   `__lpp_task_thunk{n}` / `__lpp_closure_thunk{n}` thunks; imports
   exactly the runtime symbols the corpus uses.
5. **Table census + rejections.** The gate enumerates the registry
   and asserts 518 = Family A ∪ B ∪ C ∪ D, disjoint and exact; a
   Family D builtin (`webview_window_create`) fails with the exact
   `E5003` at the exact function. This contract supersedes the 5C
   gate test 5 compile-fail expectations: closure and async programs
   now compile, and the `E5001` set shrinks to tuples. Every other
   existing workspace test stays green; strict all-target warnings;
   production stays on `LegacyEngine`.

**Not in 5C2 (stay typed rejections or unsupported):** async closures
(no frontend syntax), map types (no frontend syntax), trait/vtable
function pointers (no frontend emission), thread/file/webview/`input`
builtins (Family D), and the 5C tuple rejection.

### 5D — WASM data surface — approved contract (slice 1 implemented)

New crate `lpp-codegen-wasm` (dependencies: the same one-way set as the
Cranelift backend — `lpp-codegen-api`, `lpp-mir`, `lpp-types`,
`lpp-runtime-abi`; no HIR import). `WasmBackend` implements `Backend` for
a new `Target::Wasm32Wasi` and emits a **binary WebAssembly module** — a
self-contained `.wasm` image, no external assembler (no `wat`, no
`wasm-ld`), keeping the zero-extra-dependency promise. The v1
`src/backend/wasm.rs` (9,606 LOC) is the behavioral reference, not a
source to copy. Cranelift 0.113 has no `wasm` feature, so this is a
hand-written binary emitter regardless.

**Scope: slice 1 — the scalar data surface.** Integers (`Int -> i64`,
wrapping arithmetic, signed div/rem, bitwise, shifts including large and
negative amounts, all comparisons), booleans (`&&`/`||`/`!`, eq/ne), chars
(comparisons), floats (arithmetic, modulo, NaN, comparisons — no float
*printing*), control flow (`if`/`else`, nested, `while`, nested loops),
direct calls (multi-parameter, recursion), the string surface
(`print_str`/`str_len`/content `==`/`!=`/`write_str` without a newline),
and the `print` dispatch (int/bool/str/char). Value representation is
linear-memory `i32` offsets for strings and `i64`/`f64`/`i32` scalars; the
differential target is the **plain reference oracle**
(`execute_mir_with_stats`) — slice 1 has no managed types.

**Deferred to 5D2** (mirroring the 5C→5C2 split): the managed data
surface — structs/lists over a bump-allocator linear memory + the 24-byte
ARC header, `retain`/`release`, and generated destructors (the ARC-oracle
differential); the function-value surface (closures, function values,
async/await/`Spawn`/tasks); slices; SIMD (`VectorI64x2`); enums /
`SwitchEnum` / tuples; and float printing (byte-exact `{:.6}` in raw WASM).
`Target::Wasm32Wasi` is Phase 5's first 32-bit target (heap pointers are
`i32` linear-memory offsets); `CraneliftBackend::targets()` is unchanged.

**Control flow (goto CFG → structured WASM).** Each function body is laid
out in reverse post-order inside a `loop` dispatcher; the last guard is
followed by an explicit `br 0` to re-dispatch (a wasm `loop` does not
auto-repeat — its `end` falls through). `Goto`/`Branch`/`Return`/
`Unreachable` map onto it; a `SwitchEnum` lowers to an `if`/`else`
chain comparing the variant tag (5D2, with enums).

**WASI.** The module imports `wasi_snapshot_preview1.fd_write` (and
`proc_exit`) and exports `_start` (the WASI command entry, wrapping the
L++ `main`) and `memory`. A 2-arg `lpp_wasm_fd_write` helper wraps the
4-arg WASI import; the print family formats into a scratch region and
calls the helper.

**Builtin policy (registry-derived).** The oracle-supported
scalar/str/int/float/bool subset of the 5C2 Family A table is ported,
typed per `SemanticAbiType` with the same `semantic_result` bool reduction
as the Cranelift backend. Float printing and unported Family A entries are
`E5003`; structs (and the managed surface) are `E5001` (5D2 lifts them).

**Gates** (four tests in `tests/wasm_gate.rs`; the module runs under
Node.js `node:wasi` preview1 — an already-available host, so the gate adds
no Rust dependency):
1. **Differential (data-surface corpus).** Integers, booleans, chars,
   floats, CFG, direct calls, the string surface, and the print dispatch:
   the emitted module's stdout equals the **plain** oracle, no `fail_*`
   marker, `all_ok` present.
2. **Determinism.** Two compiles produce byte-identical `.wasm` images and
   identical export/import censuses; functions in `MirFunctionId` order.
3. **Compile-fail (struct).** A struct program fails with the exact
   `E5001` (the managed surface is 5D2).
4. **Compile-fail (float print).** A `print_float` program fails with the
   exact `E5003`.

**Production optimization pass (opt-in).** `CodegenOptions.opt_level`
(default `OptLevel::None`) selects a post-optimization level. For the wasm32
target a non-`None` level runs the emitted object through **`wasm-opt`** (a
system tool, like `clang` for the native backends — no Rust dependency) as a
one-time production pass, `-O1`…`-O4`. The default stays the raw,
byte-deterministic output, so the dev loop is unaffected. Measured on the
data-surface corpus, `-O3`/`-O4` yields ~20% smaller + ~35% faster with
byte-identical, oracle-verified output — pinned by the
`wasm_opt_level_smaller_correct_deterministic` gate (skips cleanly when
Binaryen is absent). Exposing this as a `lpp build --wasm-release` flag is
pending the codegen→driver integration (the backends are currently driven
only by the gate tests).

### 5D2 — WASM managed and function-value surfaces — approved contract

Slice narrowing (following the 5C→5C2 precedent): the 5D deferral list
bundled seven concerns. The **managed data surface** — structs, enums,
lists, the ARC heap, destructors, float printing — is **5D2a**; the
**function-value surface** — closures, function values, async/await/
`Spawn`/tasks, slices, SIMD, and the remaining builtins — is **5D2b**.
Each half is independently gateable against the Phase 4 oracle.

#### 5D2a — WASM managed data surface

Extended surface over 5D in `lpp-codegen-wasm` (same one-way dependency
set; no new external crates; still a self-contained binary module — the
runtime helpers are emitted in-module, so the import census stays exactly
`wasi_snapshot_preview1.fd_write` + `proc_exit`):

- **Heap (v1 wasm conventions, unchanged).** The bump pointer is a
  global initialized right after the static pool (string/constants);
  `Alloc(size) -> ptr` is 8-aligned with `memory.grow` of
  `max(need, 32)` pages and `unreachable` on OOM. Memory is never
  recycled (bump heap) — v1 `h_alloc` semantics, pinned by the gate.
- **ARC node (v1 wasm header, 24 bytes in front of the payload):**
  `refcount i64 @0`, `drop index i64 @8` (a wasm **table index**, not a
  pointer — 0 = no destructor), `magic 0x41524331 ("ARC1") @16`.
  `ArcAlloc(size, drop_idx)` = `Alloc(size + 24)` with header
  `rc = 1`, `drop = drop_idx`, `magic`, returning the payload offset.
  This differs from the native 24-byte header on purpose: it is the v1
  *wasm* runtime layout, and 5D2a must match the v1 wasm backend, not
  the native ABI.
- **`Retain(p)` / `Release(p)` (v1 semantics).** Both no-op on null.
  Retain: no-op when `rc == ARC_IMMORTAL` (string-literal sentinel),
  else `rc += 1`. Release: no-op on immortal; `rc < 1` is a double
  release → `unreachable` (loud failure, matching v1); `rc == 1` marks
  the node dead (`rc = 0`) and dispatches the destructor through
  `call_indirect` when `drop_idx != 0`; else `rc -= 1`.
- **Value representation.** Every managed value (struct, enum, list,
  string) is an `i32` linear-memory **payload** offset (the header sits
  24 bytes below it). Scalars stay on the wasm stack/locals
  (`i64`/`f64`/`i32`) exactly as in 5D. Call/return of managed values
  passes one `i32` — no new calling machinery.
- **Struct layout.** Fields in declaration order with native alignment
  from offset 0 (the v1 `struct_layout` rule: Int/Float/pointer `8:8`,
  Char `4:4`, Bool `1:1`); allocation is `ArcAlloc(size, drop_idx)`.
  `Load`/`Store` on field projections lower to `load64`/`load32`/
  `load8_u` + `store*` at `payload + offset`.
- **Enum layout (rewrite-defined, as in 5C).** An `i64` variant-ordinal
  tag at payload offset 0; the active variant's fields from offset 8
  with native alignment; allocation size = max over variants, 8-aligned.
  `ConstructVariant` writes the tag; **`SwitchEnum` lowers to an
  `if`/`else` chain** comparing the tag against each ordinal (the 5D
  dispatcher loop already gives every arm a block landing).
- **Lists (v1 wasm node: 32-byte payload).** `[data i64 @0][len i64
  @8][cap i64 @16][is_arc i64 @24]` allocated via `ArcAlloc(32,
  list_destroy)`. Element slots are raw 8 bytes; `ListPush` grows
  ×2 from 8 with a copied bump "realloc" and panics on null list or
  `cap > 0x1000_0000` (v1 messages, byte-identical). Element
  get/set/push/len exist per element class (`i64`, `f64`, `bool`,
  `arc`); the `arc` variants retain on store and release on replace,
  mirroring `lpp_list_set_arc` on native.
- **ARC traffic (mirrors the 5C rules over the 4E consume-vs-borrow
  table).** Retain points: `Use(Copy)` of a managed local, `Load` of a
  managed place, `Store` from a `Copy` into a bare local or field.
  Release points: reassignment of a bare managed local, replacement of
  a managed field, and the per-`Return` release pass over all managed
  locals in declaration order. Null-on-move: moved-out slots are written
  with 0 at the move site; `Release(0)` is a no-op, so every release
  point is unconditional.
- **Destructors.** One generated, **module-internal** function per
  constructed nominal aggregate — `lpp_drop_s{raw}` /
  `lpp_drop_e{raw}` in `MirAggregateId` order — releasing managed
  fields in declaration order (enums dispatch on the tag). They are
  referenced by table index in the header; the table (and its
  signature `(i32) -> ()`) is emitted only when at least one destructor
  exists.
- **String heap.** String constants live in the static pool with
  `rc = ARC_IMMORTAL` (retain/release no-ops — same as native 5C);
  runtime-constructed strings (`StrAlloc`/`StrNew` from the builtin
  subset) are `ArcAlloc` nodes with the same magic.
- **Float printing (byte-exact `{:.6}`).** The v1 wasm `PrintFloat`
  formatting algorithm is ported verbatim (sign, integer part via
  decimal division, six fixed fractional digits, `-0.000000`
  canonicalization per v1); `print` dispatch gains the float arm.
- **Builtin subset.** 5D's ported Family A entries plus the list
  family (`list_new`/`list_push`/`list_get`/`list_set`/`list_len` per
  element class) and `print_float`. Symbols/behaviors come from
  `BuiltinId::descriptor` (no string matching). Everything else stays
  typed-rejected.

**Not in 5D2a (typed rejection):** closures and their calls, function
values, async/`Await`/`Spawn`, tasks, slices, SIMD (`VectorI64x2`),
maps, and `Rvalue::Tuple`/`TupleField` (the frontend emits no tuples —
same decision as 5C): `E5001` at the exact function. These are 5D2b.

**Gates** (extend `tests/wasm_gate.rs`; the module still runs under
Node `node:wasi`):
1. **Differential (managed corpus).** Structs (construction, field
   read/write, nesting, reassignment, mixed-width layout, struct-in-
   struct, managed fields), enums (dense match dispatch, payload
   binding, wildcard arms, match on parameter and local), lists
   (literals of every element class, read/write, `list_len`,
   reassignment, managed elements, nested lists, list-in-struct),
   runtime string construction, and float printing: stdout equals the
   **ARC-mode** oracle (`execute_mir_arc` with the 4D
   `pinned_types()`), no `fail_*` marker, `all_ok` present.
2. **Differential (ARC stress corpus).** Aliasing, self-assignment,
   field/element swaps, move-then-reassign, cross-container stores,
   deep nesting, double-release program (must trap identically to the
   oracle's balance failure): object behavior equals the ARC oracle.
3. **Determinism.** Two compiles produce byte-identical `.wasm` images
   and identical import/export censuses; functions in `MirFunctionId`
   order, destructors in `MirAggregateId` order, helpers in the fixed
   5D order; the drop table is emitted only when used.
4. **Structure census.** Exports are still exactly `_start` +
   `memory`; imports exactly `fd_write` + `proc_exit`; the internal
   drop functions are named `lpp_drop_s{raw}` / `lpp_drop_e{raw}` and
   appear in the table in `MirAggregateId` order; the list destroyer
   is `lpp_drop_list` and the no-destructor no-op (table slot 0) is
   `lpp_drop_none` — the table is
   `[lpp_drop_none, drops in MirAggregateId order, lpp_drop_list]`.
5. **Compile-fail.** An async program, a `VectorI64x2` program, and a
   slice program each fail with the exact `E5001` at the exact
   function (5D2b surface); 5D's struct compile-fail is **lifted**
   (structs now lower), 5D's float-print compile-fail is **lifted**
   (floats now print), and this gate's closure compile-fail is
   **lifted by 5D2b slice 1** (closures now lower and run).
   All 5D gates and every existing workspace test stay green;
   production stays on `LegacyEngine`.

#### 5D2b — WASM function-value surface — implemented (slices 1–2: closures, bare function values)

Slice 1 lands **sync closures and capsule calls** over the 5D2a
surface (same one-way dependency set; no new external crates; the
import census stays exactly `fd_write` + `proc_exit`):

- **Capture cells (5C2 by-reference semantics, unchanged).** A
  capture is the *cell* (list-of-one for scalars, the managed value
  itself otherwise), shared between the outer function and the
  closure frame; the closure mutates it in place.
- **Capsule (16-byte ARC node).** `[code i64 @0][env i64 @8]` via
  `ArcAlloc(16, closure_destroy_slot)`; `code` is the closure's
  dispatch-table index, `env` the env node's pointer (0 for
  zero-capture closures).
- **Env node (N×8 bytes, ARC).** One slot per capture, in closure
  frame order; slot `i` at offset `8i`, width per capture class
  (`i32` pointer / `i64` Int / `f64` Float). The env owns one
  reference per managed capture (`Retain` at `MakeClosure`, released
  by the env's destructor); the closure frame's capture slots are
  **views** — loaded from the env at entry, and the exit release pass
  skips `Capture`-kind locals — so no double-free at env destruction.
- **Dispatch table.** A second funcref table (table 1; table 0 when
  there is no drop table) filled at offset 0 with the closure
  functions in `MirFunctionId` order. The drop table (table 0) gains
  the per-closure env destructors `lpp_drop_c{raw}` (in
  `MirFunctionId` order over capturing closures, after the
  aggregate/list slots) and `lpp_closure_destroy` (releases the env
  word; a NULL env is a no-op). A closure-only program still gets the
  heap, the drop table, and the dispatch table.
- **Call.** `Call(Copy(capsule), args)` lowers to: push the env word
  (`load64(capsule+8)` wrapped to `i32`), push the user arguments,
  push the code word, then `call_indirect` on the dispatch table with
  the closure's call type `(i32 env, user params...) -> result`
  (registered once per closure type; frame capture slots are not user
  parameters). A non-local capture operand is a compile error.

**Gates** (extend `tests/wasm_gate.rs`; the module still runs under
Node `node:wasi`):
1. **Differential (closure corpus).** Nine checks: counter cell
   mutation through a closure, list-of-closures calls, nested capture
   arithmetic, a managed capture (list cell), sync value capture, a
   two-capture cycle, a read-only managed capture, a minimal Int
   capture, and calling the same closure twice — stdout equals the
   ARC oracle, exit 0.
2. **Closure smoke.** A closure-only program (Int capture, no
   list/struct) compiles, dispatches, and exits 0 only if the
   captured value read through the env is correct.
3. **Compile-fail update.** The async and tuple rejections (exact
   `E5001`) stay; 5D2a gate 5's closure compile-fail is **lifted**
   (closures now lower and run).
4. **Determinism + census.** The 5D2a byte-determinism and
   census gates extend over closure programs; the import/export
   censuses are unchanged.

**Slice 2 — bare function values** lands `Operand::Function` in
value position (`f := add`) over the slice-1 dispatch surface:

- **Uniform dispatch ABI.** Every lowered wasm function — plain or
  closure — takes the env pointer as local 0 (closures read capture
  slots from it; plain functions receive NULL and ignore it). The
  dispatch table is filled with **every** sync function in
  `MirFunctionId` order, and every call type in the table is
  `(i32 env, user params...) -> result`, so a capsule may dispatch to
  any function of its type and `call_indirect` signature checks hold.
  Direct (by-name) calls and `_start`'s `main` call push `env = 0`.
- **Capsule (16-byte ARC node).** A bare function value materializes
  the no-capture capsule `[code i64 @0][env=0 i64 @8]` via
  `ArcAlloc(16, closure_destroy_slot)` — the same capsule and the
  same destroyer as a zero-capture closure. The fresh capsule starts
  at rc = 1 and ownership transfers to the use site (no retain).
- **Flow.** Assign, copy, list elements, fields, and `Return` treat
  the capsule as a `Ptr`-class managed value; the pre-scan flags the
  program (`has_function_value`) so the dispatch table and destroyer
  are emitted even when the program has no closures.

**Slice 2 gates** (extend `tests/wasm_gate.rs`): a five-check
differential corpus vs the ARC oracle — aliasing two names to one
function and calling both, swap through temporaries, reassignment of
a function local (retain/release balance), a function stored in a
list element, and calling the same value twice — plus the existing
closure corpus, which must stay green under the uniform ABI.

**Not in slices 1–2 (typed rejection, exact `E5001`):** async
functions, `Await`, `Spawn` with task nodes, slices, SIMD
(`VectorI64x2` as two `i64` lanes), and the remaining Family A
builtins — slice 3 onward, each ported from the v1 wasm reference
and differentially gated against the 5C2-style corpus.

### 5E — LLVM backend — implemented (slice 1)

New crate `lpp-codegen-llvm` (dependencies: the same one-way set as the
other backends; no HIR import). `LlvmBackend` implements `Backend` for
`Target::X86_64` (the native target set the Cranelift backend already
covers) and emits **textual LLVM IR** (`.ll`), compiling it to an
ELF relocatable object via a shell-out to `clang -c`; the object links
against the 5B/5C `c_shim.c` runtime, matching the v1 runtime semantics.
The v1 LLVM backend (~1.7k LOC) is the behavioral reference. `clang` is a
host toolchain (like the 5B `cc`), so the crate adds no Rust dependency
and no lockfile entry. LLVM IR is a natural SSA/CFG IR, so the MIR goto
CFG maps 1:1 onto LLVM basic blocks/`br`/`ret` (no dispatcher loop).

**Scope: slice 1 — the scalar data surface** (the same scalar/CFG/call/
builtin surface as 5D slice 1, expressed in LLVM IR). Value
representation: `Int -> i64`, `Float -> f64`, `Bool -> i8` (0/1),
`Char -> i32` (code point), `Void -> i64`. Strings are `ptr` into
`private unnamed_addr constant` string globals, deduplicated per literal
(`MirStringId` order). Direct calls, recursion, and the scalar/str/bool/
char builtin subset (print family, `str_len`, content `==`/`!=`,
conversions, integer helpers, float ops) are ported, typed per
`SemanticAbiType`. The differential target is the **plain reference
oracle** (`execute_mir_with_stats`) — slice 1 has no managed types.

**Deferred to 5E2** (mirroring the 5C→5C2 / 5D→5D2 split): the managed
data surface (structs/lists + ARC), the function-value surface (closures,
function values, async/await/`Spawn`/tasks), slices, SIMD (`VectorI64x2`),
and enums / `SwitchEnum` / tuples — all `E5001` until lifted.

**5E2a — float-output slice — done.** `print_float` (formerly the lone
`E5003` rejection) now lowers to `call void @lpp_print_float(double …)`
plus a `declare void @lpp_print_float(double)`, matching the Cranelift
and WASM backends. The runtime prints `printf("%f\n", …)` (6 decimals),
byte-identical to the oracle's `{:.6}` formatting (and to the WASM
backend's `{:.6}`), so all three backends now agree on float output.
Gate: `float_print_matches_oracle` in `llvm_gate.rs` replaces the old
`E5003` compile-fail with a differential over float constants and a
float local. Remaining 5E2: the managed/function-value/slice/SIMD/enum
surfaces (all still `E5001`).

**Implementation.** The lowering is a plain **non-SSA alloca emitter**: one
`alloca` per MIR local at the entry block, reads = `load`, writes =
`store`, and the MIR goto CFG maps 1:1 onto LLVM blocks/`br`/`ret` (no phi
nodes, no dispatcher loop). Floats are emitted as the `double` type and
IEEE decimal literals (the host `clang` rejects `f64` and constexpr
`sext`/`trunc`); `main` is `define i32` (the C entry point) while void
helpers are `define void`. Because the non-SSA IR is correct only at
`-O0` (the LLVM optimizer miscompiles the load/store pairs that carry
program state at `-O1` and above), the object is compiled with `clang -c
-w -O0`; raising the optimization level is a 5F concern once the lowered
IR is proven correct. The 4-test gate: differential data-surface corpus vs
the plain oracle (linking the 5B `c_shim.c`), byte-identical objects across
compiles, and exact `E5001` struct / `E5003` float-print compile-fails.

**Gates** (four tests in `tests/llvm_gate.rs`; the object is linked with
`clang` + `c_shim.c` + `-lm` and executed, mirroring the 5B gate):
1. **Differential (data-surface corpus).** Integers, booleans, chars,
   floats, CFG, direct calls, the string surface, and the print dispatch:
   the object's stdout equals the **plain** oracle, no `fail_*` marker,
   `all_ok` present.
2. **Determinism.** Two compiles produce byte-identical `.o` objects and
   identical export/import censuses; functions in `MirFunctionId` order.
3. **Compile-fail (struct).** A struct program fails with the exact
   `E5001`.
4. **Compile-fail (float print).** A `print_float` program fails with the
   exact `E5003`.

### 5F — backend safety exit — implemented

The final Phase 5 slice is a **cross-backend safety exit**, not new
lowering. It lives in a new test-only crate `lpp-codegen-gates` (an empty
lib plus one integration test; it depends on all three backends —
Cranelift, WASM, LLVM — plus the pipeline crates, so no backend lib gains
an import it does not already have, and `lpp-hir` stays a dev-dep only).
The three backends are exercised on the **shared scalar corpus** — the
data surface every slice-1 backend supports — so the gate is a genuine
cross-validation: a backend-specific divergence surfaces as a difference
against the reference oracle.

**Gates** (in `tests/phase5f_gate.rs`):
1. **Cross-backend corpus differential.** The shared scalar corpus (the
   same data-surface surface as 5B/5D/5E) is compiled and executed on
   Cranelift (`X86_64`), WASM (`Wasm32Wasi`), and LLVM (`X86_64`). Each
   backend's stdout must equal the plain reference oracle
   (`execute_mir_with_stats`), carry no `fail_*` marker, and end in
   `all_ok`. Two of the three backends link against the 5B `c_shim.c`
   and run natively; the WASM object runs under Node `node:wasi`.
2. **Deterministic object-size baseline.** For each backend, two compiles
   produce byte-identical objects (and identical censuses); the object
   size is asserted stable across compiles as the recorded size baseline
   per `OPTIMIZATION_STRATEGY.md` (deterministic build artifacts).
3. **Sanitizer run.** The native backends (Cranelift and LLVM) are
   re-linked with `-fsanitize=address,undefined` and executed; a clean
   exit with no sanitizer report is required. (The WASM object is
   validated by its Node run, not the native sanitizers.)

**Out of scope (deferred):** raising the LLVM object above `-O0` and the
managed/function-value/slice/SIMD surfaces on WASM/LLVM — those are the
5D2/5E2 lift, not the safety exit. The corpus here is deliberately the
shared scalar surface so all three backends participate; each backend's
full-surface coverage already lives in its own slice gate (5B/5C/5C2,
5D, 5E).

**Implemented.** The `lpp-codegen-gates` test-only crate is added (empty
library + `tests/phase5f_gate.rs`, depending on the three backends and,
dev-only, `lpp-hir`/`lpp-ownership`) and carries the three gates, all
green:

- **`cross_backend_corpus_agrees`** — the 12-line shared scalar corpus
  (int/float/bool/char arithmetic + comparison, string literals, `print`
  for every scalar type, `str_len`/`str_contains`/`str_concat`) is
  compiled by **all three** backends and executed against the Phase 4B
  oracle; the oracle is authoritative (`all_ok`, no `fail_*`) and the
  three backends emit **byte-identical stdout**. Native (Cranelift, LLVM)
  link `c_shim.c`; the WASM object runs under `node:wasi`.
- **`deterministic_object_sizes`** — two compiles per backend are
  byte-identical, the object size is stable, and the exported/imported
  symbol census is identical.
- **`sanitizer_clean`** — the native backends are re-linked with
  `-fsanitize=address,undefined` and execute the corpus with a clean
  exit and no sanitizer report.

Running the gate immediately caught a **latent Cranelift bug**: the
print dispatch routed a `char` argument to `lpp_print_int` (an `i64`
parameter) while lowering the char as `i32`, so the Cranelift verifier
rejected it (`arg 0 has type i32, expected i64`). No earlier corpus
exercised char-print, so it had never surfaced. The fix widens a char
argument to `i64` (`uextend`) before the `lpp_print_int` call — the same
rule the LLVM backend already applied — and the 5F corpus now pins it.
This also closes a gap: 5B's scalar corpus never printed a `char`.

### Later Phase 5 slices

- **5D2a — WASM managed data surface — implemented (above):**
  structs, enums, lists, the ARC heap over the bump allocator,
  generated destructors, runtime strings, and byte-exact float
  printing.
- **5D2b — WASM function-value surface — implemented, slices 1–2
  (above):** sync closures, capsule dispatch (by-reference capture
  cells as in 5C2), and bare function values over the uniform
  env-first dispatch ABI. **Next slice:** async/`Await`/`Spawn`/
  tasks, then slices, SIMD, remaining builtins.
- **5E — LLVM backend** (`lpp-codegen-llvm`, v1 reference 1.7k LOC).
- **5F — backend safety exit:** full v1 corpus differential,
  deterministic object-size gate, size/perf baseline per
  `OPTIMIZATION_STRATEGY.md`, sanitizers.

Slice boundaries may be narrowed further when implementation evidence
demands it, but later concerns must not be pulled into an earlier slice
without updating this contract first.

## Current validation

- 5A implemented: `lpp-codegen-api` (Target, MachineType, the dual
  `AbiType -> MachineType` maps, `BuiltinLowering` from the generated
  table, `Backend`/`CompiledModule`/`NameResolver`, the `E5xxx`
  table) with the 5-test 5A gate (518/518 builtin parity, pinned
  semantic-vs-machine divergence census, target lattice, E-table).
- 5B implemented: `lpp-codegen-cranelift` (`CraneliftBackend` for
  X86_64/Aarch64, cranelift-object 0.113) with the 5-test 5B gate
  (differential execution vs the Phase 4 oracle through a `cc` +
  libm link, byte-identical objects across compiles, ELF + symbol
  census, explicit `E5001`/`E5003` compile-fails).
- 5C implemented: the Cranelift aggregate data surface (structs,
  lists, the ARC heap, `retain`/`release`, generated destructors) with
  the 5-test 5C gate against the ARC-mode oracle.
- 5C2 implemented: the Cranelift function-value surface (closures with
  by-reference capture cells, function values, async/await/`Spawn`/
  tasks) and the builtin families (Family A/B/C + the SIMD/slice
  self-check) with the 5-test 5C2 gate.
- 5D implemented (slice 1): `lpp-codegen-wasm` (`WasmBackend` for
  `Wasm32Wasi`, a hand-written binary emitter) with the 4-test gate
  (differential data-surface corpus vs the plain oracle under Node
  `node:wasi`, byte-identical objects, exact `E5001` struct and
  `E5003` float-print compile-fails). The managed data surface
  (structs/lists/ARC), the function-value surface, slices, SIMD, and
  float printing are deferred to 5D2.
- 5E implemented (slice 1): `lpp-codegen-llvm` (`LlvmBackend` for X86_64,
  textual LLVM IR + `clang -c` shell-out, linking the 5B `c_shim.c`
  runtime), slice 1 = the scalar data surface, mirroring the 5D gate, with
  the 4-test gate (differential data-surface corpus vs the plain oracle,
  byte-identical objects, exact `E5001` struct and `E5003` float-print
  compile-fails). The managed data surface, the function-value surface,
  slices, SIMD, and float printing are deferred to 5E2.
- 5D2a approved contract (above): the WASM managed data surface
  (structs, enums, lists, ARC heap, destructors, float printing) with
  its 5-gate spec. Implementation is the next step.
- Baseline before 5A/5B: 377 workspace Rust tests pass in debug and
  release (347 after 4E plus the 23-test 4F gate suite and the 7-test
  4F ownership suite), strict all-target warnings, Clippy
  correctness/suspicious clean, 4F suites green under Miri (23/23
  and 7/7).
