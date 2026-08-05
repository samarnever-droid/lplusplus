## Summary

<!-- What changed and why? Keep this concise. -->

## Change type

- [ ] Compiler frontend / type system
- [ ] Ownership / ARC / MIR
- [ ] Cranelift / native linker
- [ ] C compatibility backend
- [ ] Runtime
- [ ] Windows / platform support
- [ ] Website / documentation
- [ ] Benchmark / CI

## Verification

- [ ] `cargo test`
- [ ] Relevant linker or runtime integration test
- [ ] `sh tests/run_aot_parity.sh` when behavior is shared by C/AOT
- [ ] Website build, if website files changed
- [ ] Windows CI path considered, if native-linker files changed

## Ownership checklist

<!-- Required for ARC, MIR, structs, closures, lists, aliases, and returns. -->

- [ ] No new unpaired ownership edge was introduced.
- [ ] `Borrow`, `Move`, `Retain`, `Release`, and `ReturnOwned` behavior is documented where relevant.
- [ ] Destructor behavior is documented where relevant.
- [ ] Regression coverage was added or updated.

## Benchmark / generated artifact checklist

- [ ] King20 Stable was not modified.
- [ ] Generated artifacts were not committed.
- [ ] Performance claims include environment and methodology.

## Documentation

- [ ] README updated if user-facing behavior changed.
- [ ] Doc.md or roadmap updated if capability boundaries changed.

## Notes for reviewers

<!-- Explicitly list unsupported cases, tradeoffs, or follow-up work. -->
