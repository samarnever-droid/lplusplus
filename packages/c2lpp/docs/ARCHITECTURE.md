# c2lpp architecture

## Design principles

1. **Pure L++ orchestration and parsing.** The project itself is L++.
2. **Use the C preprocessor, not a fake macro evaluator.** `cc -E` with source-marker filtering is
   optional and isolated behind an environment contract.
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

src/c_types.lpp
  Central C ABI → L++ type policy and numeric macro validation.

src/header_parser.lpp
  Comment state machine, define parser, declaration accumulator, callback-aware
  parameter splitter, prototype emitter, include/dependency extraction.

src/source_translator.lpp
  Phase-2 scalar C subset translator with explicit unsupported-code quarantine.

src/main.lpp
  Environment/mode contract, optional preprocessing, package scaffold and files.
```

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

## Phase 2 safety model

The source translator is allowlist-based. It translates scalar declarations,
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
