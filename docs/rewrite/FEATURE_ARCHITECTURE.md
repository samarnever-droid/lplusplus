# Accepted clean-rewrite architecture

The rewrite keeps one shared compiler pipeline and organizes language-specific
behavior as feature capsules.

```text
crates/
  lpp-common/
  lpp-frontend/        shared lexer, parser and AST infrastructure
  lpp-hir/
    src/
      {arena,ids,interner,ir,list,resolve}.rs
      lower.rs
      lower/{declaration,expression,statement}.rs
  lpp-types/
  lpp-mir/
  lpp-ownership/
  lpp-passes/
  lpp-features/
    src/
      map/
        mod.rs
        syntax.rs
        typecheck.rs
        lower.rs
        validate.rs
        abi.toml
        tests/
      list/
      string/
      tuple/
      slice/
      async_task/
      closure/
      struct_type/
      enum_type/
      ffi/
  lpp-codegen-api/
  lpp-codegen-cranelift/
  lpp-codegen-llvm/
  lpp-codegen-wasm/
  lpp-runtime-abi/
  lpp-linker/
  lpp-driver/
  lpp-cli/
  lpp-pm/
  lpp-lsp/
```

Phase 1 established `lpp-common`, `lpp-driver`, and `lpp-runtime-abi` as
workspace crates. Phase 2 adds the lossless `lpp-frontend` boundary and the
injected-filesystem package graph in `lpp-hir`; both remain shadow-only while
the v1.2 parser and resolver are authoritative. See
[`PHASE_1.md`](PHASE_1.md) and [`PHASE_2.md`](PHASE_2.md) for contracts and
validation evidence. Phase 3 now provides compact shared HIR arenas, lexical scopes,
origin-preserving declaration/expression/statement lowering, typed operand
ranges, canonical interned types, and bounded union-find inference. Feature
capsules consume these shared contracts; no feature logic is added to the
legacy adapter. Phase 4A establishes compact typed CFG storage, bounded
TypedHIR-to-MIR construction, structural/type verification, deterministic MIR
snapshots, and a verify-after-each-pass manager in `lpp-mir` and `lpp-passes`.
Phase 4B adds deterministic dense-bitset definite initialization as a core MIR
invariant and a verified, bounded execution oracle. Every pass must preserve
reference, CFG, type, and definite-initialization validity; reachable
uninitialized reads fail before another pass can observe the program. The
oracle executes current scalar, aggregate, call, and control-flow MIR with
explicit work limits and deterministic statistics, so later transformations
can be checked for observable equivalence without routing production through
the rewrite. Ownership policy remains outside the MIR model and is reserved
for `lpp-ownership`. Optimization is a cross-phase requirement defined in
[`OPTIMIZATION_STRATEGY.md`](OPTIMIZATION_STRATEGY.md), beginning with compact
frontend/HIR storage and continuing through MIR, ownership, backends, runtime,
and linking.

## Boundary rule

A feature owns its syntax contribution, typing rules, typed-MIR lowering,
validation, ABI declarations and tests. It does **not** own another complete
lexer, parser or backend.

```text
shared source/parser infrastructure
              ↓
       feature semantics
              ↓
         validated MIR
       ↙       ↓       ↘
Cranelift     LLVM     WASM
              ↓
      generated runtime ABI
```

Backends translate common typed MIR. Feature-specific backend adapters are
allowed only when common MIR plus a runtime ABI call cannot represent the
operation.

Runtime implementation is organized by feature with thin platform adapters:

```text
runtime/
  core/{alloc,arc,panic}/
  features/{map,list,string,task,network}/
  platform/{posix,windows,linux-freestanding,windows-freestanding}/
```

`abi/builtins.toml` will become the single source for source names, runtime
symbols, parameter/result types, ownership transfer, effects and target
availability. Rust declarations, C headers, backend tables, parity tests and
reference documentation will be generated from it.

## MIR validation and oracle rule

Validated MIR is the only input accepted by the Phase 4B execution oracle. Core
validation runs in this order:

1. references and function-local ownership of IDs;
2. CFG structure and successor legality;
3. instruction, operand, call, branch, and return types;
4. reachable definite initialization.

Definite initialization is a forward must analysis: parameters are initialized
at entry, instruction operands are read before their target becomes
initialized, predecessor states intersect at joins, loop back edges converge to
a fixed point, and unreachable blocks do not affect reachable states. Analysis
state is a dense function-local bitset with caller-configurable state-word and
iteration limits.

The interpreter preserves typed values rather than coercing everything through
a host scalar. It reports typed invalid-input, limit, and runtime failures and
retains available function/block/origin evidence. Step, call, depth, and
aggregate statistics are deterministic and may be used as non-timing
performance gates. It is a shadow-only semantic oracle, not a production
runtime or an ownership model.

Generic MIR consumes the stable demand-driven `InstancePlanner` output rather
than cloning and renaming HIR. A generic template has no executable MIR
identity. Each required concrete `InstanceId` is materialized once, retains its
ordered `TypeId` arguments through the Phase 3 plan, and substitutes signature,
local, and expression types under explicit MIR work/depth limits. Explicit
calls resolve by `(item, arguments)` and inferred calls by indexed concrete
function type; neither path scans generated bodies or formats type names.

Nominal features follow the same identity rule. The type stage resolves
constructor, variant, and field syntax to compact HIR identities and records
those facts beside canonical expression types. MIR consumes those facts and
materializes one concrete descriptor per used nominal `TypeId`; descriptor
fields and variant payloads are already substituted and verifier-visible.
Executable MIR never resolves aggregate semantics by spelling, guesses field
layout, or scans unrelated declarations. Immutable construction/projection is
established before places and stores, and both precede ownership placement.

A place is a typed function-local root plus a compact contiguous projection
path. Reads and stores share that representation, but instructions distinguish
value-defining assignment from effectful mutation. Dynamic index operands are
lowered once into the path; augmented assignment reuses one materialized place.
The shadow interpreter may use private handles to preserve source aliasing, but
those handles are oracle implementation details and do not select production
heap placement, cycle breaking, or reference-count policy.

Remaining feature MIR, ownership placement, ARC/move-out, and scalar/CFG
optimization remain assigned to later Phase 4 gates.

## Language evolution rule

Compatibility is a floor, not a reason to preserve accidental complexity. At
each feature boundary, review the accepted syntax and semantics for missing
capabilities, ambiguity, unsafe fallback behavior, and disproportionate user
ceremony. Proportional improvements may ship with that feature when they follow
all of these rules:

1. Existing valid syntax and observable behavior remain accepted unless a
   separately documented safety correction makes compatibility impossible.
2. Additive spellings normalize in the frontend to one canonical HIR form; no
   parallel type checker, MIR operation, optimizer path, or backend dialect is
   created for syntactic sugar.
3. New semantics receive resolved type-owned facts and validated common MIR,
   not parser flags or backend-local name matching.
4. Diagnostics explain the canonical rule and may suggest the clearer form.
5. Compile-time work, final storage, generated code, and runtime behavior remain
   explicitly bounded and receive proportional-growth tests.
6. A syntax form is not advertised as public while only the shadow rewrite can
   parse it. It must be coordinated with the production cutover or an additive
   compatibility frontend.
7. Large adjacent features are recorded with architecture hooks and deferred
   when implementing them would obscure the current feature's proof boundary.

For example, colon-delimited match arms remain compatible. A concise `=>` arm
spelling or expression-form match may later desugar to the same enum-flow HIR,
but must not create a second pattern engine or a rewrite-only public dialect.

## Quality rule

Every rewritten crate starts formatted and warning-clean: CI runs
`cargo fmt --all -- --check`, denies Clippy `correctness` and `suspicious` for
the compatibility tree, and uses full `-D warnings` for new rewrite crates.
Legacy warning suppressions may not be copied into a new crate without a local
safety comment and review.
