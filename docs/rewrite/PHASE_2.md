# Rewrite Phase 2 — span-complete frontend and module graph

Phase 2 adds a clean, shadow-only frontend and package/module graph alongside
the authoritative v1.2 parser and resolver. It does not route code generation
through the new syntax tree. The legacy frontend remains the compatibility
oracle until later feature phases lower the structural tree into typed HIR.

## Workspace layout

```text
crates/
  lpp-frontend/
    src/token.rs       lossless token and keyword model
    src/literal.rs     string, formatted-string and character scanners
    src/lexer.rs       UTF-8 byte spans, trivia, indentation and diagnostics
    src/syntax.rs      structural syntax tree and import AST
    src/parser.rs      declaration, block and compatibility validation
    src/snapshot.rs    deterministic frontend snapshots
    tests/             full-repository acceptance shadow
  lpp-hir/
    src/fs.rs          injected filesystem boundary
    src/graph.rs       deterministic package/module graph
    tests/             legacy multi-file and transitive-debt shadows
```

Both crates are `publish = false`, use only one-way rewrite dependencies, and
are members and default members of the root workspace. `lpp-frontend` depends
only on `lpp-common`; `lpp-hir` depends on `lpp-common` and `lpp-frontend`.

## Frontend contract

The lexer emits a token for every source byte. UTF-8 byte spans are half-open
and tied to a `FileId`. BOMs, spaces, comments, CRLF/LF line endings, and soft
newlines inside delimiters remain in the stream. `Indent`, `Dedent`, and `Eof`
are explicit zero-width synthetic tokens. Concatenating every non-synthetic
token reproduces the source exactly.

Lexical and syntax failures use structured `lpp-common::Diagnostic` values with
stable frontend codes:

- `E1000`–`E1005`: source size, characters, indentation, literals and numbers;
- `E1100`–`E1107`: module structure, delimiters, attributes, blocks and headers;
- `E1110`–`E1111`: tuple arity and variadic-parameter compatibility rules;
- `E1120`: import grammar.

The parser deliberately produces a lossless structural boundary rather than a
second semantic AST. It identifies declarations, indentation-owned child
statements, attributes, imports, aliases and selective imports while retaining
the complete token stream. Detailed expression parsing and feature-specific
semantic lowering belong to later HIR feature capsules.

Compatibility-sensitive checks reject malformed delimiters and declaration
headers, inline statements after a block colon, uninitialized typed bindings,
five-or-more-element tuples, and non-final variadic parameters. Trait and
`extern` function signatures remain bodyless, as in v1.2.

`syntax_snapshot` serializes token kinds/text/spans, syntax kinds/ranges, and
imports in a deterministic format. Its exact unit snapshot prevents accidental
span or token-boundary drift.

## Shadow boundary

The repository-wide frontend shadow recursively parses every `.lpp` file other
than generated build and VCS directories. Its direct-file rejection set is
exactly:

1. `pm/src/publisher.lpp` — brace syntax;
2. `packages/lpp-semver/src/semver.lpp` — typed declaration without assignment;
3. `packages/lpp-toml/src/toml.lpp` — inline statement after a block colon;
4. `tests/tuple_bad_arity.lpp` — five-element tuple;
5. `tests/variadic_bad_position.lpp` — non-final variadic parameter.

`pm/src/main.lpp` is directly valid but its package graph is rejected when the
imported `publisher.lpp` reaches the same brace syntax. This preserves the six
known frontend failures observed at their correct compilation boundary.
Semantic, ownership, type and unresolved-import negative fixtures remain valid
structural input and are intentionally deferred to later stages.

## Package/module graph contract

`lpp-hir` introduces typed `PackageId` and `ModuleId` identities and builds a
real graph from parsed import nodes. All filesystem work is injected through
the object-safe `FileSystem` boundary; `OsFileSystem` is only its production
implementation. Tests use an in-memory filesystem and do not depend on ambient
working-directory state.

Graph construction follows these rules:

1. Canonical paths are identities; paths are never lowercased.
2. Imports resolve with exact component casing, including on a simulated
   case-insensitive filesystem.
3. The importer directory and current package source root have local
   precedence. Dependency packages are sorted by name and root before lookup.
4. Multiple dependency matches are errors rather than order-dependent picks.
5. Modules and source files receive IDs in canonical-path order.
6. Import edges retain the source `ModulePath`, alias/selective-import kind and
   source span.
7. Graph traversal and edge ordering use ordered collections.
8. Import cycles are rejected with a deterministic closed path.
9. Missing and ambiguous imports retain importer and search evidence.

The graph test suite covers aliases, selective imports, dependency-order
independence, exact casing, missing imports, cycles, the existing
`tests/modules` project, and transitive package-manager syntax debt.

## Compatibility authority

No root compiler path consumes `lpp-frontend` or `lpp-hir` yet. The active
`LegacyEngine` continues to lex, parse, resolve, type-check and compile all user
programs. Phase 2 tests are shadow/differential gates; switching authority is a
separate decision after detailed expression and HIR compatibility is proven.

## Acceptance evidence

- [x] `lpp-frontend` and `lpp-hir` compile on pinned Rust 1.98.0.
- [x] Both crates pass full `-D warnings` Clippy checks.
- [x] Exact syntax snapshot, focused lexer/parser tests, and full-corpus shadow
      tests pass.
- [x] The classified direct frontend rejection set is exactly five files.
- [x] The module graph reproduces the existing multi-file import fixture and
      surfaces `pm/src/main.lpp`'s transitive publisher syntax debt.
- [x] Module/package IDs, edges and cycle reports are deterministic.
- [x] The legacy compatibility compiler remains authoritative and unchanged.
- [x] All 168 workspace tests pass, including 19 Phase 2 tests.
- [x] Frozen v1.2 compatibility fixtures pass 113/113.
- [x] Classified source validation remains unchanged: 85/95 repository,
      152/156 official packages, and 168/181 compiler tests, with every failure
      explicitly classified; SamarOS combined-source validation passes.
- [x] Native AOT parity passes 44/44.
- [x] Node-WASI WebAssembly validation passes 34/34.
- [x] The Unix local CI harness, direct ELF link test, strict generated-header
      C11 check, strict runtime C compilation, and release packaging tests pass.

## Phase 2 exit

Phase 2 exits with a reusable lossless frontend boundary and deterministic
module graph, but without semantic expression HIR or a driver authority switch.
Phase 3 can add feature-owned HIR lowering against these shared contracts while
continuing differential validation against v1.2.
