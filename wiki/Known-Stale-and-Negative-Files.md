# Known historical and negative files

This page is retained because the repository contains both executable tests and
historical notes. It is not a claim that every `.lpp` file in the repository is
a positive compile test.

## Negative tests

Some files intentionally test diagnostics or rejection contracts, such as the
mutable-capture rejection case used by `tests/run_aot_parity.sh`. Read the
individual test comments before treating a non-zero compile result as a bug.

## Historical documents

The following old planning/report files were removed because their present-tense
claims no longer matched the compiler:

```text
ImprovementKiro.md
improvementcodex.md
LPP_Comprehensive_Report.md
```

Use these current documents instead:

- `documentation/STATUS-2026-07-30.md`;
- `documentation/CURRENT_CAPABILITIES.md`;
- `documentation/Compiler_Reality.md`;
- `documentation/Cranelift_Safety_Plan.md`.

## Current validation boundaries

Package tests should run from their package directories. Windows LLVM runtime
execution requires a Windows runner. General automatic vectorization and LLVM
LTO/PGO are not implemented.

## Cleanup policy

New feature claims belong in the current status document and must include a
measured test command. Historical benchmark numbers must be labeled historical
rather than presented as current guarantees.
