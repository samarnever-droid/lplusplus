# L++ documentation

## Read this first

- [Language & Compiler Specification](../specs.md) — current authoritative language, CLI, backend, and Keel boundary spec.
- [Current Capabilities](CURRENT_CAPABILITIES.md) — verified feature, backend, and platform status matrix.
- [Project Roadmap](../ROADMAP.md) — active priority tiers and open parity gaps.
- [ARC Cycle Breaking](CYCLE_BREAKING.md) — native Bacon-Rajan trial-deletion collector & leak oracle.
- [Concurrency & Atomic ARC](CONCURRENCY.md) — current threading/task boundaries and open `Send`/`Sync` work.
- [Safety Mission](Safety_Mission.md) — S0–S4 safety levels and claims policy.
- [v0.3.0 Release Notes](release/V0.3.0.md) — detailed L++ v0.3.0 release notes ([v0.2.0 brief](release/V0.2.0.md), [v0.1 archive](release/V0.1.md)).

## Technical guides & reference

- [Generated Builtins Reference](reference/BUILTINS_GENERATED.md)
- [External C ABI](EXTERNAL_C_ABI.md)
- [Panic & Errors](PANIC_AND_ERRORS.md)
- [Usage](Usage.md)
- [Networking](Networking.md)
- [Windows Native Toolchain](Windows_Native_Toolchain.md)
- [macOS Native Toolchain](MacOS_Native_Toolchain.md)
- [Native Linker Roadmap](Native_Linker_Roadmap.md)
- [Android & Termux](ANDROID_TERMUX.md)

## Architecture & design notes

- [Compiler Architecture](../ARCHITECTURE.md) & [Rewrite Phases](rewrite/PHASE_6.md)
- [Design Notes](design/Cranelift_Safety_Plan.md) (`docs/design/`)

## Wiki

The version-controlled wiki lives in [`wiki/`](../wiki/Home.md).

## Verification

Primary repository gates (see [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) and [`scripts/fast_gate.sh`](../scripts/fast_gate.sh)):

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
python3 scripts/check_compatibility_freeze.py
sh tests/run_aot_parity.sh
```
