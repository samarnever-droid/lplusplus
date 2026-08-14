# c2lpp architecture

## Design principles

1. **Pure L++ orchestration and parsing.** The project itself is L++.
2. **Use the C preprocessor, not a fake macro evaluator.** The JSON-configured
   compiler's `-E` mode with source-marker filtering is optional and isolated
   behind validated shell-safe settings.
3. **Generate an ABI package before attempting source translation.** SQLite and
   zlib become usable immediately without pretending C pointers are safe L++
   owners.
4. **Skip uncertainty.** Unknown by-value ABI, C variadics, and complex
   declarations produce review comments instead of guessed signatures.
5. **Deterministic files.** The same header and policy generate the same package.

## Modules

```text
src/text.lpp
  Bounded string, identifier, and shell-safety helpers.

src/c_config.lpp
  Strict pure-L++ JSON parser, schema/version/type validation, defaults and
  deterministic normalized project configuration. No C2LPP_* environment
  setting is consumed by the converter.

src/c_types.lpp
  Central C ABI → L++ type policy and numeric macro validation.

src/header_parser.lpp
  Comment state machine, define parser, declaration accumulator, callback-aware
  parameter splitter, prototype emitter, include/dependency extraction.

src/source_translator.lpp
  Phase-2 scalar C subset translator with explicit unsupported-code quarantine.

src/c_token.lpp
  Stable token IDs and file/offset/line/column provenance records.

src/c_audit.lpp
  Streaming checked-buffer lexer/inventory. It conserves every source byte,
  emits stable reason codes, and does not materialize a whole-amalgamation token
  list.

src/c_project.lpp
  Multi-file manifest reader and deterministic local/external/unresolved include
  graph.

src/c_lexer.lpp
  On-demand significant-token cursor over checked buffers; tokens retain exact
  file/offset/line/column spans.

src/c_normal_ir.lpp
  Explicit C scalar types, typed expressions, function signatures, symbol tables
  and stable normalized-IR records.

src/c_typed_parser.lpp
  Precedence parser, statement parser, function-atomic validation/quarantine and
  normalized-IR-to-L++ emission.

src/c_native_profile.lpp
  Strict no-binding profile v1 integrating typedef struct/bitfield declarations,
  global relocation dependency, checked pointer places, ownership and CFG.

src/c_frontend_profile.lpp
  Strict no-binding profile v2 integrating macro provenance, forward/callback
  typedefs, nested aggregate arrays, const globals, ordinary/variadic prototypes,
  pointer places and loop/switch/goto lowering. Unsupported input fails without
  a binding fallback.

src/c_translation_unit.lpp
  Allocation-bounded raw-byte translation-unit partitioner. It classifies
  arbitrary-order typedef, aggregate, global, prototype and function records,
  flags function-pointer/variadic declarations, and preserves exact spans.

src/c_declaration_graph.lpp
  Base-type-family and pointer/array/function declarator-shape resolver over TU
  records. It deliberately does not claim complete declarator/ABI semantics.

src/c_function_graph.lpp
  Raw-byte balanced body scanner and structural control inventory for all
  discovered function spans. It does not yet create typed ASTs or resolved CFG
  edges.

src/c_function_sweep.lpp
  Uses TU/body spans to select bounded candidates, pre-registers signatures,
  closes calls over accepted bodies, and emits complete pure-L++ functions.

src/c_global_data.lpp
  Extracts referenced immutable flat integer-array globals with constant
  initializers and emits checked pure-L++ accessors; mutable/relocating data
  fails closed.

src/c_aggregate_data.lpp
  Builds demand-bounded SysV x86-64 aggregate field layouts. Complete records and
  proven safe prefixes drive integer member/array places; ambiguous padding,
  inline aggregates and pointer-field values stay quarantined.

src/c_call_graph.lpp
  Whole-source direct/declared/indirect call sites plus allocation-family sites.

src/c_control_targets.lpp
  Function-scoped label/goto resolver and conservative block/edge inventory.

src/c_ownership_graph.lpp
src/c_ownership_proof.lpp
  Per-function allocation/reallocation/free site classification; explicitly
  marks functions still requiring path-sensitive proof.

src/c_graph_consistency.lpp
  Cross-pass denominator, label, base-type and ownership-count invariants.

docs/GRAPH_VISUALIZATION.md
  Coordinated whole-unit force-graph and per-function CFG/ownership viewer plan.

backends/sqlite/src/
  Vendored, tested pure-L++ SQLite-compatible implementation used only by the
  explicitly labeled curated-backend mode. It is not counted as C translation.

src/c_memory.lpp
  Explicit allocation contexts and provenance-bearing checked C pointers.

src/c_place.lpp
  Flattened typed integer/bitfield lvalues over CPtr allocation descriptors;
  avoids nested managed-pointer ownership and provides checked updates/copies.

src/c_layout.lpp
  Raw explicitly destroyed target layout tables for structs, unions and integer
  bitfields.

src/c_globals.lpp
  Hidden zero-initialized C global/static region and ordered initialization.

src/c_cfg.lpp
  Checked block-state machine used as the lowering target for switch,
  fallthrough, labels and goto.

src/main.lpp
  JSON-configured audit/translation/binding dispatch, optional preprocessing,
  package scaffold and deterministic output files.
```

## Configuration boundary

The executable reads `c2lpp.json` from its working directory because portable
OS argv is not available on every L++ runtime path. The parser accepts one flat,
versioned JSON object; it rejects unknown and duplicate keys, invalid JSON value
types, trailing data, unsupported schema versions and unsafe host-command atoms.
Every successful operation persists a canonical normalized configuration.

## Generated safety boundary

Generated pointer handles are currently `Int`, not ARC references. They must not
be inserted into L++ owning containers as if they were managed objects. A future
policy file will assign handle ownership operations such as:

```text
sqlite3_open  -> creates sqlite3 handle through out parameter
sqlite3_close -> destroys sqlite3 handle
z_stream      -> borrowed/mutable buffer structure
```

Those facts will generate higher-level safe wrappers around the low-level
`extern` layer.

## Why no regex-only parser

The parser tracks comments, declaration accumulation, brace depth, callback
parenthesis depth, identifiers, and unsupported ABI. It remains deliberately
smaller than a complete C parser, but it avoids treating a comma inside a
function-pointer callback as a top-level parameter separator.

## Whole-input audit invariant

Audit mode scans `buf_read` data with checked `buf_get8` access. Every iteration
must advance by at least one byte and contributes exactly its half-open byte
span to `covered_bytes`. A successful report requires
`covered_bytes == bytes`, zero unknown bytes, and no unterminated comments or
literals. Manifest mode additionally requires every quoted include to resolve
to exactly one listed file; system angle includes remain explicit external
edges. This is lexical/dependency coverage, not semantic translation.

## Typed normalized-IR safety model

`translate-ir` does not emit from raw lines. It creates provenance-bearing
tokens, resolves scalar C types and local symbols, parses expressions by C
operator precedence, checks assignment/return/call type and arity, serializes a
stable IR, and emits L++ only for a completely accepted function. A rejected
function is omitted as a unit and carries a stable reason and source span;
parsing then resumes at the following function.

The typed parser integrates provenance-bearing pointers, checked scalar and
aggregate places, pointer arithmetic, typedef casts, referenced immutable arrays,
and a demand-bounded SysV aggregate slice. Data-pointer fields use
`CAbiPointerPlace` and a raw context side table while target storage remains eight
bytes. Conditional returns, local initialization and assignment emit lazy branch
statements rather than eager helpers. Pointer logical operands become explicit
short-circuit null tests. Character constants are normalized to C integer values;
`sizeof` uses target metadata and never evaluates dereference operands. Comma
statements preserve source order, and the body scanner tracks ternary depth so
`:` is not mislabeled as CFG input. Side records propagate across nonoverlap
`memcpy` and `realloc`. Conditional call arguments, function pointers, ambiguous
ABI padding, mutable globals, callbacks and unstructured control remain atomic
rejections.

Pointer depth two is represented as a CPtr to consecutive eight-byte ABI pointer
slots. Dereference and indexing load full provenance through `CAbiPointerPlace`;
assignment updates the side table. One-dimensional C array parameters decay to
CPtr with explicit element width. Pointer depth above two and multidimensional
array parameter layout are rejected rather than guessed.

Braced do/while loops lower to `while true` with an explicit bottom condition;
continue is rejected until a condition trampoline is available. Unbraced if/else
call expressions and empty statements are accepted atomically.

## Translation-unit partition invariant

The raw scanner advances over every active preprocessed top-level declaration,
uses balanced delimiter/string/comment state, and stores graph text in a growing
raw buffer rather than repeated immutable concatenation. Function bodies are
bounded and skipped as units after their exact spans are recorded. The pinned
SQLite gate requires zero unknown declarations; this does not imply semantic translation of those bodies.
The declaration-shape and body-graph passes have separate completeness counters
so 100% structural coverage cannot be mislabeled as implementation conversion.

## Native profile no-binding invariant

For JSON `mode: "native"`, accepted source produces only pure-L++ modules,
normalized IR, JSON/report metadata and a pure package manifest. The profile
asserts zero extern blocks, zero native links, balanced allocation/free and no
pointer escape. Rejected source exits non-zero after writing a diagnostic report;
it never invokes header binding generation.

Profiles v1 and v2 are deliberately narrow. Completeness claims apply only to
their exact accepted grammars, not arbitrary C, zlib or SQLite. Profile-v2
callback/variadic declarations are graph records only when no implementation is
present; they never create an extern binding.

## C compatibility model invariants

- A `CPtr` contains context/allocation identities, offset, subobject bounds,
  generation, mutability and element size; it is never a bare integer address.
- Allocation descriptors become tombstones on free and remain diagnosable until
  explicit context destruction.
- Destroyed contexts retain a 56-byte tombstone header so copied `CMemory` and
  `CPtr` values report `C2-MEM-CONTEXT-DESTROYED` instead of reading freed
  metadata.
- ABI-width pointer fields keep eight-byte object storage and use a raw context
  side table for complete provenance; side metadata is explicitly destroyed.
- Struct/union layout selects an explicit target ABI and uses integer field IDs
  backed by raw tables with explicit destruction.
- C globals live in zeroed hidden storage with uninitialized, initializing and
  initialized states.
- CFG execution has explicit block IDs, transitions, a step limit, checked slots
  and one terminal return.
- ASan+UBSan runs with LeakSanitizer enabled; a managed-list layout design was
  rejected and replaced after that gate found leaks.

## Legacy Phase 2 safety model

The original source translator is allowlist-based. It translates scalar declarations,
fixed local scalar arrays, list-backed indexing/assignment, canonical ascending
`for`, calls, `if`/`else`, `while`, increment/decrement and returns. Pointer
parameters/member access, `sizeof`, ternaries, `switch`, `goto`, and unrecognized
ABI become `c2lpp phase2 unsupported` comments. A generated file is never
described as semantically complete while those markers remain.

Tests compile the same fixture as native C and translated L++, execute both, and
compare output. A strict translation report counts every quarantined construct.
This makes Phase 2 real but deliberately incomplete.

## Future clang integration

The pure-L++ frontend can consume a normalized declaration stream produced by
clang. A later adapter can ask clang for JSON AST and feed a stable intermediate
format to the same L++ type-policy/emitter modules without rewriting package
logic.
