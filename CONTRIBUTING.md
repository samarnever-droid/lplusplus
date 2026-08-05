# Contributing to L++

Thank you for contributing to L++.

L++ is an experimental native language toolchain. The highest priorities are correctness, ownership safety, reproducible benchmarks, and honest documentation.

## Development principles

```text
1. Reject unsupported ownership behavior instead of silently compiling it.
2. Keep Cranelift AOT as the authoritative ownership implementation.
3. Add a regression test for every compiler/runtime bug fixed.
4. Do not commit generated benchmark inputs, binaries, objects, or node_modules.
5. Keep README and Doc.md aligned with verified behavior.
```

## Before opening a pull request

Run the relevant checks:

```bash
cargo test
sh tests/run_aot_parity.sh
sh tests/test_lpp_link_elf.sh
sh tests/test_lpp_link_negative.sh
sh tests/test_packaged_runtime.sh
sh tests/test_direct_installed_build.sh
sh tests/test_source_commands.sh
```

For website work:

```bash
cd website
npm ci
npm run build
```

## Ownership changes

Any change that affects ARC, MIR, destructors, closures, aliases, lists, or returns must document:

```text
AllocateArc / Borrow / Move / Retain / Release / ReturnOwned impact
new ownership edge
new destructor behavior
new positive regression
new negative regression where applicable
```

## Benchmark changes

- **King20 Stable v1 is frozen.** Do not edit its manifest or expected output.
- Put new experimental workloads in King20 Experimental.
- Generated 10k/50k/100k benchmark inputs are ignored and regenerated on demand.
- Report single-run metrics as development measurements, not universal claims.

## Linker changes

- Linux direct ELF changes belong under the native linker roadmap.
- Windows work must preserve COFF/MSVC fallback until direct PE output is verified.
- Do not claim direct-link support for an unsupported runtime import or platform.

## Pull request scope

Keep PRs focused:

```text
one ownership fix
one linker slice
one platform slice
one website/docs slice
one benchmark suite change
```

Explain the verified boundary and any intentionally unsupported behavior in the PR description.
