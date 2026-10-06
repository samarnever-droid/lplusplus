# Rewrite correctness baseline

**Measured:** 2026-10-06

The rewrite is judged on executable correctness and intended rejection behavior, not on textual parity with the legacy compiler.

## Authored native corpus

The corpus contains every authored `.lpp` program under `tests/` and `examples/`. Generated outputs, dependencies, binaries, and duplicated generated corpora are not included.

### Check gate

```text
total=196  scored=178  skipped=18
  run:    PASS=165  COMPFAIL=0  RUNFAIL=0  ASSERTFAIL=0  TIMEOUT=0
  reject: XFAIL-OK=13  REJECT-BAD=0
correct: 178/178 = 100.0%
gate: PASS
```

### Compile, link, and execute gate

```text
total=196  scored=178  skipped=18
  run:    PASS=165  COMPFAIL=0  RUNFAIL=0  ASSERTFAIL=0  TIMEOUT=0
  reject: XFAIL-OK=13  REJECT-BAD=0
correct: 178/178 = 100.0%
gate: PASS
```

The denominator excludes 18 files that are not standalone native/headless programs: library modules without `main`, GUI/display programs, and wasm-target rejection fixtures. The 13 negative programs count as correct only when the compiler emits a compile-stage diagnostic; a timeout, linker failure, runtime failure, or unrelated nonzero status does not count as a valid rejection.

Run the gate with:

```bash
cargo build -p lpp -p lpp-runtime
bash scripts/corpus_correctness.sh check
bash scripts/corpus_correctness.sh run
```

`scripts/corpus_correctness.sh` exits nonzero for every incorrect scored outcome, records complete failure diagnostics, distinguishes uncoded compile failures from runtime failures, validates its mode, resolves `lpp.exe` on Windows, and cleans both Unix and Windows executable names.

The executable corpus is required in CI on:

- Linux x86-64;
- Windows x86-64 through the MSVC host linker;
- macOS arm64;
- macOS x86-64.

Only Linux x86-64 was executed in the current local environment. The other host runs are implemented in the workflow but remain pending until GitHub Actions executes this change.

## WebAssembly corpus

The permanent rewrite WASM driver gate remains green:

- 24/24 positive legacy corpus programs compile;
- every emitted module validates and executes;
- stdout matches exactly;
- WASI input-line behavior passes;
- intentional host/SIMD/FFI capability rejections remain enforced.

Run it with:

```bash
cargo test -p lpp-driver --test rewrite_wasm_corpus_gate
```

## Native host-format and deployment coverage

The stable v0.1 native backend is Cranelift. Native target triples and objects now follow the host operating system:

| Host | Object/executable path | Runtime path | Current evidence |
| --- | --- | --- | --- |
| Linux x86-64 | ELF via host `cc` or the Linux-only direct linker | `liblpp_runtime.so` | Local corpus check/run 178/178; clean installed-layout gate 1/1 |
| Windows x86-64 | COFF `.obj` linked by MSVC `cl.exe`; `.exe` output | import `.lib` plus copied `lpp_runtime.dll` | Cross-target Rust/test compilation passes; Actions execution pending |
| macOS arm64 | Mach-O through host `cc`; ABI-prefixed external symbols | `liblpp_runtime.dylib` plus rpath | Cross-target Rust/test compilation passes; Actions execution pending |
| macOS x86-64 | Mach-O through host `cc`; ABI-prefixed external symbols | `liblpp_runtime.dylib` plus rpath | Cross-target Rust/test compilation passes; Actions execution pending |

The release workflow builds four platform archives, packages the corresponding rewrite runtime, and runs an installed-layout source-to-executable smoke before creating each archive. Those release jobs are defined but have not yet run on this change.

Cross-target `cargo check --tests` passed for:

```text
x86_64-pc-windows-msvc
x86_64-apple-darwin
aarch64-apple-darwin
```

This establishes conditional-code compilation, not linker/loader or runtime behavior.

## LLVM evidence boundary

LLVM remains experimental and opt-in for v0.1. Compiler-free gates cover its defined textual lowering and typed rejections. External object/execution and cross-backend tests run when clang is available; six such local checks reported environment skips because this sandbox has no clang.

Linux CI and release preflight execute `clang --version` before the complete workspace suite. A missing compiler therefore fails release validation instead of turning those conditional gates into release evidence. See [`LLVM_SUPPORT.md`](LLVM_SUPPORT.md) for the supported and unsupported LLVM surface.

## Regression evidence

The following evidence is green after the latest cutover work:

- complete Rust workspace suite — pass locally, with the six clang-dependent checks explicitly environment-skipped as described above;
- native authored corpus check — 178/178 correct;
- native authored corpus compile/link/execute — 178/178 correct;
- rewrite WASM corpus — 24/24 positive programs with exact stdout, plus capability-rejection checks;
- rewrite WASM driver groups — 3/3;
- ownership crate — 48/48;
- linker — 32/32;
- Keel — 74/74;
- `lpp-pm` — 42/42;
- rewrite config/setup/dev/retirement subprocess gate — 1/1;
- installed Linux rewrite runtime gate — 1/1;
- package-release Python tests — 3/3;
- Windows and both macOS cross-target test compilation — pass;
- workflow YAML parsing, formatting, and `git diff --check` — pass.

## Closed correctness gaps

The rewrite now includes the earlier corpus repairs and the cutover-specific host/deployment repairs:

1. integer truthiness verifies consistently with native and WASM lowering;
2. contextual fixed-width literals are range-checked and stored in the destination machine representation;
3. literal zero is the narrowly scoped legacy null sentinel for recursive nominal constructor fields;
4. MIR materializes transitive nominal aggregate descriptors used behind containers;
5. runtime discovery works from source-tree, environment override, and installed sibling `bin/` + `lib/` layouts;
6. native object formats follow the host instead of always emitting Linux ELF;
7. Mach-O external names use the required platform ABI prefix;
8. Windows uses `.obj`/`.exe`, resolves the runtime import library, and deploys the runtime DLL beside the generated executable;
9. foreign operating-system target spellings are rejected instead of silently producing a different format;
10. non-host objects cannot enter build/run linking accidentally.

## What 100% does and does not mean

The 100% figures above are **corpus compatibility measurements**, not whole-product cutover completion. They establish that the current authored native corpus and defined WASM corpus are green in their measured environments. They do not establish universal language completeness, all-platform parity, perfect memory safety, or completed release operations.

The process still defaults to `LegacyEngine` by design. The rewrite remains selected with `LPP_ENGINE=rewrite` until the Windows/macOS executable corpus, all four installed release archives, required LLVM Linux gates, and final ownership/runtime deployment review pass on the reviewed commit. See [`CUTOVER_READINESS.md`](CUTOVER_READINESS.md) for the weighted assessment and exit criteria.
