# L++ documentation

## Read this first

- [Current status — 2026-07-30](STATUS-2026-07-30.md)
- [Current capabilities](CURRENT_CAPABILITIES.md)
- [Compiler reality](Compiler_Reality.md)
- [Safety contract](Cranelift_Safety_Plan.md)

These files describe the current MIR-first compiler. Older reports and design
notes are historical unless they are linked from the status page.

## Main guides

- [Language guide](../Doc.md)
- [Usage](Usage.md)
- [Networking](Networking.md)
- [Windows native toolchain](Windows_Native_Toolchain.md)
- [macOS native toolchain](MacOS_Native_Toolchain.md)
- [Native linker roadmap](Native_Linker_Roadmap.md)
- [Runtime architecture](../runtime/ARCHITECTURE.md)
- [Safety mission](Safety_Mission.md)

## Wiki

The version-controlled wiki is in [`wiki/`](../wiki/README.md). Architecture,
type-system, status, and linker pages are updated against the same status source.

## Verification

The repository's primary executable checks are:

```sh
cargo test --release -j1
sh tests/run_aot_parity.sh
sh scripts/check_safety_mission.sh
```

Package validation is documented in the status page and handoff notes.
