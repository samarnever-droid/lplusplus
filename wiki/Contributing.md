# Contributing

## Before changing code

1. Read the relevant wiki page and existing tests.
2. Identify backend/platform scope.
3. Preserve unsupported-feature rejection rather than adding a silent fallback.
4. Keep generated artifacts out of the repository.

## Required checks

```sh
cargo test --locked
```

Run focused tests for your subsystem. Networking changes require:

```sh
cargo fmt --manifest-path runtime/lpp-net/Cargo.toml --check
cargo test --manifest-path runtime/lpp-net/Cargo.toml --locked
sh tests/test_rust_network_runtime.sh
```

## Pull requests

Explain what is implemented, what is explicitly not implemented, test commands/results, platform assumptions, and any ABI/ownership consequences. Do not make benchmark or safety claims without reproducible evidence.

## v0.1.3 documentation status

For the current supported subset and explicit feature boundaries, see
[`documentation/CURRENT_CAPABILITIES.md`](../../documentation/CURRENT_CAPABILITIES.md).

Do not use historical benchmark numbers or roadmap text as current guarantees.
