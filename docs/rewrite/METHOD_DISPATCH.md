# UFCS Method Dispatch

This document describes how `receiver.method(args)` calls are resolved and
lowered in the rewrite engine, and which files participate. It exists because
the type/MIR pipeline is dense and method dispatch threads through several
crates.

## Language model

L++ has no separate method-call node. The frontend parses `p.describe()` as a
`Call` whose callee is a `Field { base: p, name: describe }`. Two source forms
map onto the same mechanism:

1. **Free-function UFCS** — `p.magnitude_squared()` where `magnitude_squared` is
   a top-level `def magnitude_squared(p: Point) -> Int`. The receiver becomes
   the first argument: `magnitude_squared(p)`.
2. **`impl` methods** — `p.describe()` resolved to `impl Describe for Point:
   def describe(self) -> Str`. Impl/trait methods are lowered as ordinary
   top-level `HirItemKind::Function` items (`declaration.rs::lower_nested_function`)
   with `definition: None`, so they are *not* name-resolvable as free functions.
   Trait method declarations use `signature_only: true` (no body); impl methods
   have bodies and are therefore reserved and lowered as MIR functions.

The receiver's `self` parameter is given the placeholder type `Self` by the
frontend (`declaration.rs`, `named_type("Self")`), which name resolution leaves
as an `UnresolvedName`.

## Resolution (type stage) — `crates/lpp-types/src/check/`

- **`check.rs`**
  - `method_index: BTreeMap<(DefId, Symbol), HirItemId>` — the compatibility
    single-candidate table exported for older MIR paths.
  - `method_candidates: BTreeMap<(DefId, Symbol), Vec<HirItemId>>` — the full
    candidate set for a receiver nominal definition and method name. This is the
    authoritative dispatch list for overlapping concrete/generic impl methods.
  - `method_generics: BTreeMap<HirItemId, Vec<TypeParamId>>` — in-scope generics
    for each method. Impl lowering folds the used impl parameters into each
    method's own `Function::type_parameters`, so this list is the complete
    generic context for substitution.
  - `resolve_impl_self_types()` — binds `Self` to the impl's target type for
    concrete **and generic** impls. It rewrites the method's parameter locals,
    and re-interns the method's signature type so every occurrence of the `Self`
    placeholder (parameters and result) becomes the target nominal. Generic impl
    methods therefore carry receivers such as `Box[T]` into inference/MIR.

- **`check/call.rs`**
  - `infer_call(call_expression, callee, ...)` — at entry, if the callee is a
    `Field`, tries `infer_method_call(...)`; on `Some(result)` it returns that,
    otherwise falls through to normal call inference.
  - `infer_method_call(...)`:
    1. Infer/resolve the receiver; if not a `Nominal`, return `Ok(None)`.
    2. If `name` is a real struct field, return `Ok(None)` (field value call).
    3. Look up `method_candidates`; if absent, return `Ok(None)`.
    4. Try candidates most-specific first (fewer in-scope generics first).
    5. For each candidate, instantiate its `method_generics` with fresh inference
       vars.
    6. **Trial** unify the receiver + arguments against the (instantiated)
       signature, snapshotting the `InferenceTable` first. If arity or any
       unify fails, restore the snapshot and try the next candidate.
    7. On success, record `AggregateExpressionFact::MethodCall { method }` on the
       call expression and return the result type.

- **`aggregates.rs`** — `AggregateExpressionFact::MethodCall { method }` is the
  fact carried to MIR. `record_constructor_call` ignores it (a method call is
  not a constructor).

## Lowering (MIR) — `crates/lpp-mir/src/builder/expression.rs`

- `lower_call` checks for `AggregateExpressionFact::MethodCall` before the
  constructor/builtin paths and delegates to `lower_method_call`.
- `lower_method_call` extracts the receiver (`base` of the `Field` callee),
  re-resolves generic-body method facts against the concrete receiver through
  `method_candidates`, resolves generic method instances through
  `instance_type_functions` (or concrete methods through `item_functions`), lowers
  `[receiver, ...args]` into an operand list, and emits
  `Rvalue::Call { callee: Operand::Function, .. }`.
- `resolve_field_projection` can derive a projection from the concrete nominal
  receiver type and field name if an older type-check path did not emit an
  `AggregateExpressionFact::FieldProjection`, which keeps monomorphized generic
  impl method bodies lowerable while the field fact surface is tightened.

## Linkage names — `crates/lpp-codegen-cranelift/src/lower.rs`

Distinct functions can share a source name (e.g. `describe` implemented for
several types). Methods are only ever called by function id, so the export-name
loop tracks used names and disambiguates a collision by appending the MIR
function id (`describe`, `describe_5`, ...). This prevents
`IrVerificationFailed(DuplicateDefinition)`.

## Status

Working: direct method calls on concrete types (free-function UFCS and concrete
`impl` methods) — greens `test_methods`, `test_traits`.

Working: **trait-typed parameters** (`def f(x: SomeTrait)`) — greens
`test_dyn_dispatch` (see "Trait-typed parameter desugaring" below).

Working: overlapping concrete/generic impls with most-specific-wins and generic
method monomorphization — greens `generic_trait_impls`.

Working: turbofish on free functions and generic methods — greens
`generics_turbofish`.

The method-dispatch failures that previously held the rewrite corpus at 97.2%
are closed in the 2026-10-04 compiler pass.

## Trait-typed parameter desugaring — `crates/lpp-hir/src/lower/declaration.rs`

A parameter whose annotation is a bare *trait* name is not a trait object in the
rewrite; it is sugar for a trait-bounded generic. During function lowering
(`lower_function`), each non-`self` parameter annotation is passed to
`desugar_trait_parameter`:

1. If the annotation is a `TypeRefKind::Named(sym)` and `sym` resolves — via the
   already-built name index (`symbol_is_trait`, `DefinitionKind::Trait`) — to a
   trait, a fresh type parameter `$impl{n}` is allocated with `bound: Some(<the
   original trait type-ref>)`.
2. The parameter's type-ref is rewritten to `Named($impl{n})`, so the local now
   refers to the synthetic generic.
3. The synthetic `TypeParamId` is appended to the function's explicit type
   parameters (`type_parameter_ids`), and the merged list becomes
   `Function::type_parameters`.

The name index is built **before** lowering (`lower_package`), so trait-ness is
known at this point. This is a pure HIR-stage rewrite: after it, `def
make_speak(animal: Speak)` is identical to `def make_speak[$impl0: Speak](animal:
$impl0)`, which the existing inference + `InstancePlanner` monomorphization
already handle — a concrete `make_speak[Dog]` / `make_speak[Cat]` is emitted per
call site with direct (static) dispatch. No runtime vtable is involved.

Non-goals / guards: only bare `Named` annotations are desugared (applied forms
like `Speak[T]` and tuples are left alone); `self` parameters are skipped; a name
that resolves to a concrete type or an in-scope generic parameter is left
unchanged. Return-position trait types are *not* desugared (returning an
unsized trait object would need boxing) and still fail downstream.

Regression coverage: `crates/lpp-hir/tests/trait_param_desugar.rs` (asserts the
synthetic bounded generic is created and that scalar params stay non-generic) and
the end-to-end corpus file `tests/test_dyn_dispatch.lpp`.
