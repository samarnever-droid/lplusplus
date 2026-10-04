# Rewrite Phase 1 — workspace and common contracts

Phase 1 introduces the clean compiler boundary alongside the frozen v1.2
implementation. It does not rewrite language semantics: the existing compiler
is the first engine behind the new driver contract.

## Workspace layout

```text
crates/
  lpp-common/       source map, stable spans, structured diagnostics, targets
  lpp-driver/       DriverRequest, DriverContext, CompilerSession, engine trait
  lpp-runtime-abi/  ABI schema validation and deterministic generators
abi/
  builtins.toml     v1 builtin/runtime ABI source of truth
  generated/        checked Rust descriptors and exact symbol manifest
runtime/
  include/lpp_abi.h generated C declarations
  lpp-net/          existing Rust network runtime workspace member
```

The root compatibility package and existing `lpp-net-runtime` package are
workspace members; the obsolete nested lockfile was removed so all Rust code is
resolved by the pinned root lock. New crates are
`publish = false`, use one-way dependencies (`lpp-driver` depends only on
`lpp-common`), and pass full `-D warnings` Clippy checks.

## Common contracts

- `FileId` is a stable session-local integer identity.
- `Span` is a validated half-open byte range tied to one `FileId`.
- `SourceMap` preserves source text, precomputes line starts, validates UTF-8
  boundaries, and exposes deterministic `FileId` order.
- `Diagnostic` owns a stable code, severity, message, primary span, labels,
  notes, and help. Rendering happens only at the driver boundary.
- `TargetSpec` validates native triples and models the three legacy WebAssembly
  spellings without relying on target-lexicon behavior that predates WASIp1.

The legacy stages still return strings internally. They will adopt structured
stage errors as each stage is replaced; Phase 1 does not parse those strings
into identities in the new crates.

## Driver boundary

`lpp-driver` provides a typed `DriverRequest`, session-owned `DriverContext`,
`CompilerEngine`, `DriverOutcome`, and `CompilerSession`. The root CLI now
contains only request creation, stack/thread setup, diagnostic rendering, and
final process exit. `LegacyEngine` adapts the unchanged v1 orchestration behind
that contract.

As part of the move, process termination was removed from the legacy engine:
configuration and child-program paths return exit codes to the CLI instead.
The compiler implementation is compiled once through the library rather than
being declared a second time in the binary. The compatibility unit suite is
therefore reported once (129 tests), while a new integration test proves that
`lpp --version` reaches the v1 engine through `CompilerSession`.

`src/legacy_driver.rs` temporarily retains the pre-rewrite monolithic driver.
It is compatibility code moved from the old binary, not a new-stage design;
its replacement is distributed across the feature-oriented crates in later
phases and no new feature logic may be added to it.

## ABI registry and generation

`abi/builtins.toml` records all 512 entries from the v1 builtin table, including
source names, symbols, semantic and lowered signatures, feature ownership,
ownership transfer, effects, and target availability. Fields not yet audited
are explicitly marked `legacy_unspecified`/`legacy`, rather than guessed.

The generator validates the schema and deterministically emits:

- `abi/generated/builtins.rs` — typed Rust-facing descriptors;
- `abi/generated/v1.symbols` — 354 unique linked symbols;
- `runtime/include/lpp_abi.h` — declarations for 349 `lpp_*` runtime symbols;
- `docs/reference/BUILTINS_GENERATED.md` — generated reference table.

Two v1 conflicts are represented explicitly instead of silently normalized:

1. `file_size`/`lpp_file_size` are duplicate source names for path and file
   descriptor operations.
2. `lpp_thread_spawn` has void and i64 table declarations; all shipped C
   runtimes return `int64_t`, which is the generated canonical declaration.

Workspace tests compare the schema's complete symbol set directly with
`src/builtins.rs` and fail if any checked generated output drifts. Regenerate
with:

```sh
cargo run --locked -p lpp-runtime-abi --bin lpp-abi-gen -- \
  abi/builtins.toml .
```

## Acceptance evidence

- [x] Workspace and the three Phase 1 crates compile on pinned Rust 1.98.0.
- [x] New crates pass Clippy with full `-D warnings`.
- [x] Workspace tests pass: 129 compatibility-library tests, one CLI routing
      integration test, two existing runtime/linker integration tests, three
      `lpp-net-runtime` tests, and 14 tests across the new crates.
- [x] The CLI invokes the legacy v1 pipeline through `CompilerSession`.
- [x] ABI generation is deterministic and reproduces all 354 v1 linked symbols.
- [x] The generated C ABI header passes strict C11 syntax compilation.
- [x] Frozen compatibility fixtures pass 113/113.
- [x] Classified source validation remains unchanged: 85/95 repository,
      152/156 official packages, and 168/181 compiler tests, with every failure
      explicitly classified; SamarOS combined-source validation passes.
- [x] Native AOT parity passes 44/44.
- [x] Node-WASI WebAssembly validation passes 34/34.

## Phase 1 exit

Phase 1 exits with the old implementation still selected unconditionally as
`legacy-v1`. Phase 2 can now add the span-complete frontend and module graph as
new driver stages without coupling them to CLI process behavior or backend
internals.
