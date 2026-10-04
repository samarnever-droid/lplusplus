# Rewrite Phase 3 — resolved HIR, types, generics, and traits

## Rewrite roadmap (Phases 0–7)

1. **Phase 0 — secure and freeze** — complete: remove unsafe CI paths, pin the
   toolchain and dependencies, and freeze the v1.2 compatibility oracle.
2. **Phase 1 — workspace and common contracts** — complete: typed driver,
   diagnostics, sources, targets, optimization options, and generated ABI data.
3. **Phase 2 — frontend and modules** — complete: lossless parsing, deterministic
   package graph, exact-case imports, and edition-1 resolution compatibility.
4. **Phase 3 — HIR, types, generics, and traits** — complete: compact resolved
   HIR, canonical inference, trait solving, stable instances, and semantic
   parity exit gates.
5. **Phase 4 — MIR and ownership** — planned: typed MIR, verification, pass
   management, cycle breaking, escape analysis, and ARC/move planning.
6. **Phase 5 — backends** — planned: shared backend contract followed by
   Cranelift, WebAssembly, and LLVM parity.
7. **Phase 6 — linker and runtime** — planned: modular object/link contracts,
   generated runtime ABI integration, and platform adapters.
8. **Phase 7 — CLI, PM, LSP, and cutover** — planned: secure tooling,
   incremental compiler reuse, old/new shadowing, and controlled default-engine
   cutover.

Phase 3 is complete. Optimization is delivered with each semantic contract
under [`OPTIMIZATION_STRATEGY.md`](OPTIMIZATION_STRATEGY.md), rather than
postponed until backend work.

## Phase 3 slices

1. **3A — optimized identity and resolution foundation** — complete
   - one shared source buffer per parsed module;
   - compact tokens without token-owned strings;
   - one-pass module parsing with `FileId` remapping;
   - compact typed IDs and contiguous typed arenas;
   - identifier interning;
   - deterministic module scopes, import bindings, and edition-1 flat view.
2. **3B — arena HIR and expression lowering** — complete
   - expression/statement IDs rather than recursive boxed trees;
   - origin chains and structured lowering diagnostics;
   - module-local definitions, locals, fields, variants, and type parameters.
3. **3C — canonical types and inference** — complete
   - interned `TypeId` shapes;
   - union-find inference variables with occurs checks;
   - function/container/tuple/nominal types and deterministic substitutions.
4. **3D — traits and generics** — complete
   - indexed, memoized trait solving with coherence and bounded search;
   - demand-driven monomorphization with stable instance keys and growth caps.
5. **3E — compatibility and performance exit** — complete
   - v1 type/diagnostic differential suite;
   - deterministic HIR/type/instance snapshots;
   - compile-time, memory, scaling, specialization-size, and concurrent-work
     gates.

## 3A contracts

`StringInterner` stores each identifier spelling once and returns a four-byte
`Symbol`. `Arena<I, T>` provides contiguous allocation behind typed four-byte
IDs such as `DefId`, `ExprId`, `StmtId`, `LocalId`, and `InstanceId`.

`build_name_index` walks modules and declarations in stable graph/source order.
It creates module-local definition maps and resolves module aliases and
selective imports to typed targets. `ResolutionMode::Namespaced` retains clean
module identity. `ResolutionMode::LegacyFlat` adds the v1 compatibility view
without copying or destructively flattening declarations.

Resolution diagnostics are structured:

- `E3001` — duplicate definition in one module;
- `E3002` — missing name in a selective import;
- `E3003` — colliding import binding;
- `E3004` — duplicate definition in the edition-1 flat namespace;
- `E3099` — compact ID/interner capacity exhaustion.

## Optimization evidence required for 3A

- `Token` is exactly 16 bytes and contains no heap-owning `String`.
- Token text is a source-span view into one `Arc<str>`.
- Parsed syntax and `SourceMap` share the same source allocation.
- Item metadata references syntax by index instead of cloning declaration trees.
- Package graph discovery parses each file once, then remaps file identities.
- `Symbol` and all arena IDs are four bytes.
- Repeated name interning returns one symbol and one stored spelling.
- Dependency ordering and observable outputs remain deterministic.

## 3B contracts

`lower_package` lowers a deterministic package graph and its `NameIndex` into
one `HirPackage`. Expressions, statements, bodies, scopes, locals, syntactic
type references, type parameters, fields, variants, match arms, and items live
in typed contiguous arenas. Calls, tuples, lists, blocks, parameters, fields,
and other variable-length operands use eight-byte `IdRange<I>` values into
append-only `IdList<I>` stores instead of one `Vec` allocation per node.

Expression lowering covers literals, names, unary and binary operators, calls,
explicit generic calls, fields, indexing, tuples, lists, `?`, postfix `await`,
`spawn`, and inline or indented closures. Statement lowering covers bindings,
destructuring, assignments, returns, branches including `elif`, loops, match,
`break`, and `continue`. Declaration lowering preserves functions, structs,
enums, traits, impls, extern blocks, constants, aliases, generic parameters,
parameters, fields, and variants.

Lexical scopes own ordered binding lists. Every declaration creates a distinct
`LocalId`, including compatible same-scope shadowing, and each name expression
records a local, definition, or module target when resolution is already
possible. Unknown global names remain explicit `NameBinding::Unresolved` for
the Phase 3C builtin/type resolver; lowering never guesses builtin semantics by
string.

Every HIR node references an `OriginId`. Direct nodes have source origins;
rewrites have a typed desugaring origin linked to their source parent. Phase 3B
records augmented assignment, `elif`, and implicit inline-closure returns this
way.

Lowering diagnostics are structured:

- `E3100` — malformed declaration, statement, expression, or type syntax;
- `E3101` — unsupported declaration placement;
- `E3102` — non-place assignment target;
- `E3103` — orphaned branch or match-arm node;
- `E3104` — invalid decoded numeric literal;
- `E3199` — HIR arena, list, or interner capacity exhaustion.

## Optimization evidence required for 3B

- Every arena ID and `Option<ID>` is four bytes through nonzero ID encoding.
- `IdRange<I>` is eight bytes.
- `Expression`, `Statement`, `Origin`, and `Local` are respectively 32, 20,
  20, and 24 bytes and own no recursive boxes or vectors.
- Arena and list capacities are estimated once from package token/item counts.
- Name resolution stores compact IDs and same-scope shadowing does not clone
  expression trees.
- Augmented assignment reuses the target `ExprId`; it does not clone the left
  expression.
- Repeated package lowering is structurally identical.
- The standalone lowering shadow covers at least 180 repository programs; the
  current baseline is 185.

## 3C contracts

`lpp-types` is a one-way rewrite crate over `lpp-common` and `lpp-hir`; it does
not import the legacy analysis modules. `TypeInterner` assigns canonical
four-byte `TypeId` values to error, never, primitive, tuple, list, map, slice,
task, function, nominal, generic-parameter, bound-variable, inference-variable,
and unresolved-name shapes. Equal shapes and equal variable-length type lists
reuse one identity.

`InferenceTable` stores inference variables contiguously and uses union by rank,
deterministic stable-ID tie breaking, and path compression. Every variable
retains an explicit inference level, optional source origin, optional binding,
and canonical self type. Binding performs an iterative, work-budgeted occurs
check and lowers nested variable levels to prevent escaping inference state.
Structural unification covers functions, containers, tuples, and nominal type
arguments and reports deterministic mismatch, arity, capacity, occurs-check,
work-limit, and depth-limit errors.

Generalization replaces eligible representatives with indexed bound variables
in stable `InferVarId` order and records each source variable's level and
origin. Instantiation allocates fresh variables in that same order. Explicit
generic substitutions use ordered `TypeParamId` bindings, memoize each visited
shape during a rewrite, and return canonical interned output. Normalization,
generalization replacement, instantiation, and substitution consume a shared
work budget; recursive structural rewrites also enforce an explicit depth cap.

`infer_hir_package` is a shadow-only typed-HIR pass. It registers signatures
before bodies, resolves primitive/container/tuple/nominal type references,
instantiates declared generics, infers literals, locals, expressions, calls,
closures, lists, control-flow bodies, and returns, and records compact
expression/local/type-reference/item assignments. The pass never routes the
root compiler away from `LegacyEngine`.

## Optimization evidence required for 3C

- `TypeId`, `TypeListId`, `InferVarId`, and each optional ID are exactly four
  bytes.
- `TypeKind` is exactly 12 bytes and `InferenceVariable` is exactly 24 bytes.
- Canonical type/list interning prevents duplicate structural allocations.
- Inference representatives use rank, path compression, and stable-ID tie
  breaking rather than linear substitution chains.
- Occurs checks and graph discovery are iterative; every type operation has a
  finite work budget and recursive rewrites have a 256-level default cap.
- Ordered generalization metadata and substitutions make repeated inference
  structurally identical.
- The permanent repository type shadow analyzes all 185 standalone lowered
  programs deterministically within default budgets; 148 currently complete
  without a shadow type error. Remaining semantic differences stay
  non-authoritative until the Phase 3E differential gate.
- A real two-module generic/list/tuple package receives complete canonical HIR
  assignments and produces identical output on repeated runs.

## 3D contracts

`TraitIndex` lowers resolved HIR trait impls into compact `TraitImplId` rules.
Candidates are indexed by `(trait DefId, self-type head)` with a separate
wildcard bucket for generic self patterns, so solving a nominal goal does not
scan unrelated traits or nominal types. Canonical `TraitGoal` results are
memoized. Candidate IDs, substitutions, diagnostics, and ambiguity lists remain
in stable order.

Coherence structurally unifies implementation patterns. Equally specific
patterns that can overlap are rejected in source order; a strictly more
specific concrete pattern may overlap a generic fallback and wins selection.
Declared generic bounds become recursive trait goals after candidate
substitution. Unresolved inference inputs return `Deferred`; missing impls,
ambiguity, cycles, and bounded-search failures remain distinct outcomes.
`TraitSolverLimits` caps recursive depth, candidates per goal, candidates for an
entire solve, goals, and structural work.

`InstanceKey` is the eight-byte pair `(HirItemId, TypeListId)`. It identifies
functions, nominal constructors, and nested HIR items without string mangling or
cloned types. `InstancePlanner` deduplicates keys before materialization,
assigns compact `InstanceId` values while consuming a `BTreeSet` worklist, and
records parent, source origin, depth, round, and required-versus-optional
status.

HIR generic demands are collected from normalized call and constructor types.
Demands inside a generic item remain parameterized dependencies rather than
eager roots. Materializing a concrete parent substitutes those dependencies
and queues newly concrete children, so nested `deep[T] -> wrap[T] -> Box[T]`
chains advance one deterministic fixed point. Required instances have global,
per-item, depth, and round limits. Optional specialization additionally consumes
the profile's `optional_specialization_instances` and
`fixed_point_iterations` budgets; disabling optional growth at `O0` never
suppresses a required instance.

Trait and instance diagnostics and plans are part of `ShadowTypeOutput`. The
root compiler remains authoritative through `LegacyEngine`; Phase 3D does not
route production compilation through rewrite semantics.

## Optimization evidence required for 3D

- `TraitImplId`, `InstanceId`, and their optional forms are four bytes;
  `InstanceKey` is eight bytes.
- A structural test indexes 512 same-trait implementations with distinct
  nominal heads and visits exactly one candidate for a matching goal.
- Repeating a canonical goal hits the solver memo without another candidate
  search.
- Candidate matching and coherence reuse canonical `TypeId` graphs and ordered
  `TypeSubstitution` bindings rather than cloning HIR or formatting type names.
- Generic discovery stores one parameterized dependency per call shape and
  specializes only keys reached by concrete demand; it does not build Cartesian
  products of observed type arguments.
- The ordered worklist deduplicates recursive and repeated requests before code
  generation, and every growth dimension has a hard cap.
- A real HIR test discovers eight stable instances across two concrete types,
  including a two-edge transitive chain. Repeated runs produce identical trait
  indexes and instance plans.
- The 185-program repository shadow retains at least 11 indexed trait rules and
  21 demanded generic instances with no trait-index or instance-collection
  diagnostics among programs that complete shadow typing.

## 3E exit contracts

`hir_snapshot` serializes the complete resolved HIR contract in stable-ID
order: symbols, definitions, module scopes, every typed arena, and each compact
`IdList` store. It never observes hash-map iteration order. `semantic_snapshot`
adds canonical interned types and type lists, inference variables, normalized
expression/local/type-reference/item assignments, generalized schemes, indexed
trait rules with specificity, trait diagnostics, and demanded instance records.
The exact representative contract is checked in at
`crates/lpp-types/tests/snapshots/phase3-semantic.snap`; both an exact comparison
and repeated repository shadows guard determinism.

`semantic_metrics` counts HIR payloads, interned shapes, inference state,
assignments, schemes, trait rules, diagnostics, and instances. Its
`minimum_payload_bytes` is a deterministic structural payload floor computed
from element counts and Rust element sizes, not an allocator- or host-specific
RSS estimate. The scaling gate compares 200 and 2,000 generic calls and bounds
HIR nodes, structural payload, serialized contract size, elapsed compile time,
and required specializations. Every hard assertion uses deterministic counts,
a broad same-process ratio, or a deliberately loose timeout rather than a
historical timing from another machine.

The root differential suite runs the authoritative v1 lexer, parser,
monomorphizer, resolver, and type checker beside the rewrite graph, HIR, and
shadow type pass. It currently freezes acceptance for 24 curated positive v1
programs and the stable `TypeMismatch` class for three negative programs. This
work also closed two observed compatibility gaps: indexing `Str`/`StrSlice`
produces `Str`, and named type aliases resolve to their target with an explicit
cycle diagnostic. `LegacyEngine` remains the production authority; passing
this bounded suite does not change engine selection.

## Optimization evidence required for 3E

- The checked-in semantic snapshot contains all HIR/type/trait/instance IDs and
  is byte-identical on repeated runs.
- The 200-to-2,000-call release measurement grew from 2,020 to 20,020 HIR nodes,
  82,440 to 816,840 minimum payload bytes, and 447,729 to 4,563,301 snapshot
  bytes: each remains linear within the structural gate.
- Repeating one concrete generic call 200 or 2,000 times produces exactly one
  required instance, preventing specialization-size growth with call count.
- On the recorded two-worker local release run, 12 independent 500-call
  packages took 56 ms sequentially and 33 ms concurrently (parallel/sequential
  ratio 0.581). This is evidence, not a brittle universal speed threshold; CI
  instead requires parallel results to be byte-identical and enforces only a
  catastrophic-regression ceiling.
- A single 2,000-call release shadow compile took 10 ms on that evidence run;
  the executable gate permits up to ten seconds so host noise cannot masquerade
  as an algorithmic regression.

## Current state

Slices 3A, 3B, 3C, 3D, and 3E are implemented in `lpp-common`, `lpp-driver`,
`lpp-frontend`, `lpp-hir`, and `lpp-types`. Shared typed optimization profiles
carry bounded inlining, specialization, unrolling, and fixed-point budgets
through a `CompilerSession`. Compact source/token/HIR/type storage, interning,
namespaced plus legacy-flat resolution, origin-preserving lowering, canonical
type inference, indexed trait solving, and demand-driven stable generic
instances are covered by unit, repository-shadow, and real multi-file tests.

The root compiler still uses `LegacyEngine`; the typed HIR remains a shadow
stage after Phase 3 so later MIR and backend parity work cannot silently change
production behavior. The next implementation slice is the bounded Phase 4A
typed-MIR foundation described in the Phase 4 plan.

## Current validation

- All 210 workspace tests pass, including 30 focused `lpp-types` tests and the
  two root legacy-versus-rewrite compatibility tests.
- The Phase 2 full-repository syntax shadow and 185-program HIR lowering shadow
  remain exact; the type/trait/instance shadow analyzes the same 185 programs
  twice deterministically.
- The exact semantic snapshot, linear structural scaling, specialization-size,
  loose compile-time, and deterministic concurrent-work gates pass in debug and
  release profiles.
- Rewrite crates, including `lpp-types`, pass full `-D warnings`; workspace
  Clippy correctness and suspicious gates pass. Hosted and Unix/PowerShell local
  CI definitions enforce the strict crate set.
- Frozen v1.2 fixtures pass 113/113.
- Classified source validation remains unchanged.
- Native AOT parity passes 44/44; Node-WASI passes 34/34.
- The complete ten-stage Unix local CI harness passes.
