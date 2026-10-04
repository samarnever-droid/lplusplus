# Rewrite Phase 4 — typed MIR and ownership

## Rewrite roadmap (Phases 0–7)

1. **Phase 0 — secure and freeze** — complete.
2. **Phase 1 — workspace and common contracts** — complete.
3. **Phase 2 — frontend and modules** — complete.
4. **Phase 3 — HIR, types, generics, and traits** — complete.
5. **Phase 4 — MIR and ownership** — active: typed CFG construction,
   verification, pass management, cycle breaking, escape analysis, ARC/move-out
   planning, and optimizer equivalence.
6. **Phase 5 — backends** — planned.
7. **Phase 6 — linker and runtime** — planned.
8. **Phase 7 — CLI, PM, LSP, and cutover** — planned.

The root compiler remains routed through `LegacyEngine`. New MIR is shadow-only
until the Phase 4 differential and ownership gates pass.

## Approved Phase 4 architecture

The stage boundary is:

```text
TypedHIR + GenericInstances
  -> typed MIR CFG
  -> structurally and semantically validated MIR
  -> ownership plan (Frame / Owned / Shared + arena strategy)
  -> optimized and revalidated MIR
  -> Phase 5 backend contract
```

Dependencies remain one-way: `lpp-hir -> lpp-types -> lpp-mir -> lpp-passes`.
Ownership policy will live in `lpp-ownership` and consume validated MIR; MIR must
not import a backend, runtime implementation, legacy AST, or legacy MIR.

Every pass declares its required and preserved invariants. Debug and CI paths
verify MIR after every pass. Observable MIR serialization uses stable IDs and
source order, never hash-map iteration.

## Phase 4 slices

### 4A — typed CFG, verifier, and pass contract — implementation complete

**Bounded implementation scope**

- Add `lpp-mir` with four-byte function/block/local/instruction IDs,
  eight-byte compact list ranges, contiguous arenas, and append-only operand and
  ID lists.
- Model typed functions, basic blocks, locals, three-address instructions,
  constants/operands/rvalues, and explicit `goto`, boolean branch, return, and
  unreachable terminators.
- Build MIR from fully typed HIR for non-generic function bodies covering
  parameters and locals; literals; local and direct-function names; unary and
  binary expressions; tuples and lists; direct/indirect typed calls; local
  assignment; expression and return statements; `if`; `while`; `break`; and
  `continue`.
- Reject constructs outside this first lowering boundary with structured,
  origin-carrying diagnostics. No silent fallback or partial function is
  returned.
- Verify local/block/function references, same-function CFG edges, entry-block
  membership, operand and rvalue typing, boolean branch conditions, and return
  types.
- Add deterministic full-MIR serialization and an exact checked-in snapshot.
- Add explicit builder capacity/work limits and structural size/growth tests.
- Add `lpp-passes` with the `MirPass` contract, required/preserved invariant
  declarations, deterministic pass sequencing, and verify-after-each-pass
  behavior. This slice adds no scalar optimization pass.

**Explicitly deferred from 4A**

- Generic-instance-specific MIR materialization and substitution.
- Closures, `try`, async/await, spawn, `for`, `match`, field/index stores, and
  ABI-aware builtin lowering.
- Definite-initialization dataflow and backend-legality validation.
- Cycle breaking, escape lattice, arena/frame placement, ARC insertion,
  move-out planning, and ownership-balance proof.
- Scalar optimizations and the unoptimized-versus-optimized MIR interpreter.

These are subsequent Phase 4 slices, not omissions from the full Phase 4 exit.
Keeping them out of 4A prevents ownership decisions from becoming implicit in
an unverified data model.

**4A source distribution**

```text
crates/lpp-mir/
  src/ids.rs       compact MIR identities
  src/storage.rs   compact ranges and append-only list storage
  src/ir.rs        typed MIR data model and invariants
  src/builder/     bounded TypedHIR-to-MIR CFG builder by concern
  src/verify/      structural/CFG and operand/type verification
  src/snapshot.rs  deterministic observable contract
  src/lib.rs       stage API only
  tests/           builder, verifier, limits, and exact snapshots

crates/lpp-passes/
  src/manager.rs   pass trait, invariant state, sequencing, revalidation
  src/lib.rs       pass API only
  tests/           precondition, preservation, and invalid-pass regressions
```

**4A exit gate**

1. A representative typed package lowers to the exact deterministic MIR
   snapshot twice.
2. Positive and deliberately malformed CFG/type programs prove every 4A
   verifier class.
3. The pass manager rejects unmet preconditions and rejects invalid output from
   a pass before another pass can observe it.
4. IDs/ranges retain compact layout, builder growth is bounded, and repeated
   lowering does not allocate per-node operand vectors.
5. Full Phase 3 shadows, compatibility gates, workspace tests, strict rewrite
   Clippy, frozen fixtures, classified source baseline, native AOT, WASI, and
   local CI remain green.
6. Production engine selection is unchanged.

**4A implementation evidence**

- `MirFunctionId`, `BasicBlockId`, `MirLocalId`, `InstructionId`, and their
  optional forms are four bytes; `ListRange<T>` is eight bytes.
- Final MIR owns no per-node operand vectors. Functions, blocks, locals, and
  instructions use typed contiguous arenas; parameters, function-local IDs,
  block IDs, instruction IDs, and operands use shared append-only stores.
- The representative two-function program lowers `if`, `while`, nested
  `break`, tuples, lists, arithmetic/comparison operations, local mutation, and
  a direct call into 2 functions, 11 blocks, 15 locals, and 16 instructions.
  Its full checked-in snapshot is byte-identical across repeated builds.
- The verifier reports stable reference (`E4101`), CFG (`E4102`), and type
  (`E4103`) classes. The pass manager reports initial-invalid, unmet-
  precondition, pass-failure, and invalid-output classes `E4201` through
  `E4204`, and invalid output is rejected before the next pass runs.
- The release structural gate grew 100 to 1,000 straight-line bindings from
  200 to 2,000 locals, 200 to 2,000 instructions, 401 to 4,001 shared-list
  entries, and 60,485 to 616,391 snapshot bytes. All dimensions remain within
  the declared linear-ratio limits.
- New MIR and pass crates are included in hosted, Unix, and PowerShell strict
  warning-clean Clippy gates. No production driver or legacy engine route
  imports them.

### 4B — definite initialization and executable MIR oracle — implementation complete

**Bounded implementation scope**

- Add deterministic forward must-initialization analysis over validated CFGs.
  Parameters begin initialized; a local becomes initialized only after its
  rvalue operands are read; join states use intersection; unreachable blocks
  do not poison reachable states.
- Use dense bitsets indexed by function-local position, stable block order, and
  explicit state-word/iteration limits. Do not allocate one set node per local
  or depend on hash iteration.
- Report an origin-carrying `E4104` diagnostic for every reachable read that is
  not definitely initialized.
- Promote `DefiniteInitialization` into the pass manager's validated invariant
  set and re-establish it after every pass.
- Add a deterministic, bounded MIR interpreter for `Void`, integer, float,
  boolean, tuple, list, and function values; unary/binary operations; calls;
  branches; loops; and returns.
- Bound interpreter steps, call depth, and aggregate elements with typed
  execution errors. Reject invalid MIR before execution.
- Differentially execute equivalent structured programs and hand-mutated
  non-semantic MIR variants where practical, establishing the oracle needed by
  later optimization slices.

**Explicitly deferred from 4B**

- Generic-instance-specific MIR materialization and substitutions.
- Closures, async/await, spawn, `for`, `match`, field/index access and stores,
  strings requiring source-buffer materialization, and ABI-aware builtins.
- Ownership classification, cycle breaking, ARC/move-out, and scalar/CFG
  transformations. No ownership-blind optimization pass is introduced merely
  to claim an optimized run.

**4B source distribution**

```text
crates/lpp-mir/src/
  verify/definite.rs  bounded must-initialization dataflow
  interpret/mod.rs    public values, limits, errors, outcomes, and work stats
  interpret/eval.rs   deterministic evaluator and resource accounting
crates/lpp-mir/tests/
  interpreter.rs      execution, differential, limit, and scaling gates
```

**4B exit gate**

1. Diamond and loop CFGs prove intersection/fixed-point behavior; malformed
   MIR proves `E4104`; unreachable reads remain non-diagnostic.
2. Recursive calls and infinite loops hit explicit depth/step limits.
3. Representative arithmetic, call, branch, loop, tuple, and list programs
   execute deterministically to exact values.
4. Pass-manager tests prove definite initialization is established initially
   and invalid output is rejected before the next pass.
5. Phase 4A and all Phase 0–3 compatibility/performance gates remain green;
   production routing remains legacy-only.

**Implementation evidence**

- `verify_mir` now runs definite initialization only after reference, CFG, and
  type validation succeeds. It computes reachability from each function entry,
  excludes unreachable predecessors, initializes parameter bits at entry,
  intersects predecessor outputs, and checks reads before setting each
  instruction target bit.
- States are dense function-local `u64` bitsets in stable MIR block order. The
  public `verify_mir_with_limits` API distinguishes state-word and fixed-point
  iteration exhaustion as `E4105`; defaults are 16,000,000 state words and
  1,000,000 iterations. Reachable uninitialized reads are origin-carrying
  `E4104` diagnostics.
- `DefiniteInitialization` is in `CORE_MIR_INVARIANTS`. A pass-manager
  regression deliberately redirects a valid instruction target, observes
  `E4104` immediately after that pass, and proves the following pass never
  sees invalid MIR.
- `execute_mir` verifies the complete MIR first, then evaluates typed void,
  integer, float-bit, boolean, tuple, list, and function values. It implements
  calls, unary and binary operators, branches, loops, and returns with wrapping
  integer arithmetic and deterministic float-bit results. Site-local failures
  retain function, block, and origin evidence.
- Interpreter defaults are 1,000,000 steps, call depth 256, and 1,000,000
  aggregate elements. `E4301` identifies invalid MIR/input typing, `E4302`
  identifies a resource limit, and `E4303` identifies a runtime semantic
  failure. `execute_mir_with_stats` returns steps, calls, peak call depth, and
  constructed-or-copied aggregate-element work.
- The structural execution gate grows 100 to 1,000 source bindings and records
  exactly 201 to 2,001 interpreter steps with one call in each run, remaining
  within the declared tenfold work-ratio bound. Equivalent expression trees
  and repeated CFG execution produce exact equal observable values.
- The Phase 4B suites contain 10 MIR builder/verifier tests, 6 interpreter
  tests, and 4 pass-manager tests. They pass in debug and release profiles.
  The complete workspace remains at 230 passing tests, strict rewrite Clippy is
  warning-clean, and the ten-stage Unix local CI harness remains 100% green.

### 4C — complete feature and instance MIR — active

The feature-completion boundary is split into three ordered internal gates so
that ownership policy is not smuggled into syntax lowering and the legacy
string-dispatch builtin table is not copied into the rewrite.

#### 4C1 — generic-function MIR instances — implementation complete

**Bounded implementation scope**

- Consume only complete, diagnostic-free required `InstancePlanner` records.
  Generic templates do not become executable MIR; each demanded function
  `InstanceId` materializes once with its ordered concrete type arguments.
- Apply canonical `TypeSubstitution` to the function signature and every local,
  expression, and explicit type argument under separate MIR type-work and
  type-depth limits. The shared `TypeInterner` remains the source of `TypeId`
  identity.
- Preserve the Phase 3 stable instance IDs in `MirFunction`; use indexed
  `(item, arguments)` and `(item, concrete function type)` resolution for
  explicit and inferred generic calls. Do not mangle names, clone HIR, scan
  bodies for call targets, or build an argument Cartesian product.
- Reset source-local mappings between materialized functions so two instances
  can never share a MIR local. Run full reference, CFG, type, and definite-
  initialization verification over the result.
- Extend the interpreter differential with inferred, explicit, nested, and
  recursively reached concrete instances. Record deterministic function/code
  growth and exact observable values.

**4C1 source distribution**

```text
crates/lpp-mir/src/
  builder/mod.rs         instance worklist, indexes, and type-work budget
  builder/function.rs    per-instance substitution and isolated local mapping
  builder/expression.rs  explicit/inferred generic call resolution
  ir.rs                  stable optional InstanceId on materialized functions
  snapshot.rs            observable instance identity
  verify/mod.rs          instance metadata and uniqueness validation
crates/lpp-mir/tests/
  generic_instances.rs   materialization, execution, limits, and growth gates
```

**4C1 exit gate**

1. One generic template demanded at two concrete types emits exactly two
   concrete MIR functions and no template function.
2. Explicit and inferred calls select the same stable instance; nested generic
   dependencies execute to exact values and repeated builds are identical.
3. Every function/local/instruction type verifies after substitution, instance
   locals are disjoint, malformed/incomplete plans and type-work exhaustion
   produce structured diagnostics, and growth remains proportional to unique
   demanded instances rather than call count.
4. Production routing and all Phase 0–4B gates remain unchanged.

**4C1 implementation evidence**

- `build_mir` now accepts the mutable shared `ShadowTypeOutput` so newly
  concrete structural types are interned canonically and remain available to
  verification and later stages. MIR type substitution defaults to 8,000,000
  work units and depth 256; exhaustion is distinguished as `TypeWork` or
  `TypeDepth` capacity failure.
- Non-generic functions retain source order. Required function-instance records
  follow their stable Phase 3 `InstanceId` order. Every materialized
  `MirFunction` carries both that ID and its canonical ordered `TypeListId`;
  generic templates and optional specialization records emit no executable
  function.
- Explicit generic calls resolve by `InstanceKey`; inferred calls resolve from
  a precomputed `(HirItemId, concrete function TypeId)` index. A per-instance
  substitution cache avoids repeating structural rewrites for the same HIR
  type, and source-local mappings are cleared after each function.
- MIR snapshot schema `mir-v2` exposes instance identity and argument-list
  identity. Verification rejects duplicate instance IDs and missing/spurious
  instance arguments as `E4101`; invalid/incomplete instance plans and missing
  call instances use builder class `E4006`.
- Five integration tests cover two-type inferred/explicit demand, omitted
  templates, skipped optional-specialization records, disjoint instance locals,
  nested and recursive dependencies, internal tuple/list substitution, exact
  execution, repeat determinism, malformed plans and arity, function/work/depth
  exhaustion, and 20-versus-200 repeated calls. Both call-count inputs contain
  exactly one required instance and exactly two MIR functions, proving that
  generic body count follows unique demand rather than call count.

#### 4C2 — nominal data and structured control — active

The nominal/control boundary is divided into three internal gates. Typed
semantic identity must be established before MIR represents a field or variant,
and immutable aggregate behavior must verify before mutation introduces places.

##### 4C2A — resolved nominal facts and immutable aggregate operations — complete

**Bounded implementation scope**

- Build a deterministic aggregate index from HIR definitions and retain typed
  expression facts for struct constructors, enum-variant constructors, and
  nominal field projections. Resolve symbols once to compact `HirItemId`,
  `FieldId`, and `VariantId` identities; MIR must not scan declarations or
  compare field/variant strings.
- Correct shadow inference for field types and enum-variant callable types.
  Substitute canonical generic nominal arguments into ordered field and payload
  types under the existing type-work/depth budget. Constructor arguments are
  positional in this gate; missing/extra arguments and unknown fields/variants
  are structured type failures.
- Materialize each concrete nominal type used by executable MIR as one ordered
  descriptor carrying its source definition, canonical `TypeId`, optional
  required `InstanceId`, canonical `TypeListId`, fields, and variants. Generic
  templates and optional-specialization records remain non-executable.
- Add immutable MIR construction for structs and enum variants alongside the
  existing tuple construction, plus immutable named struct-field projections.
  Evaluate constructor operands and projection bases exactly once in source
  order. The verifier checks descriptor identity, concrete field types, arity,
  and projection result types. Tuple/list index projections and enum payload
  bindings remain in 4C2B/4C2C respectively.
- Extend the bounded interpreter with structural nominal values so constructors
  and projections can be differentially executed without selecting heap
  placement, reference counting, or a backend layout.

**4C2A source distribution**

```text
crates/lpp-types/src/
  aggregates.rs            aggregate index and resolved expression facts
  check/signature.rs       canonical constructor/variant signatures
  check/body.rs            field and variant resolution
  check.rs                 facts in ShadowTypeOutput
crates/lpp-types/tests/
  aggregate_semantics.rs   resolution, diagnostics, generics, determinism
crates/lpp-mir/src/
  ids.rs                   compact nominal descriptor identities
  ir.rs                    concrete descriptors and immutable aggregate rvalues
  builder/aggregate.rs     descriptor, constructor, and projection lowering
  verify/types.rs          descriptor/constructor/projection typing
  interpret/               structural nominal execution
crates/lpp-mir/tests/
  nominal_aggregates.rs    structure, execution, limits, and growth gates
```

**Rules and diagnostics**

- `TypeKind::Nominal` definition and argument identities are authoritative.
  Qualified enum constructors resolve against that definition; normal field
  projections require a struct value. Ambiguous or unresolved forms fail before
  MIR.
- Aggregate type errors distinguish unknown field, unknown variant, invalid
  aggregate base, and constructor arity. Existing `ShadowTypeError` origins and
  global type-work/depth limits remain authoritative.
- MIR capacity failures remain `E4001`, unsupported later 4C2/4C3 surfaces
  remain `E4002`, inconsistent/missing typed aggregate facts use `E4006`, and
  malformed descriptor/rvalue typing remains verifier class `E4103`.
- New limits bound descriptors, descriptor fields/variants, and aggregate
  operands independently of the interner and existing MIR storage caps.

**Optimization evidence required**

- Definition, field, variant, and concrete nominal-type indexes are built once
  in deterministic order. Use-site resolution does not scan unrelated nominal
  declarations.
- One canonical descriptor is emitted per concrete nominal `TypeId`; repeated
  construction/projection does not duplicate schemas.
- Descriptor fields/variants and constructor operands use append-only compact
  stores. Growth tests compare 20 and 200 repeated operations and require
  descriptor count to remain constant while instruction/work counts scale
  linearly.

**4C2A exit gate**

1. Non-generic and generic struct constructors lower to exact concrete
   descriptors; nested field reads execute to exact values with no unresolved
   generic type in MIR.
2. Enum variants with zero, one, and multiple payloads construct with stable
   variant identity and exact payload width/type; templates and optional records
   remain non-executable.
3. Unknown fields/variants, wrong constructor arity, malformed descriptors, and
   descriptor/operand limits fail with structured diagnostics.
4. Repeated builds and snapshots are identical; repeated use does not duplicate
   descriptors; debug/release, workspace, compatibility, and local-CI gates
   remain green; production routing remains legacy-only.

**4C2A implementation evidence**

- `AggregateFacts` builds deterministic definition/member indexes once and
  attaches compact struct-constructor, enum-variant, and field-projection facts
  to typed expressions. Generic field and payload types are substituted from
  canonical nominal arguments under the shared type-work/depth budget.
- MIR lazily interns one `MirAggregateId` per concrete nominal `TypeId` and
  stores ordered `MirFieldId`/`MirVariantId` lists in append-only shared stores.
  Descriptors retain source definition, canonical argument-list identity, and
  required planner identity when present; optional-specialization identity is
  deliberately omitted.
- `ConstructStruct`, `ConstructVariant`, and `Field` consume resolved compact
  identities only. Constructor operands and projection bases lower once in
  source order. Verification checks concrete descriptor identity, kind,
  membership, reference closure, arity, operand/result types, uniqueness, and
  unresolved generic leakage before definite-initialization analysis runs.
- The bounded interpreter represents nominal values structurally, accounts for
  aggregate clone/construction work, executes immutable struct projections, and
  validates external nominal values recursively against their descriptors.
- Semantic and `mir-v3` fixtures expose aggregate facts and concrete descriptor
  tables. Dedicated tests cover generic and nested structs, all enum payload
  widths, malformed descriptors/rvalues, required/optional planner records,
  compact IDs, capacities, 20-versus-200 schema deduplication/growth,
  deterministic snapshots, bounded execution, and legacy-accepted return-value
  compatibility.

##### 4C2B — places, stores, destructuring, and list iteration — implementation complete

**Long-term representation contract**

- Add type-owned place facts for tuple/list projection, source assignments,
  augmented assignments, tuple destructuring, and list iteration. Struct-field
  identity remains owned by the 4C2A aggregate facts; no fact or MIR layer may
  resolve members by spelling after type checking.
- Normalize both legacy tuple `.N` syntax and existing `[N]` syntax to the same
  statically checked tuple projection. Tuple indices must be non-negative,
  compile-time constants in bounds. List indices remain dynamic `Int` values.
  String, slice, and map indexing stay deferred until their ABI/borrow contracts
  are available.
- Represent a place by compact `MirPlaceId`, a function-local root, a declared
  terminal `TypeId`, and a contiguous projection list. Projection kinds are a
  resolved `MirFieldId`, a compact tuple index, or one already-lowered list-index
  operand. Places contain no names, formatted types, backend offsets, or runtime
  pointers.
- Refactor MIR instructions into explicit `Assign` and `Store` kinds. `Assign`
  defines compiler temporaries and source bindings; `Store` mutates an existing
  typed place. Projection reads use `Rvalue::Load`, and structural list length
  uses `Rvalue::ListLen`. A store is never disguised as a value-producing
  rvalue.
- Direct source reassignment requires a mutable local. Nested field/list stores
  require a mutable root local, while projection stores rooted at parameters
  retain reviewed v1 behavior. Tuple elements are readable but not independently
  assignable. Initialization remains distinct from mutation for definite-
  initialization analysis.
- Evaluate assignment roots and dynamic indices before the right-hand side,
  exactly once each. An augmented assignment performs one place construction,
  one load, one right-hand-side evaluation, and one store. Tuple destructuring
  evaluates its source once and loads elements left-to-right.
- Lower `for item in list` to explicit condition/body/step/exit blocks. Evaluate
  the iterable once, retain one hidden list local and one hidden index local,
  read length at each condition, load the current element in the body, route
  `continue` to step, and route `break` to exit. `range` and other ABI-backed
  iterables are not silently treated as lists in this gate.

**Interpreter and alias contract**

- Keep the public structural `ExecutionValue` API, but execute lists and nominal
  structs through compact handles into a private bounded interpreter heap.
  Handle copies preserve aliases, so a field/list store through one alias is
  visible through another. Tuples remain value containers and may hold shared
  handles.
- Import arguments into the private heap and export return values structurally.
  Heap nodes, structural traversal, steps, and aggregate elements remain
  bounded. Existing logical aggregate-copy accounting remains observable even
  though a handle copy is constant-time. Equality/export traversals are cycle-
  aware without selecting production placement, ARC, or cycle-breaking policy.

**4C2B source distribution**

```text
crates/lpp-hir/src/lower/expression.rs
  legacy tuple .N normalization
crates/lpp-types/src/
  places.rs                  projection/assignment/destructuring/iteration facts
  check/body.rs              place, mutability, index, and loop typing
  infer/model.rs             structured place diagnostics
  snapshot.rs                place facts and metrics
crates/lpp-types/tests/
  aggregate_semantics.rs     tuple/list/place semantic gates
crates/lpp-mir/src/
  ids.rs                     MirPlaceId
  ir.rs                      places, projections, instruction kinds, loads/list len
  builder/expression.rs      fact-driven place materialization and projection loads
  builder/statement.rs       stores, destructuring, and list-loop CFG
  verify/{mod,types,definite}.rs
                              place/store/reference/type/init invariants
  interpret/                 bounded alias-preserving runtime heap
  snapshot.rs                mir-v4 place tables and instruction kinds
crates/lpp-mir/tests/
  places_mutation.rs         structure, malformed MIR, execution, limits, growth

tests/rewrite_phase4c2_compat.rs
  legacy-accepted mutation/destructuring return-value differentials
```

**Verification and capacity rules**

- Verify place/root/projection references, same-function local ownership,
  concrete type transitions, descriptor membership, tuple bounds, list index
  type, terminal type, store value type, and root mutability. Reject tuple
  stores, orphan/malformed places, and cross-function index operands.
- Definite initialization treats a load or store as a read of its root and index
  operands. Only assignments initialize targets; a projection store cannot
  initialize an absent root.
- Add independent limits for places, projection entries, interpreter heap nodes,
  and structural traversal. Existing block/local/instruction/operand, type-work,
  expression-depth, step, and aggregate-element limits remain authoritative.

**Optimization requirements**

- Resolve projection semantics once in the type stage and materialize compact
  paths without declaration scans or string comparisons.
- Store all paths contiguously. Reuse one place for augmented load/store and one
  iterable handle/index local per list loop. Interpreter handle copies are
  constant-time while logical work accounting remains bounded.
- Do not hoist list length across mutation, erase checked indexing, choose
  backend offsets, or introduce ownership-sensitive optimizations early.
- Growth tests compare 20 and 200 projections/stores/iterations: MIR growth must
  remain linear and 4C2A nominal descriptor count must remain constant.

**4C2B exit gate**

1. Nested struct-field and list-element stores preserve aliases and return exact
   values in the bounded interpreter.
2. Static tuple projection/destructuring, generic list indexing, list loops,
   nested loops, `break`, and `continue` execute with reviewed v1 behavior.
3. Negative/out-of-bounds indices fail deterministically; immutable roots,
   malformed places, projection/type errors, and capacities fail structurally.
4. Semantic and `mir-v4` snapshots are deterministic; debug/release workspace,
   strict rewrite Clippy, compatibility freeze, AOT/WASI, and local-CI gates are
   green. Production routing remains legacy-only.

**Implementation evidence**

- `PlaceFacts` records checked tuple/list projections, assignment and augmented
  assignment identity, tuple destructuring, and list iteration in expression-
  and statement-indexed compact tables. Tuple `.N` lowers to the same HIR index
  shape and checked fact as `[N]`; dynamic and out-of-bounds tuple indices are
  rejected before MIR.
- MIR stores compact `MirPlaceId` values with contiguous field, tuple, and
  already-evaluated list-index projections. `InstructionKind::Assign` is kept
  distinct from `InstructionKind::Store`; `Rvalue::Load` and `ListLen` are the
  sole place-read and structural list-length operations. Nested store bases are
  materialized before their right-hand sides so aliasing calls cannot retarget a
  pending write.
- Tuple destructuring evaluates once and loads left-to-right. List iteration has
  explicit condition/body/step/exit blocks, reloads length in the condition,
  and routes nested `continue`/`break` edges to the correct step/exit blocks.
- Verification checks references, function-local roots and index operands,
  projection type transitions, field membership, tuple bounds, list index and
  terminal/store types, mutable roots, parameter-root projection compatibility,
  tuple-store rejection, and definite initialization. Stores read but never
  initialize their roots.
- The interpreter imports public structural values into a private bounded heap.
  Four-byte handles preserve list and nominal aliases across local copies and
  calls; results export structurally. Heap-node, traversal, aggregate-element,
  step, and call-depth work remain independently bounded.
- Checked-in semantic-v2 and mir-v4 fixtures cover the new facts and paths.
  Dedicated tests cover exact aliasing/mutation results, nested target freezing,
  `.N`/`[N]`, destructuring, iterable/index single evaluation, nested loop
  control flow, malformed MIR, runtime bounds, independent capacities,
  20-versus-200 linear growth, and v1-accepted compatibility cases.
- Final validation passed 262 debug and 262 release workspace tests, strict
  warning-clean rewrite Clippy, workspace correctness/suspicious Clippy, all 10
  local-CI stages, 113 frozen v1.2 fixtures, AOT parity 44/44, and Node-WASI
  34/34. The driver-routing gate still proves production uses `LegacyEngine`.

##### 4C2C — enum dispatch, `match`, and `try` — complete

**Compatibility findings**

- Existing source uses statement-form `match value:` with indented
  `Variant(bindings):` arms. Bare variants, one-level enum qualification, `_`,
  multiple parsed bindings, duplicate variants, wildcard-before-later-arms, and
  non-exhaustive matches are accepted. First matching arm wins; an unmatched
  statement match falls through without an effect.
- The legacy MIR compares tags linearly and binds only the first payload even
  though the grammar accepts several. The rewrite preserves accepted syntax and
  first-match/fallthrough behavior, but treats the single-binding lowering as a
  defect: every declared payload is resolved, typed, and bound left-to-right.
- Postfix `value?` evaluates `value` once, treats declaration-order variant zero
  as success, unwraps its sole payload, and propagates any other variant. The
  legacy checker guesses `Int` and its scalar-enum fallback scans declarations;
  those unsafe guesses are not copied. Valid nominal programs retain their
  behavior and malformed uses receive structured type errors.
- The architecture guide's `=>` arm example is aspirational, not accepted by
  the current lexer. `:` remains canonical in this slice. A future additive
  `=>` spelling must normalize to the same HIR and ship through the production
  cutover rather than becoming a rewrite-only public dialect.

**Long-term representation contract**

- Add compact type-owned `EnumFlowFacts` indexed by statements, expressions,
  and match arms. A match fact retains its resolved enum item and subject type,
  source-order arm range, coverage, and wildcard identity. Each arm retains a
  resolved `VariantId` or wildcard marker. A try fact retains the input enum,
  declaration-order success variant, success payload type, enclosing return
  enum, and whether residual propagation can return the original handle or must
  rebuild another concrete instance.
- Resolve bare arm names only against the subject enum. Qualified names must
  name that same enum. MIR consumes only `HirItemId`, `VariantId`, `FieldId`,
  `TypeId`, and their concrete MIR identities; no variant spelling, declaration
  scan, tag guess, or generic-name formatting survives type checking.
- Match payload arity is exact. Unit variants accept no bindings; payload
  variants bind every field left-to-right in the arm scope. Duplicate and
  unreachable arms remain accepted for compatibility and are represented in
  facts, while dense dispatch preserves first-arm precedence. Coverage is
  recorded even though non-exhaustive statement matches retain legacy
  fallthrough behavior.
- `?` requires a nominal enum whose first declared variant has exactly one
  payload. The expression type is that payload type. Every residual variant is
  propagated unchanged when the input and function return types are identical.
  If the return type is another concrete instance of the same enum definition,
  the residual variant is rebuilt only when all corresponding residual payload
  types agree. Other carriers, definitions, or payloads fail before MIR.

**MIR and interpreter contract**

- Add `Terminator::SwitchEnum` with one evaluated subject, a concrete
  `MirAggregateId`, and a dense contiguous target table in descriptor variant
  order. Repeated entries share wildcard/fallthrough blocks; arm bodies are not
  duplicated. `MirVariant` records and verifies its declaration ordinal so the
  bounded interpreter selects a target in constant time without scanning.
- Add checked `PlaceProjection::Downcast(MirVariantId)`. Payload reads use a
  subject root followed by one downcast and one resolved field projection.
  Downcast preserves the nominal carrier while refining legal variant fields.
  Stores through a downcast remain rejected until ownership and enum-mutation
  semantics are designed.
- Match subjects and try operands are materialized exactly once. Match arms
  route fallthrough to one join, and terminating `return`/`break`/`continue`
  arms retain their terminators. Uncovered variants target the join. `?` routes
  success to a payload result temporary and each residual to an early return;
  same-type residuals reuse the original handle, while cross-instance residuals
  reconstruct the resolved target variant and preserve nested aliases.
- Verification checks switch subject/descriptor types, enum kind, exact dense
  target width, function-local targets, ordinals, downcast membership, payload
  field membership and type, prohibited stores, and definite initialization.
  Runtime dispatch and downcasts remain checked and origin-carrying.

**Optimization and capacity requirements**

- Dense dispatch replaces the legacy linear comparison chain. Each source arm
  is lowered once, wildcard/fallthrough targets are shared, only selected-arm
  payloads are loaded, and same-carrier residual propagation allocates nothing.
- Use a separate append-only switch-target store and independent target limit.
  Final MIR has no per-switch vector or name. Growth tests vary variants,
  matches, payloads, and concrete generic instances from 20 to 200 and require
  linear storage/code growth with constant descriptor reuse.
- Bump observable contracts to deterministic `semantic-v3` and `mir-v5`.
  Snapshot and execution tests prove one switch rather than synthetic tag
  comparisons, exact-once evaluation, first-match precedence, payload order,
  nested control flow, generic residual rebuilding, malformed MIR failures, and
  bounded execution.

**Source distribution**

```text
crates/lpp-types/src/
  enum_flow.rs              compact match/arm/try facts
  check/body.rs             enum resolution, payload typing, coverage, try rules
  aggregates.rs, check/call.rs
                            unit-variant value/call compatibility
  check.rs, infer/model.rs  stage integration and structured diagnostics
  snapshot.rs              semantic-v3 facts and metrics
crates/lpp-mir/src/
  ir.rs                     dense switches, ordinals, downcasts, target storage
  builder/{aggregate,function,statement,expression}.rs
                            descriptors, match/try CFG, payload materialization
  verify/{mod,types,definite}.rs
                            enum/reference/type/CFG/init invariants
  interpret/eval.rs         checked constant-time dispatch and payload loads
  snapshot.rs               mir-v5
crates/lpp-{types,mir}/tests/
  enum-flow semantics, execution, malformed MIR, limits, growth, snapshots

tests/rewrite_phase4c2_compat.rs
  accepted syntax/result and diagnostic-class differentials
```

**Implementation evidence**

- `EnumFlowFacts` now stores normalized nominal subject/carrier/result types,
  source arm ranges, coverage, wildcard reachability, resolved variant/field/
  binding ranges, success extraction, residual mappings, and direct-return mode.
  Structured errors reject invalid subjects, paths, payload arity, carriers,
  success shapes, enclosing return types, and incompatible residual payloads.
- MIR now uses descriptor-order `SwitchEnum` tables from an independently
  bounded append-only target store. Enum descriptors carry verified ordinals;
  payload reads use `Downcast` then resolved `Field`; downcast stores are
  rejected. Bare unit-variant values and the established zero-argument call
  spelling both lower to the same constructor MIR.
- Match and try operands are materialized once. Every source arm body is emitted
  once, while only first-match targets are reachable and wildcard/uncovered
  entries share blocks. Direct residual propagation returns the original
  alias-bearing runtime handle without allocation; cross-instance propagation
  rebuilds every residual variant with alias-preserving payload handles.
- Reference, aggregate, CFG, type, store, and definite-initialization
  verification understand enum switches and downcasts. The interpreter uses a
  checked ordinal lookup followed by one dense target index, with no tag scan.
- Checked-in `semantic-v3` and `mir-v5` fixtures include enum-flow facts,
  switches, target tables, ordinals, and downcast paths. Focused tests cover
  exact execution, compatibility, malformed ordinals/tables/references/stores,
  independent capacity failure, and separate 20-versus-200 dispatch, payload,
  and generic-enum-instance growth.

**Exit gate**

1. Unit, single-payload, multi-payload, qualified, bare, wildcard,
   non-exhaustive, duplicate, nested, and generic matches preserve reviewed
   source behavior and execute exact results.
2. Success, same-carrier residual, cross-instance residual, nested, and
   side-effecting `?` cases execute exactly once and preserve aliases.
3. Every invalid subject/path/arity/carrier/residual and malformed
   descriptor/switch/downcast/field/target/init case fails structurally.
4. Semantic-v3 and mir-v5 snapshots, 20-versus-200 growth, debug/release,
   strict rewrite Clippy, frozen compatibility, AOT/WASI, and local CI pass.
   Production remains routed through `LegacyEngine`.

##### 4C3 — ABI and callable concurrency surfaces — approved contract

Source-backed string/char values, generated ABI-registry builtin resolution,
closures, async/await, tasks, and spawn represented without choosing ownership
placement. The feature-completion boundary is split into three ordered internal
gates so that string materialization does not wait on closure representation and
the legacy string-dispatch builtin table is not copied into the rewrite.

#### 4C3A — source-backed strings and ABI-registry builtins

**Bounded implementation scope**

- `lpp-runtime-abi` generation gains semantic signature fields: the generated
  `BuiltinAbi` carries `semantic_parameters`/`semantic_result` of a new
  `SemanticAbiType` (`any`, `bool`, `f64`, `i32`, `i64`, `str`, `str_slice`,
  `vector_i64x2`, `void`) alongside the existing machine-lowering types. All
  checked-in generated outputs are regenerated through `lpp-abi-gen`; the drift
  test keeps them exact.
- `lpp-types` depends one-way on `lpp-runtime-abi`. A `BuiltinIndex` built once
  from `lpp_runtime_abi::generated::BUILTINS` maps builtin source spellings to a
  four-byte `BuiltinId` (an index into the generated table). A call whose callee
  is an `Unresolved` name with a builtin spelling is resolved: the name receives
  the registry function type (`any` becomes a fresh per-occurrence inference
  variable; every other semantic type maps to its primitive), and the call
  expression records a `BuiltinFact` in a new `BuiltinFacts` table on
  `ShadowTypeOutput`. Unknown spellings remain unresolved exactly as before.
- `build_mir` gains a `&SourceMap` parameter. String and character literals are
  decoded from the source text (escaping `\n`, `\r`, `\t`, `\0`, `\"`, `\'`,
  `\\`, `\xNN`; quoted and triple-quoted forms; exactly one character for
  character literals) into owned interned MIR constants: `Constant::String`
  carries a `MirStringId` into a program-owned string arena plus the source span;
  `Constant::Character` carries the decoded `char` plus the span. Repeated
  identical spellings share one string. Decoding failures (unrepresentable
  scalar) are structured builder diagnostics, never silent fallbacks.
- Formatted (f-string) literals remain outside this boundary: the rewrite HIR
  retains them as single literals and does not expose interpolation parts, so
  the builder rejects them with `UnsupportedConstruct::FormattedString`.
- Add `Rvalue::Builtin { builtin: BuiltinId, arguments: ListRange<Operand> }`.
  The verifier checks registry presence, exact arity, and that every argument
  type matches its semantic parameter kind (`any` accepts any concrete type).
- The interpreter gains `ExecutionValue::String` and `ExecutionValue::Char` and
  executes the deterministic pure builtin subset: integer helpers
  (`abs`, `min`, `max`, unsigned comparisons/shifts/division, `popcount64`,
  `clz64`, `ctz64`, byte swaps, rotations, truncations, checked and wrapping
  arithmetic), float math (`floor`, `ceil`, `pow`, `sqrt`, `fmod`), string
  operations (`str_concat`, `str_len`, `str_contains`, `str_starts_with`,
  `str_ends_with`, `str_find`, `str_replace`, `str_trim`, `str_to_lower`,
  `str_to_upper`), conversions (`int_to_str`, `str_to_int`, `float_to_str`,
  `bool_to_str`, `u64_to_str`, `u64_to_hex`, `str_to_u64`), and structural list
  operations (`list_new`, `list_push`, `list_get`, `list_set`, `list_len`).
  Printing builtins (`print`, `print_str`, `print_int`, `print_float`,
  `print_bool`, `write_str`, `eprint_str`) execute with their text appended to
  a new deterministic `ExecutionOutcome.output` line buffer. Builtins whose
  behavior requires OS state (`sleep`, `time_ms`, `random`, file/env/exit,
  threads, atomics, maps, slices, task-runtime internals) fail at execution
  with structured `UnsupportedBuiltin` diagnostics; no legacy string-dispatch
  table is consulted.
- `Str + Str` lowers to string concatenation and `==`/`!=` compare string and
  character values; other binary operators on strings remain type-stage errors.

**Rules and diagnostics**

- Builtin resolution happens only at the type stage; MIR consumes
  `BuiltinFacts` and never resolves names by spelling.
- Missing/inconsistent builtin facts and malformed builtin rvalue typing use
  builder class `E4006`; `UnsupportedConstruct::FormattedString` uses `E4002`.
- The interpreter reports `UnsupportedBuiltin` as `E4303`; string/char values
  participate in equality, heap containment, and aggregate element accounting.

**4C3A exit gate**

1. String/char constants with escapes, concatenation, equality, and the
   deterministic builtin subset execute to exact values; printing is captured
   in output order; f-strings and OS-effect builtins fail structurally.
2. `print_str` is typed `fn(Str) -> Void`, `str_len` `fn(Str) -> Int`, and
   `any`-parameter builtins unify per argument; unknown names remain unresolved
   and fail in MIR exactly as before.
3. Builtin calls, string constants, and captured output scale linearly from 20
   to 200 repeated operations; repeated identical literals share one string.
4. Semantic-v4 and mir-v6 snapshots are deterministic; all prior gates remain
   green; production routing remains legacy-only.

#### 4C3B — closures and shared capture cells

**Bounded implementation scope**

- A HIR closure expression lowers to a distinct `MirFunction` with
  `MirFunctionKind::Closure` and a `captures: ListRange<MirLocalId>` naming the
  enclosing function's locals captured in HIR declaration order. A local is a
  capture when it is referenced from within the closure body (including nested
  closure bodies, which reference the enclosing frame's locals) and its
  declaring scope is not inside the closure body's scope. The closure function's
  frame begins with one `MirLocalKind::Capture` local per capture followed by
  the closure parameters; its public `ty` remains the closure expression's
  user-facing function type.
- Add `Rvalue::MakeClosure { function: MirFunctionId, captures: ListRange<Operand> }`
  which evaluates each capture operand exactly once in source order.
- The interpreter represents closures as `RuntimeValue::Closure(HeapId)` with
  `HeapNode::Closure { function, captures: Vec<HeapId> }`; every capture is
  wrapped in `HeapNode::Cell(RuntimeValue)` (a cell whose value is the current
  capture). Frame capture locals hold `RuntimeValue::Cell(HeapId)`; reading a
  cell dereferences, storing through a cell rebinds the cell. `MakeClosure`
  dereferences a cell-valued capture and copies the current value into a fresh
  cell, reproducing v1 value-at-creation environment semantics: repeated calls
  share cells (stateful mutation persists across calls), aggregate captures
  keep handle aliasing, and the enclosing scope is not updated by closure
  mutation. A closure call applies cell captures plus user arguments.
- Closures with default parameters remain rejected with a structured
  `UnsupportedConstruct::ParameterDefault`.
- The type stage enforces the v1 spawn rule: assigning to a captured local
  inside a closure passed directly to `spawn` is a structured error.

**4C3B exit gate**

1. The stateful counter closure (v1 `test_mutable_closure.lpp`) executes 1, 2,
   3; aggregate capture mutation is visible through both the closure and the
   enclosing scope; outer scalar state is not updated by closure mutation; a
   closure stored in a list and retrieved executes (v1 `list_closures.lpp`).
2. Nested closures capture the enclosing closure frame's locals; capture
   counts, cells, and execution remain deterministic and bounded.
3. Default-parameter closures, closure arity/type mismatches, and malformed
   make-closure rvalues fail structurally.
4. Mir-v6/semantic-v4 snapshots, 20-versus-200 closure growth, debug/release,
   strict Clippy, frozen compatibility, and production routing remain green.

#### 4C3C — async functions, tasks, await, and spawn

**Bounded implementation scope**

- `Function.is_async` lowers to `MirFunctionKind::Async`; the public item type
  is already `fn(params) -> Task[Return]` from Phase 3C. A direct call to an
  async function materializes `HeapNode::Task { function, arguments, result }`
  without executing the body.
- Add `Rvalue::Await(Operand)`: the operand type is `Task[T]` and the result
  type is `T`. The interpreter executes a pending task to completion on the
  interpreter thread (depth-first, step/depth bounded) and stores its result; a
  second await returns the stored result, matching v1 idempotent double-await.
- Add `Rvalue::Spawn(Operand)`: the operand is a closure value. The task is
  created detached and the interpreter executes it to completion immediately at
  the spawn point: v1 schedules spawns on OS threads with unobservable output
  ordering, so immediate deterministic execution preserves every observable v1
  guarantee without nondeterminism. The result is discarded; `spawn` is `Void`.
- `execute_mir` auto-drains a single top-level task result (v1 wraps an
  `async def main` in an executor), returning the task's result value.
- Verification: await operands/results and spawn operand typing; async bodies
  verify like any function; task/closure heap nodes count against the existing
  heap-node limit.

**4C3C exit gate**

1. `second().await` chains execute to exact values (v1 `async_await_chain.lpp`
   shape); double await is idempotent; `async def main` auto-drains; spawn runs
   its closure exactly once and returns Void.
2. Awaiting an already-awaited task, spawning a non-closure, and awaiting a
   non-task fail structurally.
3. Repeated await of one task allocates no additional heap nodes; task and
   closure counts scale linearly with distinct tasks/closures, not with await
   count.
4. All 4C3A/4C3B evidence, workspace, strict Clippy, frozen compatibility,
   AOT/WASI, and local-CI gates remain green; production remains on
   `LegacyEngine` with rewrite MIR shadow-only.

### 4D — ownership graph and placement — approved contract

A read-only ownership plan over validated MIR: per-cell
Frame/Owned/Shared placement, the type containment graph with cycle
breaking, per-function frame arenas, and a plan proof. 4D never mutates
MIR and never imports a backend, runtime, legacy AST, or legacy MIR; it
consumes `MirProgram` plus the `TypeInterner` for type-kind
introspection. The compact MIR layout budget is untouched because no
MIR field gains width.

**Bounded implementation scope**

- New crate `lpp-ownership` (workspace and default member) with one-way
  dependencies on `lpp-hir` (diagnostic origins), `lpp-mir`,
  `lpp-passes` (the pass adapter), and `lpp-types`.
- `compute_ownership_plan(program, types) -> Result<OwnershipPlan,
  OwnershipError>`:
  - **Cell placement.** Every `(function, local)` cell receives exactly
    one placement. A cell is `Frame` unless it is a `Capture`-kind local
    (its cell lives inside a closure value, per the 4C3B capture-cell
    semantics) or it is used at an escape site. Escape sites are the
    operand of `Terminator::Return`, the operand of `Rvalue::Spawn`,
    the operand of `Rvalue::Await`, every capture operand of
    `Rvalue::MakeClosure`, every argument of `Rvalue::Call`, and
    every argument of `Rvalue::Builtin` (per-builtin retention is not
    modeled in v1, so every builtin value argument escapes
    conservatively).
    Escape marking is a single monotone pass: marking never propagates
    through values, because every transfer in v1 MIR is a structural
    copy and the source cell outlives or underlies its copies. A heap
    cell's final placement is `Shared` when the node of its own type is
    a cycle member (a function type expands to every callable with
    that type), otherwise `Owned`; containment is not transitive for
    cells — an acyclic container of a `Shared` value releases the
    shared child and stays `Owned`.
  - **Containment graph.** Nodes: one per `Closure`/`Async`
    `MirFunction` (a value of that callable's type), one per
    `MirAggregate` (a value of that nominal instance), one per
    `List(E)`/`Slice(E)`/`Task(E)`/`Tuple(elements)`/`Map{K, V}` type
    encountered in any function, local, parameter, return, field, or
    capture position. Edges: a callable node contains its capture
    payload types; an aggregate node contains every field type of
    every variant; list/slice/task nodes contain their element type;
    tuple and map nodes contain their element/key/value types. A
    `Function` type-id expands to every callable whose `ty` equals it
    (sound over-approximation). Primitive, string, and character types
    are leaves.
  - **Cycle breaking.** Tarjan SCC over the containment graph; an SCC
    of size greater than one or a self-edge is a cycle. Every cycle
    member is strategy `Shared` (ARC-managed from 4E onward); every
    other node is `Owned` (recursive deallocation is sound because the
    remaining graph is acyclic). Containers of `Shared` types that are
    not themselves cycle members remain `Owned`.
  - **Arena planning.** One frame arena per function that has at
    least one `Frame` cell; each arena lists its `Frame` cells in
    local declaration order. `Owned`/`Shared` cells are never arena
    members.
- `verify_ownership_plan(program, types, plan) -> Vec<OwnershipPlanError>`
  recomputes the escape analysis, containment graph, SCC partition, and
  arena partition independently and proves the plan: (V1) cell
  completeness and uniqueness; (V2) `Frame` exactly when the cell is
  non-escaping and non-capture, and the heap class matches the type
  strategy; (V3) node, edge, and strategy correctness (`Shared`
  exactly for cycle members); (V4) arena bijection with `Frame`
  cells and stable order; (V5) determinism: the plan's snapshot equals
  a fresh recomputation's snapshot.
- `ownership_plan_snapshot(plan) -> String` emits stable IDs in fixed
  order — cells by function id then local order; nodes as callable
  functions by id, aggregates by id, then `List`/`Slice`/`Task` nodes
  by element type id, `Tuple` nodes by element-list id, and `Map`
  nodes by (key, value) ids — never hash-map iteration.
- `lpp-passes` gains `MirPass::established()` (default `&[]`); the
  manager unions `pass.established()` into the outcome after post-pass
  revalidation. Existing passes are unaffected by the default.
  `OwnershipPlanPass` (name `"ownership-plan"`) requires the core
  invariants, preserves the core invariants plus `Ownership` and
  `NoOwningCycles`, and establishes `Ownership` and
  `NoOwningCycles` by computing and proving the plan; a proof failure
  fails the pass structurally.

**Rules and diagnostics**

- Construction failures (`OwnershipError`, code `E4401`) are structural
  guards against unvalidated MIR only: a nominal type with no matching
  aggregate instance (`MissingAggregateInstance`) or a missing local
  (`MissingLocal`). Validated MIR never triggers them.
- Plan proof failures use `OwnershipPlanError` (code `E4402`) with the
  offending entity, block, and origin: missing/duplicate/unknown cells,
  invalid cell placement, missing/duplicate nodes, invalid edges,
  invalid strategy, and missing/extra/misordered arenas.
- 4D performs no allocation, no MIR mutation, and no execution; the
  plan is the 4E input for explicit retain/release lowering.

**4D exit gate**

1. Frame/Owned/Shared are decided per source program: non-escaping
   locals are `Frame`; returned, spawned, awaited, captured, and
   call-argument locals are heap; `Capture`-kind locals are heap with
   their type's strategy.
2. A self-referential list of capturing closures and a
   mutually-capturing closure pair become `Shared` with exactly the
   cycle members marked; acyclic containers of `Shared` types remain
   `Owned`.
3. Every tampered plan (flipped placement or strategy, dropped or
   extra arena, reordered cells) fails the proof with `E4402`; every
   computed plan passes with zero errors.
4. The pass manager establishes `Ownership` and `NoOwningCycles` after
   `ownership-plan`, drops them through a later pass that does not
   preserve them, and the previously rejected ownership precondition
   becomes satisfiable.
5. Snapshots are byte-identical across recomputations; 20- and
   200-function programs scale linearly per function in cells, nodes,
   arenas, and cycle counts.
6. The plan and proof run over every Phase 4C3 exit-gate program with
   zero errors; MIR is unchanged by 4D; all prior gates, workspace
   tests, strict warnings, frozen compatibility, AOT/WASI, and
   local-CI gates remain green; production remains on `LegacyEngine`
   with rewrite MIR shadow-only.

### 4E — ARC and move-out — approved contract

Deterministic reference-counted ownership for the MIR oracle's heap
values. The 4D ownership plan supplies the pinned set; every transfer is
classified by a single consume-vs-borrow call contract; every
retain/release/free is an explicit, accounted operation; and ownership
balance is proven twice — statically over the MIR (a token dataflow) and
at end of execution (the runtime proof). 4E never mutates MIR: the MIR
contract, snapshots, and the legacy execution entries are untouched; ARC
is a property of a new execution entry plus the static proof. Production
stays on `LegacyEngine`; rewrite MIR remains shadow-only.

**Bounded implementation scope**

- `lpp-mir` interpreter gains one entry:
  `execute_mir_arc(program, types, entry, arguments, limits, pinned:
  &[TypeId]) -> Result<ExecutionOutcome, InterpreterError>`. The legacy
  entries (`execute_mir`, `execute_mir_with_stats`) keep their exact
  behavior — no refcounts, no balance proof — so every prior gate stays
  green.
  - A refcount table runs in lockstep with the heap: allocation sets
    the count to one; the count table is indexed by `HeapId`.
  - **The consume-vs-borrow call contract** — one transfer table, the
    single source of truth for the runtime and the static checker:
    - **Move (consume).** The reference is transferred, the source
      slot is consumed (marked moved-out; no release at its death), and
      no retain is emitted: every argument operand of `Rvalue::Call`
      (function and closure calls alike), the operand of
      `Terminator::Return`, and the value-argument operands of
      `Rvalue::Builtin` per builtin — `list_push` value, `list_set`
      value (the replaced element is released), constructor builtins
      (tuple/nominal field arguments), and the print builtins (the
      value is observed, never retained).
    - **Borrow (retain).** The receiver gains one reference and the
      source keeps its own: the result of `list_get` (the list keeps
      its element; the caller gains one), the result of
      `Rvalue::Await` (the task keeps its result; the awaiter gains
      one), every capture operand of `Rvalue::MakeClosure` (the
      closure node retains each capture; the source slot stays alive —
      the 4C3B cell semantics), the operand of `Rvalue::Spawn` (the
      task node retains the closure back-reference), an `Assign` whose
      value is `Use(Copy(local))` (the target gains one; the source
      keeps its one), and nominal field-projection reads (a fresh
      reference per read).
    - **Receiver (no count change).** The list operands of
      `list_push`/`list_set`/`list_get`/`list_len` (the container keeps
      its own reference for its whole life) and a call's callee
      operand (invocation is an indirection, not a transfer — the
      callee is never consumed by the call). A callee read still
      requires the callee to be alive: invoking a moved closure is a
      use-after-move (`E4403` statically; `UninitializedLocal` at
      runtime).
  - **Cell death.** Reassignment (an `Assign` to an initialized slot,
    or a `Store` with empty projection) releases the old value first.
    Function exit releases every slot that still holds a heap value and
    has not been moved; a moved slot is a no-op. Capture writeback
    (4C3B) releases the closure slot's old value and moves the frame
    slot's reference into it; if the body moved the capture out, the
    frame slot is empty and the closure keeps its own reference
    (the cell persists).
  - **Move-out optimization.** A pure move emits zero ARC traffic (no
    retain, no release); a frame cell of a primitive type emits no
    release at all.
  - **Free at count zero.** Recursive, depth-first, in
    element/field order: lists release their elements, nominals their
    fields, closures their captures, tasks their arguments then result
    then closure back-reference, strings drop their data. Freeing a
    dead node is a double free; a release below zero is an underflow.
  - **Pinned set.** A heap node whose own type's 4D node is `Shared`
    (a cycle member) is pinned: its count never decrements below one,
    it is never freed, and it is reported instead of leaked. Value
    cycles exist only over `Shared` type nodes (a value cycle implies a
    type cycle), so pinning is complete.
  - **End-of-execution balance proof** (successful completion only; a
    failed execution is abandoned and proves nothing): the public
    `ExecutionValue` outcome is materialized as a deep copy, the entry
    result reference is released, and every non-pinned heap node must
    be dead. A survivor reports `OwnershipLeak { nodes }`. Entry
    arguments materialize fresh nodes into the entry's parameter
    slots; the program owns and releases them on completion, so no
    reference crosses the execution boundary.
  - `ExecutionOutcome` gains `arc: Option<ArcStats>` with
    `ArcStats { retains, releases, frees, pinned_live }` — `None` on
    the legacy entries. The `heap_nodes` stat and the
    `max_heap_nodes` limit keep their allocation-count meaning (the
    limit counts allocations, not live nodes).
  - New `InterpreterErrorKind` arms `OwnershipUnderflow` and
    `OwnershipLeak { nodes: usize }`, code `E4304`.
- `lpp-ownership` gains:
  - `OwnershipPlan::pinned_types() -> Vec<TypeId>` — the ascending
    type ids of the `Shared` nodes; the runtime's pinned input.
  - `verify_ownership_balance(program, types, plan) ->
    Vec<OwnershipBalanceError>` (code `E4403`): a deterministic
    worklist dataflow over each function's CFG tracking one token
    state per heap-value cell (heap-typed per the type interner) —
    `Alive` (holds the reference), `Dead` (moved out), `MayDead`
    (path-dependent). Transfer rules are exactly the call contract
    above. It proves: (B1) consuming or borrowing a `Dead` or
    `MayDead` cell is `UseAfterMove { function, block, local }` — a
    consume requires `Alive`; (B2) every heap-value local of every
    function is tracked (`MissingCell` is a structural guard against
    unvalidated MIR only); (B3) the result is a fixed point computed
    in block-ascending order with a bounded iteration count, so two
    runs agree byte for byte. Loops: a consume in the body kills the
    cell on the back edge, so a second-iteration consume or borrow is
    `UseAfterMove`; a cell redefined in the body stays `Alive` at the
    join (no false positive). `MayDead` at function exit is legal: the
    runtime releases conditionally on slot liveness.
  - `OwnershipBalancePass` (name `"ownership-balance"`): requires the
    core invariants plus `Ownership`, preserves the core invariants
    plus `Ownership` and `NoOwningCycles`, and establishes
    `MirInvariant::OwnershipBalance` (a new `lpp-mir` enum variant;
    the one-way dependency rule is untouched).
  - The 4D API (`compute_ownership_plan`, `verify_ownership_plan`,
    snapshots, `OwnershipPlanPass`) is unchanged.

**Rules and diagnostics**

- E4403 (`OwnershipBalanceError`): `UseAfterMove { function, block,
  local }` with the offending entity, block, and origin; `MissingCell`
  for unvalidated MIR only.
- E4304 (`InterpreterError`): `OwnershipUnderflow` (a release or free
  with no live reference) and `OwnershipLeak { nodes }` (surviving
  non-pinned nodes at end of execution).
- 4E mutates no MIR, adds no instructions, bumps no snapshot version,
  and leaves the legacy execution entries bit-for-bit in behavior.
- The runtime table and the static table are the same contract; if
  they ever disagree, the static checker (compile time) reports first
  and the runtime proof (execution time) is the backstop.

**4E exit gate**

1. Hand-computed ARC traffic on reference programs: exact retain,
   release, and free counts covering a `list_get` retain, a
   reassignment release, a capture retain plus writeback release, and a
   moved slot that emits no release at death.
2. The consume-vs-borrow table holds end to end: a value pushed into a
   list is freed exactly when the list dies (no leak, no double free);
   a list receiver is reused across many pushes without a double
   release; a moved callee is flagged (the call reads the callee);
   repeated `await` retains per await; `list_set` releases the
   replaced element.
3. Move-out: a pure move chain emits zero ARC traffic until the final
   free; a returned heap local frees exactly once; entry arguments
   materialize and release inside the execution (no cross-boundary
   reference).
4. Static balance: a use-after-move (a read after a consume, and a
   second consume) fails with `E4403` at the exact function, block, and
   local; a move into a loop body is flagged when the second iteration
   re-consumes; a loop that redefines the cell each iteration passes;
   every 4C3 and 4D exit-gate program passes with zero errors.
5. Runtime proof: the self-referential and mutually capturing
   programs run with zero leaks — exactly their cycle members pinned
   (`pinned_live` matches the live plan `Shared` instances); the same
   programs with an empty pinned set report `E4304` leaking exactly
   the cycle members; a program that leaves an `Owned` node alive
   reports `E4304`.
6. Pass wiring: the manager establishes `OwnershipBalance` after
   `ownership-balance` (which requires `ownership-plan` first), and a
   later pass that does not preserve it drops it.
7. Every existing workspace test stays green (the legacy entries are
   untouched); the ARC entry runs every 4C3 exit-gate program with the
   plan's pinned set at zero errors and zero leaks with deterministic
   `ArcStats`; strict warnings, frozen compatibility, AOT/WASI, and
   local-CI gates remain green; production remains on `LegacyEngine`
   with rewrite MIR shadow-only.

**4E implementation evidence (contract refinements)**

Implementation against the contract surfaced four boundary details,
each covered by a test:

- The builder lowers every user declaration `x := <expr>` to a
  copy-temp plus a `Use(Copy(temp))` into the user local; each
  heap-valued declaration therefore contributes exactly one retain
  and one extra release at function exit. The hand-computed counts in
  the gate are derived from the lowered MIR on that basis.
- String literals are interned per program: the cache holds one
  reference for the run, a read in a read-only position (a builtin
  receiver/index) releases its reader reference after the read, and a
  read stored in a slot (an assignment) keeps its reference until the
  slot dies. `print`/`write_str` consume their value: the reference
  dies with it.
- A task node pins only through its contents: it is pinned when the
  spawned function's result type or the spawned closure's function
  type is a 4D cycle member, and is otherwise ephemeral. A detached
  task node must die with its last owner; its closure back-reference
  and stored captures are released when it frees, so a fire-and-forget
  `spawn` leaks nothing. Nodes with no recorded type remain
  conservatively pinned (reported, never leaked).
- The `max_aggregate_elements` reservation walks a value's reachable
  structure counting each heap node at most once, so reading a
  self-referential local (a list holding a closure that captures the
  list) is bounded instead of unbounded.
- The static dataflow treats an unreached block's in-state as bottom:
  the first reach installs the incoming state, later reaches meet it
  in. (Meeting the first reach against an all-dead placeholder would
  wrongly downgrade `Alive` to `MayDead` on single-path programs.)

### 4F — optimizer and safety exit — approved contract

The Phase 4 exit gate: proof that the MIR stage is a safe
optimization substrate. A small set of ownership-aware scalar and
CFG passes rewrites verified MIR **in place** — no new entities, no
ID churn, no arena mutation — and every rewrite is covered by
per-pass revalidation, unoptimized-versus-optimized equivalence
under both execution entries, ownership-sensitive regressions,
compile-fail gates, and sanitizer-profile evidence. The artifact
handed to the Phase 5 backend contract is the **optimized and
revalidated MIR**: the same program shape (same IDs, same block and
instruction order) with folded constants, propagated single-def
constants, and folded branches. Production stays on `LegacyEngine`;
rewrite MIR remains shadow-only.

**Bounded implementation scope**

- `lpp-passes` gains three `MirPass` implementations (a new
  `passes` module) plus level-driven entry points:
  1. `const-fold` (scalar)
  2. `const-prop` (scalar)
  3. `branch-fold` (CFG)
  - `optimization_passes() -> PassManager`: one canonical sweep of
    the three passes in that order.
  - `run_optimization(program, types, level)`: sweeps the canonical
    pipeline up to `OptimizationBudget::fixed_point_iterations`
    times (O0: 1, O1: 2, Oz: 2, Os: 3, O2: 4, O3: 6 — the budget
    table is the source of truth), stopping early as soon as a
    sweep changes nothing.
  - No new `lpp-mir` API, no new error codes (the manager's
    `E4201`–`E4204` and the existing verifier codes are reused),
    no `lpp-common` ABI changes. `OptimizationOptions::verify_each_pass`
    is inert in the rewrite: the manager revalidates after every
    pass unconditionally (the 4A rule).

**const-fold** — rewrites an instruction's `Rvalue` in place when
every operand of the rvalue is `Operand::Constant` and the oracle
semantics for that operator and type pair are total. The result is
always a primitive constant and the rewrite is
`Binary/Unary{...} -> Use(Constant(result))`; the target local,
its type, and every other instruction are untouched. Bit-for-bit
against the interpreter:

- Int: `+ - *` fold with `wrapping_add/sub/mul` (unconditional);
  `/ %` fold with `wrapping_div/rem` only when the right operand is
  nonzero (zero keeps the runtime `DivisionByZero`); bitwise and
  shift operators fold with the wrapping semantics; the six
  comparisons fold to `Bool`. `LogicalAnd`/`LogicalOr` on ints are
  never folded (the runtime rejects them).
- Float: `+ - * / %` fold with the same IEEE-754 `f64` operations
  the oracle applies (unconditional — inf/NaN agree
  bit-for-bit); the six comparisons fold to `Bool`.
- Bool: `LogicalAnd`/`LogicalOr`/`Equal`/`NotEqual`. Char: the six
  comparisons. String: `Equal`/`NotEqual` only (the result is
  `Bool`). String `Add` (concatenation) is never folded — it
  allocates a heap node at runtime, so folding it would change
  ownership and ARC traffic (deferred).
- Mixed-type constant pairs are never folded (the type checker
  rules them out in validated MIR; if one ever appears, runtime
  behavior is preserved as-is).

Required: the core invariants. Preserved: the core invariants plus
`Ownership`, `NoOwningCycles`, and `OwnershipBalance` — the
rewrite removes constant reads only, introduces no references, and
never produces a heap-typed result.

**const-prop** — a local `l` is a **single-def constant** when:
`l` is `User` or `Temporary` (not `Parameter`, not `Capture`); the
function contains exactly one
`Assign{target: l, value: Use(Constant(c))}`; `c` is
`Integer`, `FloatBits`, `Bool`, or `Character` (never `String` — a
string operand is a heap node); and `l` is not the root of any
`MirPlace`. Every inline `Operand::Copy(l)` in the function —
`Use`, `Unary`/`Binary` operands, `Branch` condition,
`SwitchEnum` subject, `Store` value, `Return` operand, and
`Await`/`Spawn`/`ListLen` operands — is rewritten in place to
`Operand::Constant(c)`. Operands inside `ListRange` lists (call
and builtin arguments, aggregate fields, closure captures) are not
touched: the operand arena is read-only across crates, and
propagation into it is deferred. The single def itself is kept
(4F removes no instructions).

Required: the core invariants. Preserved: the core invariants plus
the ownership trio (it removes reads, adds none, and heap-typed
locals are never candidates).

**branch-fold** — `Branch{condition, then, else}` becomes
`Goto(target)` when the condition is `Constant(Bool(b))` or
`Copy(l)` where `l` is a single-def Bool constant; `target` is
`then` when `b`, else `else`. The abandoned arm becomes
unreachable; that is legal validated MIR (unreachable blocks never
poison reachable state — the 4B rule) and the interpreter executes
such programs (verified by probe before approval). The block
remains in the program (4F removes no blocks).

Required: the core invariants. Preserved: the core invariants plus
the ownership trio (it removes exactly one read, the condition).

**Safety exit gates (test evidence)**

1. **Equivalence.** For every gate program: `value` and `output`
   are identical across the unoptimized legacy run, the optimized
   legacy run, and the optimized ARC run; `ExecutionStats.steps`
   never increases (in-place rewrites keep the instruction and
   block counts, so steps are in fact equal); the optimized
   program passes `verify_mir` (the manager's per-pass `E4204`
   gate) and `verify_ownership_balance`; `ArcStats` may change
   (balanced reader traffic is removed, e.g. by string-constant
   comparisons) but the balance proof holds and `pinned_live` is
   unchanged. Chains of constant-dependent definitions peel one
   layer per sweep: a chain deeper than the level's
   `fixed_point_iterations` is only partially reduced, and a run
   at `O2` on its own output makes no further change.
2. **Ownership-sensitive regressions.** The 4E
   self-referential and mutually capturing programs (zero leaks
   pinned, `E4304` unpinned) run through the pipeline: the
   optimized MIR proves clean and executes with identical
   `value`/`output`/`pinned_live`. The 4C2B no-hoist shape (a
   `list_len` read followed by a `list_push`, in loop order) is
   untouched: the passes move no instructions, so the read keeps
   its instruction and its order relative to the push.
3. **Compile-fail gates.** Hand-mutated bad inputs are rejected
   with exact codes: a stub pass that writes a `String` constant
   to an Int-typed target fails the manager's post-pass
   verification with `E4204` (a `TypeMismatch` diagnostic); a stub pass
   requiring `OwnershipBalance` before the ownership passes ran
   gets `E4202`; a failing stub gets `E4203`; already-invalid MIR
   at entry gets `E4201`; a hand-mutated use-after-move program
   fails the balance proof with `E4403` both before and after the
   optimization pipeline — the pipeline does not launder invalid
   programs.
4. **Sanitizer evidence.** The 4F suite runs in debug (assertions
   plus overflow checks) and in `--release`, and under Miri where
   the pinned toolchain provides it; every optimized gate program
   also runs under a tight `InterpreterLimits` bound (twice the
   observed step count) proving no hidden step inflation.
5. **Linear growth.** The gate family scales with a size
   parameter N: the pipeline completes for N=20 and N=200,
   equivalence holds at both sizes, and the optimized step count
   grows approximately linearly (the 200/20 ratio bounded away
   from quadratic).

**Out of scope (deferred)**

- No instruction or block removal (dead-code elimination,
  unreachable-block pruning, block merging): the storage is
  append-only and cross-crate passes cannot rebuild a block's
  instruction range.
- No propagation into operand lists (call and builtin arguments,
  aggregate fields, closure captures): the operand arena is
  read-only across crates.
- No string-concat folding, no list-length hoisting, no checked
  indexing erasure, no backend offsets, no value renumbering, no
  loop unrolling, no inlining.
- No production wiring change: the pipeline is opt-in via the
  API; the shadow MIR path is untouched in 4F.

**Exit criteria**

- All five gates green; the full workspace suite green in debug
  and release; strict all-target warnings; the Clippy
  correctness and suspicious groups clean.
- Production stays on `LegacyEngine`; rewrite MIR remains
  shadow-only; the Phase 5 backend contract takes the
  **optimized and revalidated MIR** as its input.

**4F implementation evidence (contract refinements)**

Implementation against the contract surfaced seven boundary
details, each covered by a test or recorded here:

- The probe before approval confirmed both branch-fold
  consequences at once: `verify_mir` accepts an unreachable block
  (constant condition, and the `Goto` fold), and the interpreter
  executes the folded program. No verifier or interpreter change was
  needed.
- Steps tick once per instruction and once per block terminator, so
  in-place rewrites keep the tick count exactly: every equivalence
  gate asserts `steps_after <= steps_before` and observes equality,
  and the tight-limit gate re-runs each optimized program inside its
  exact observed step budget (twice, proving determinism).
- Constant chains peel one layer per sweep: a sweep can only fold a
  binary whose operands are already constants, and a def only becomes
  a single-def constant after the previous sweep folded it. A
  two-layer chain converges in four sweeps (O2's budget); a
  three-layer chain is partially reduced at O2 and completes at O3.
  `fixed_point_iterations` is therefore a depth bound on constant
  chains, and a run stops early on the first no-op sweep.
- Rustc's const arithmetic picks a different NaN payload than the
  runtime SSE operations, so the fold (which applies the same runtime
  `f64` operations as the oracle) is asserted on NaN-ness and
  deterministic bit patterns (infinities, signed zero) in the unit
  tests; exact value agreement against the oracle is covered by the
  e2e NaN/inf equivalence gate through both execution entries.
- A `list_len` read inside a `while` loop over a list whose element
  type is first inferred by a push in the same loop fails v1
  inference (`GenericTypeMaterialization`) — a pre-existing builder
  limitation, not a 4F one. The no-hoist regression therefore uses
  the single-block push/len/push order, which is the same guarantee
  (the read keeps its instruction and its order relative to the
  mutation) without the loop.
- Miri is a nightly-only component and is not available for the
  pinned stable 1.98.0 toolchain; the sanitizer evidence ran the 4F
  suites on the nightly channel's Miri instead (23/23 and 7/7).
- `O0` applies one sweep (the budget table's
  `fixed_point_iterations: 1`), not zero: the budget is the source
  of truth, and O0's single sweep is cheap (three in-place passes).

### Later Phase 4 slices

All Phase 4 slices are now contracted: 4A–4E are implemented and
4F (the optimizer and safety exit) is approved above.

Slice boundaries may be narrowed further when implementation evidence demands
it, but later concerns must not be pulled into an earlier slice without
updating this contract first.

## Current validation

- All 377 workspace Rust tests pass (347 after 4E, plus the 23-test
  4F gate suite in `crates/lpp-passes/tests/phase4f_gate.rs` and the
  7-test 4F ownership suite in
  `crates/lpp-ownership/tests/phase4f_ownership.rs`), in debug and
  optimized-release profiles. The focused enum-flow suites pass 6
  semantic and 13 MIR tests alongside the 14/14 place suite and exact
  accepted legacy/rewrite result differentials. The 4E ARC entry runs
  every 4C3/4D gate program at zero errors and zero leaks with the
  plan's pinned set and deterministic `ArcStats`; the legacy entries
  are bit-for-bit unchanged.
- The 4F optimizer and safety exit is green: every gate program is
  value- and output-identical across unoptimized legacy, optimized
  legacy, and optimized ARC runs with non-increasing steps; the
  optimized MIR passes `verify_mir` per pass and `verify_ownership_balance`
  after the pipeline; the 4E self-referential and mutual-capture
  programs keep their exact `ArcStats` (2 and 4 pinned, zero leaks)
  and their `E4304` unpinned reports; use-after-move keeps its
  `E4403` through the pipeline; the compile-fail gates reject bad
  pass output and bad input with `E4204`/`E4202`/`E4203`/`E4201`;
  20-versus-200 loop growth is linear under the pipeline; and the 4F
  suites pass under Miri (nightly) — 23/23 and 7/7 — alongside the
  debug (assertions plus overflow checks) and release runs.
- All rewrite crates, including `lpp-mir` and `lpp-passes`, pass strict
  all-target `-D warnings`; the whole workspace passes the blocking Clippy
  correctness and suspicious groups.
- Exact `semantic-v3` and `mir-v5` snapshots, malformed enum/place/store/
  reference/CFG/type/initialization cases, checked runtime failures, compact
  layout and capacity assertions, alias-preserving execution, and independent
  20-versus-200 dispatch, payload, and generic-instance growth pass in debug and
  release profiles.
- Frozen v1.2 fixtures pass 113/113; the classified source baseline is
  unchanged; direct ELF passes; native AOT passes 44/44; Node-WASI passes
  34/34; and all ten Unix local-CI stages pass. The driver-routing gate confirms
  production remains on `LegacyEngine`; rewrite MIR remains shadow-only.
