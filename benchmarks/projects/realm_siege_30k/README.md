# Realm Siege 30K — Cross-language game-scale benchmark project

A deterministic single-player tower-defense simulation expressed in six native-language implementations:

```text
L++
C
C++
Rust
Go
Java
```

The project is generated reproducibly rather than committing hundreds of thousands of generated lines. Each implementation has the same game model:

- waves of enemies
- deterministic spawn/health/damage math
- tower upgrades
- resource economy
- branch-heavy combat decisions
- repeated helper calls
- deterministic final score oracle

Generate all sources:

```sh
python3 generate.py
```

Generated artifacts are written under `generated/` and intentionally ignored by Git. The source count target is at least 30,000 lines **per language implementation**.

## Full multi-file L++ package

The generator also creates a real L++ package at:

```text
generated/lpp_project/
├── lpp.toml
└── src/
    ├── main.lpp
    ├── rooms_0.lpp
    ├── rooms_1.lpp
    ├── rooms_2.lpp
    ├── rooms_3.lpp
    └── rooms_4.lpp
```

It contains more than 40,000 L++ lines split across six source files. With an installed L++ runtime, build it as a package:

```sh
cd generated/lpp_project
lpp build
./target/release/realm_siege_30k
```

The verified deterministic game score is:

```text
8490
```

## Benchmark rules

- All six versions must print the same final score.
- Game logic stays integer-only for reproducibility.
- No networking, filesystem, random device, wall clock, or external assets.
- Build and runtime results belong in a generated report, never in this README.
