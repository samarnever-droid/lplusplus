# L++ documentation

## Read this first

- [Language & Compiler Specification](../specs.md) — current authoritative language, CLI, backend, and Keel boundary spec.
- [Current Capabilities](CURRENT_CAPABILITIES.md) — verified feature, backend, and platform status matrix.
- [Project Roadmap](../ROADMAP.md) — active priority tiers and open parity gaps.
- [ARC Cycle Breaking](CYCLE_BREAKING.md) — native Bacon-Rajan trial-deletion collector & leak oracle.
- [Concurrency & Atomic ARC](CONCURRENCY.md) — current threading/task boundaries and open `Send`/`Sync` work.
- [Safety Mission](Safety_Mission.md) — S0–S4 safety levels and claims policy.
- [v0.3.0 Release Notes](RELEASE_NOTES_v0.3.0.md) — detailed L++ v0.3.0 release notes ([v0.2.0 brief](../docs/release/V0.2.0.md), [v0.1 archive](../docs/release/V0.1.md)).

## Technical guides

- [External C ABI](EXTERNAL_C_ABI.md)
- [Panic & Errors](PANIC_AND_ERRORS.md)
- [Usage](Usage.md)
- [Networking](Networking.md)
- [Windows Native Toolchain](Windows_Native_Toolchain.md)
- [macOS Native Toolchain](MacOS_Native_Toolchain.md)
- [Native Linker Roadmap](Native_Linker_Roadmap.md)
- [Android & Termux](ANDROID_TERMUX.md)

## Historical pre-rewrite archives

- [Historical Status — 2026-07-30](STATUS-2026-07-30.md) *(superseded by `specs.md` and `CURRENT_CAPABILITIES.md`)*
- [Historical Compiler Reality — 2026-07-30](Compiler_Reality.md) *(describes pre-rewrite `src/` layout, superseded by `ARCHITECTURE.md`)*

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
