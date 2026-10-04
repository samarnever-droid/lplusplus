# CI workflows hosted outside `.github/workflows/`

`wasm.yml` belongs at `.github/workflows/wasm.yml`. It lives here because the
automation token used to prepare this change is a GitHub App token without the
`workflows` OAuth scope, which GitHub requires for writing files under
`.github/workflows/` (the automation could not push it there directly).

To enable the WebAssembly backend CI, a maintainer with `workflows` permission
only needs to run:

```bash
git mv ci/wasm.yml .github/workflows/wasm.yml && git commit -m "ci: enable wasm backend workflow" && git push
```

What it runs on every push/PR to master (Linux + macOS):

1. `cargo test --locked` (includes the wasm encoder unit tests)
2. `cargo build --release --bin lpp`
3. `sh tests/run_wasm_tests.sh` under pinned wasmtime — compiles every
   `tests/wasm/cases/*.lpp` to a `.wasm` module, executes it, and diffs the
   stdout against `.expected`; also runs the `tests/wasm/reject/` negative
   cases (clear WebAssembly-specific diagnostics) and triple-alias checks.
