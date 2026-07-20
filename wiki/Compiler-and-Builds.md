# Compiler and Builds

## Commands

| Command | Meaning |
|---|---|
| `lpp check file.lpp` | Analyze one source file without artifacts |
| `lpp emit file.lpp` | Emit C source explicitly |
| `lpp emit file.lpp --aot` | Emit C plus Cranelift object |
| `lpp build` | Build the package in `lpp.toml` |
| `lpp run` | Build and run the package |

## Link modes

The normal host-link path compiles/links supported runtime components with the platform toolchain. It is the compatibility path for facilities that require richer platform runtimes, including current networking.

On Linux x86-64, the direct ELF linker can be selected with:

```sh
LPP_LINKER=direct lpp build
```

It supports the verified direct subset, not every host-link feature. Unsupported requirements should fail clearly rather than produce a broken executable.

## Tests

```sh
cargo test --locked
sh tests/run_aot_parity.sh
sh tests/test_rust_network_runtime.sh
```

The nested Rust network runtime is tested separately:

```sh
cargo test --manifest-path runtime/lpp-net/Cargo.toml --locked
```

## v0.1.3 current-status note

This page is maintained with the project, but current support claims are
centralized in [Current Capabilities](../documentation/CURRENT_CAPABILITIES.md).

```text
Use LppData/build/release and LppData/cache for package artifacts.
Use host-linked AOT for filesystem/networking work.
Do not assume direct ELF supports files, networking, JSON, or threads.
Do not claim language-wide Rust-equivalent safety outside the verified AOT subset.
```
