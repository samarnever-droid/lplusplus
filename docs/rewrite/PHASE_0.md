# Rewrite Phase 0 — secure and freeze

Phase 0 makes the existing v1.2 implementation safe and measurable before the
clean feature-oriented compiler is introduced.

## Completed in the foundation change

- [x] Removed unauthenticated remote-shell/tunnel workflows.
- [x] Reduced non-release/non-deployment workflow permissions to `contents: read`.
- [x] Pinned Rust 1.98.0 with rustfmt and Clippy components.
- [x] Replaced the masked `lpp --checkall || true` CI command with an exact,
      reviewed source baseline.
- [x] Classified existing failures as `known-debt`, `custom-dialect`, or
      required `compile-fail` behavior.
- [x] Pinned the v1.2.0 oracle tag/commit and froze 113 compatibility fixtures.
- [x] Added release `SHA256SUMS` generation and checksum verification in Unix
      and Windows installers.
- [x] Rejected unsafe archive paths before release extraction.
- [x] Disabled automatic self-update/global-app pipe-to-shell execution.
- [x] Required a caller-provided trusted SHA-256 digest before portable LLVM
      download/extraction.
- [x] Removed pipe-to-shell installation instructions from maintained docs and
      website source.
- [x] Pinned every third-party GitHub Action to a reviewed immutable commit SHA.
- [x] Added GitHub/Sigstore provenance attestations for release archives and the
      `SHA256SUMS` manifest.
- [x] Added deterministic, metadata-normalized release archive generation with
      reproducibility tests.
- [x] Split default host-ISA builds from explicit `--features all-arch` CI and
      release builds.
- [x] Added an exact-commit v1.2 oracle materializer with `Cargo.lock` and
      optional prebuilt-binary checksum verification.
- [x] Added a separate combined-source SamarOS dialect validator while retaining
      explicit standalone-module incompatibility classifications.
- [x] Archived every remaining v1.2 `known-debt` source failure with an explicit
      rewrite disposition in `compatibility/v1.2.0/KNOWN_DEBT.md`.
- [x] Applied the pinned formatter baseline and enabled blocking
      `cargo fmt --all -- --check` in CI.
- [x] Fixed the high-signal Clippy findings and enabled blocking `correctness`
      and `suspicious` lint groups; lower-signal legacy warnings remain visible.

## Validation completed

- Rust 1.98.0 host-ISA `cargo test --locked`: 128 library tests, 129 binary
  tests, and 2 integration tests passed.
- Native AOT compatibility: 44/44 passed; WebAssembly: 34/34 passed.
- The frozen-fixture checker passed all 113 entries. Three consecutive source
  baseline runs passed with 10 repository, 4 official-package, and 13 required
  compile-fail classifications; the combined SamarOS validator also passed.
- Unix installer integration accepted a verified archive and rejected digest
  mismatch, parent traversal, and symbolic-link attacks.
- Deterministic archive unit tests passed, including repeat-build byte identity.
- Workflow YAML parsing, immutable-action checks, shell/Python syntax, installer
  synchronization, and remote-execution scans passed.
- Website `npm ci`, high-severity audit gate, and Vite 7.3.6 production build
  passed. The upstream Monaco dependency still reports one low and one moderate
  advisory, but no high or critical advisory.

The constrained local environment cannot compile the exact all-architecture
Cranelift set and has no PowerShell runtime. The dedicated all-arch Linux CI job,
Windows installer execution, release provenance publication, and deploy job
therefore require hosted-runner confirmation.

## Remaining before Phase 0 exit

- [ ] Obtain green hosted-runner confirmation for the all-architecture,
      PowerShell/Windows, release-attestation, and Pages jobs that cannot execute
      completely in the constrained local environment.

## Manual impact analysis

GitNexus metadata is mentioned in `AGENTS.md`, but no GitNexus index/runner is
present in this checkout. Before the three touched Rust entry points were
changed, their call sites were inspected with repository search:

| Symbol | Direct upstream caller | Behavioral impact |
|---|---|---|
| `handle_setup_llvm` | `real_main` | setup now requires `LPP_LLVM_ARCHIVE_SHA256` and verifies it |
| `cmd_self_update` | `pm::run_command` | `--check` remains; automatic remote execution returns exit 2 |
| `install_lpp_opencode_global` | `cmd_install_command` | unverified global remote installer returns exit 2 |

No lexer, parser, type-system, MIR, ownership, backend, linker, runtime ABI, or
language semantics were changed in this foundation slice.

## Baseline commands

```sh
python3 scripts/check_compatibility_freeze.py
python3 scripts/check_source_baseline.py target/release/lpp
cargo test --locked
sh tests/run_aot_parity.sh
sh tests/run_wasm_tests.sh
```
