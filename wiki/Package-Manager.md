# Package Manager

L++ includes a self-hosted package manager written in L++ itself.

## Commands

```bash
lpp new myapp          # Create new project
lpp build              # Compile to native binary
lpp run                # Compile and execute
lpp check              # Type-check without compiling
lpp clean              # Remove build artifacts
lpp list               # List dependencies
lpp tree               # Show dependency tree
lpp metadata           # Print package manifest
lpp outdated           # Show unpinned dependencies
```

## Project Layout

```
myapp/
  lpp.toml             # Package manifest
  src/
    main.lpp           # Entry point
  tests/
    test_main.lpp      # Test files
  .lpp_packages/       # Installed dependencies
```

## lpp.toml

```toml
[package]
name = "myapp"
version = "0.1.0"
description = "My L++ application"
authors = ["Your Name"]

[dependencies]
lpp-zip = "0.1.0"
```

## Package Registry

The official registry is hosted on GitHub Pages:

```
https://samarnever-droid.github.io/lplusplus/registry/index.json
```

Available packages: lpp-zip, lpp-math, lpp-strings, lpp-collections, lpp-algo, lpp-convert

## Configuration

First-run config saved to `~/.lpp/config.json`:

```bash
lpp config                       # Show config
lpp config set linker direct     # Use lpp-link (no external compiler)
lpp config set linker host       # Use system cc/cl.exe
lpp config set linker auto       # Auto-detect (default)
```
