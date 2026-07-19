# Getting Started

## Install

Release installers are intended to let end users install L++ without installing Rust:

```sh
curl -fsSL https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/install.ps1 | iex
```

For source development, install a current Rust toolchain and build:

```sh
cargo build --release
```

## First program

Create `hello.lpp`:

```lpp
def main():
    print("Hello, L++")
```

Check it without writing artifacts:

```sh
lpp check hello.lpp
```

Emit C explicitly:

```sh
lpp emit hello.lpp
```

Emit C plus Cranelift object output:

```sh
lpp emit hello.lpp --aot
```

## Packages

A package has an `lpp.toml` manifest and source entry point. Use:

```sh
lpp build
lpp run
```

On Linux x86-64, `LPP_LINKER=direct lpp build` selects the direct ELF linker where the package only uses its supported runtime-free/direct-runtime subset.

Do not use legacy `lpp file.lpp` as an implicit build command; it prints guidance to use `check`, `emit`, or `build` explicitly.
