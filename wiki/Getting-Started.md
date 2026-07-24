# Getting Started

This page walks through installing L++, compiling a first program, creating a package project, and understanding which linker is used.

## Install from release

Linux/macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/install.sh | sh
export PATH="$HOME/.lpp/bin:$PATH"
lpp --version
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/install.ps1 | iex
lpp --version
```

## Build from source

```bash
git clone https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
cargo build --release --bin lpp --bin lpp-link
./target/release/lpp --version
```

## First program

Create `hello.lpp`:

```lpp
def main():
    print_str("Hello from L++!")
    print(42)
```

Run:

```bash
lpp hello.lpp
```

Or from a source checkout:

```bash
./target/release/lpp hello.lpp
```

## Check without compiling

```bash
lpp --check hello.lpp
```

For a directory of `.lpp` files:

```bash
lpp --checkall
```

## Create a package project

```bash
lpp new myapp
cd myapp
lpp build
lpp run
```

Typical layout:

```text
myapp/
  lpp.toml
  src/
    main.lpp
  tests/
```

## Package commands

```bash
lpp new <name>       # create project
lpp init <name>      # initialize current directory
lpp install          # install dependencies
lpp add <name>       # add dependency
lpp remove <name>    # remove dependency
lpp update           # refresh lockfile
lpp list             # list dependencies
lpp tree             # dependency tree
lpp metadata         # package metadata
lpp outdated         # unpinned dependencies
lpp clean            # remove build output
lpp check            # check package
lpp build            # build native binary
lpp run              # build and run
lpp test             # run tests/
```

## Linker choice

L++ supports two linker paths:

| Linker | Command style | Use case |
|---|---|---|
| Direct linker | `lpp-link` | zero external toolchain, small freestanding binaries |
| Host linker | `cc`, `clang`, `cl.exe` | full libc/CRT compatibility |

Config is stored in `~/.lpp/config.json`:

```bash
lpp config
lpp config set linker direct
lpp config set linker host
lpp config set linker auto
```

Per-run override:

```bash
lpp --linker direct app.lpp
lpp --linker host app.lpp
```
