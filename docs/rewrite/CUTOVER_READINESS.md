# Rewrite cutover readiness

**Assessment date:** 2026-10-06

## Executive answer

- **Native authored-corpus compatibility on the locally executed Linux host:** 178/178 correct (**100%**).
- **Defined rewrite WASM corpus:** 24/24 compile, validate, execute, and match stdout (**100%**).
- **Whole-product replacement readiness:** approximately **94%**, with an honest planning range of **92–95%**.
- **Default engine:** still `LegacyEngine`; the rewrite remains opt-in with `LPP_ENGINE=rewrite` until the new cross-host and packaged-release jobs pass on the reviewed commit.

The 94% figure is a weighted engineering estimate, not a test-coverage percentage. The implementation now contains the mechanisms needed to reach the requested 95% cutover threshold, but one point remains deliberately uncredited until GitHub Actions executes the Windows, macOS arm64, macOS x86-64, and installed-release paths. A green authored corpus on one machine is not a substitute for that evidence.

## Evidence-backed scorecard

| Subsystem | Weight | Current evidence | Credited readiness |
| --- | ---: | --- | ---: |
| Front end, typed HIR, MIR, verification, and optimization pipeline | 22 | All 178 scored authored native cases reach their intended accept/reject result; the complete local workspace suite passes | 22 |
| Native Cranelift and runtime behavior | 22 | Linux x86-64: 165/165 positive programs compile, link, execute, and satisfy assertion markers; 13/13 negatives reject. Host-format ELF/Mach-O/COFF generation and host-specific ABI naming are implemented; external execution is pending on Windows and both macOS architectures | 21 |
| Direct WASM backend in its defined capability boundary | 14 | 24/24 exact-output corpus; validation, multiline input, ARC suites, and host-capability rejection gates pass | 14 |
| Ownership planning, balance proofs, and deployment behavior | 10 | 48/48 ownership tests pass; proofs run before and after optimization; executable managed-data corpora pass locally. Cross-host runtime/memory-tool evidence remains pending | 9 |
| Keel and `lpp-pm` | 10 | 116/116 combined tests pass. Project/package routes are implemented through Keel, and the rewrite no longer carries a named whole-command legacy fallback list | 10 |
| Linker, runtime packaging, and cross-host execution | 8 | 32/32 linker tests pass; clean Linux `bin/` + `lib/` installation executes; Windows MSVC/DLL and macOS dylib/rpath paths are implemented; Windows and macOS test targets compile-check locally | 7 |
| LLVM backend | 5 | The implemented scalar/callable subset has textual gates, configurable setup, and an explicit experimental v0.1 policy. External object/execution gates require clang in Linux CI; managed aggregates/closures are intentionally outside this tier | 3 |
| CLI/toolchain compatibility and default routing | 6 | Every historical named route is implemented, mapped to Keel, or explicitly retired. LLVM setup aliases, `dev`, Lreact compatibility, config, update, benchmark, selectors, and rollback selection are gated | 6 |
| Release-matrix and operational cutover | 3 | Required preflight and four release archives are defined; each archive packages the rewrite runtime and has installed-layout execution smoke logic. CI also defines full authored-corpus execution on Windows, macOS arm64, and macOS x86-64, but those jobs have not yet run on this change | 2 |
| **Total** | **100** |  | **94** |

A successful run of the new cross-host CI/release matrix would supply the missing evidence for approximately **95%**. It would not by itself justify claiming 100% or silently changing the default; the reviewed commit and rollback plan must still be accepted first.

## Implementation completed in this cutover pass

1. `setup llvm [path]` and `toolchain install llvm [path]` now locate, probe, and persist the LLVM compiler through shared `lpp-config`.
2. LLVM support is explicitly experimental and opt-in for L++ v0.1; stable release guarantees remain on Cranelift.
3. `dev` maps to Keel `run`; plaintext-token `login` is retired because registry authentication uses Git credentials.
4. `lreact help/dev/run/build` have defined compatibility behavior, while the bundled legacy scaffold is explicitly retired.
5. No named whole-command legacy fallback list remains.
6. Keel discovers the configured, active, sibling, or profile-level `lpp` executable for embedded project builds rather than assuming `lpp` is on `PATH`.
7. Cranelift native triples now follow the build host: Linux ELF, macOS Mach-O, or Windows COFF, for x86-64 and AArch64.
8. Mach-O external imports and the C entry symbol receive the required platform symbol prefix.
9. Rewrite linking supports Unix host `cc` and Windows MSVC `cl.exe`; Windows runtime import libraries and DLL deployment are discovered automatically.
10. Temporary/output naming follows host conventions (`.o`, `.obj`, `.exe`), and runtime discovery is centralized across source and installed layouts.
11. Host triple selection no longer accepts a foreign operating-system spelling and then emits a different object format; build/run also reject non-host objects before linking.
12. CI defines full authored-corpus execution on Windows, macOS arm64, and macOS x86-64, plus focused native compile-link-run smokes.
13. Release preflight requires formatting, the workspace suite, clang availability, native corpus check/run, and the clean installed-layout gate.
14. Linux x86-64, Windows x86-64, macOS arm64, and macOS x86-64 archives package the Rust rewrite runtime and execute a program from the packaged layout before archive creation.
15. Native object tests now assert the host format, while Windows-only C-link execution remains covered by the explicit MSVC driver smoke rather than an unconfigured test process.

## Current validation evidence

### Green locally

- `cargo test --workspace --locked` — pass. Six external LLVM object/cross-backend checks reported environment skips because this sandbox has no clang; textual LLVM tests still ran. Linux CI and release preflight explicitly require `clang --version`, so that skip cannot silently satisfy release validation.
- Native authored corpus `check` — **178/178**.
- Native authored corpus `run` — **178/178**: positives **165/165**, intended negatives **13/13**, exclusions **18**.
- Rewrite WASM corpus — **24/24**, plus rewrite driver **3/3**.
- Ownership — **48/48**.
- Linker — **32/32**.
- Keel — **74/74**; `lpp-pm` — **42/42**.
- Rewrite config/setup/dev/retirement subprocess gate — **1/1**.
- Clean installed Linux `bin/` + `lib/` runtime gate — **1/1**.
- Cross-target `cargo check --tests` for `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, and `aarch64-apple-darwin` — pass for the rewrite/compiler/test packages.
- Deterministic package-release Python tests — **3/3**.
- All workflow YAML parses, `cargo fmt --all -- --check`, and `git diff --check` — pass.

### Still external and pending

- Actual Windows COFF generation, MSVC import-library linking, DLL copying/loading, and 178-case corpus execution.
- Actual macOS arm64 and x86-64 Mach-O linking, dylib loading, and 178-case corpus execution.
- Creation and installed execution of all four final release archives.
- Required GitHub checks on the exact reviewed commit.

Cross-target compilation proves the conditional Rust code type-checks for those targets. It does **not** prove the platform linker, loader, archive layout, or executable behavior; those claims remain pending until Actions runs them.

## Remaining cutover blockers

### Must finish before changing the default engine

1. Run the complete required GitHub Actions matrix on the reviewed commit and investigate any host-specific failure rather than waiving it.
2. Require green authored-corpus execution on Linux x86-64, Windows x86-64, macOS arm64, and macOS x86-64.
3. Require green installed-layout execution for all four release archives.
4. Confirm the Linux clang-backed LLVM object/cross-backend gates, while retaining LLVM's experimental v0.1 status.
5. Complete the final ownership/runtime deployment review, including available platform memory tools, without converting the safety-first claim into a universal memory-safety claim.
6. Only then change `src/main.rs` to the rewrite default, keeping `LPP_ENGINE=legacy` as an explicit rollback path for the agreed transition window.

### Release operations still requiring owner action

- Review and commit the implementation/release changes.
- Confirm required CI is green on that commit.
- Approve the default-engine change as a separate, evidence-triggered step.
- Create the intended `v0.1` tag.
- Publish checksummed/attested release artifacts and the final website/marketing media.

## Required interpretation

“100% native corpus” means every currently scored authored corpus case behaves correctly in the tested environment. It does **not** mean universal language completeness, all-platform parity, perfect memory safety, or that the rewrite is already safe to make the default without the remaining cross-host evidence.
