# Getting Started

This page walks through installing L++, compiling a first program, creating a package project, and understanding which linker is used.

## Install from a pinned source release

Linux/macOS:

```bash
git clone --depth 1 --branch v0.1 https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
LPP_FROM_SOURCE=1 sh install.sh
export PATH="$HOME/.lpp/bin:$PATH"
lpp --version
```

Windows PowerShell:

```powershell
git clone --depth 1 --branch v0.1 https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
$env:LPP_FROM_SOURCE = "1"
.\install.ps1
lpp --version
```

Do not pipe a downloaded installer directly into a shell. Prebuilt archives and
the matching `SHA256SUMS` manifest are available on the GitHub Releases page;
the installers reject missing or mismatched checksums.

## Development build from source

```bash
git clone https://github.com/samarnever-droid/lplusplus.git
cd lplusplus
cargo build --release --locked --bin lpp --bin lpp-link
./target/release/lpp --version
```

The default build includes the host Cranelift ISA. Add `--features all-arch` when
building a compiler that must emit native objects for other architectures.

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


## Linux install troubleshooting

Release Linux binaries are intended to be static/musl-friendly so they work in small Alpine-like environments.

If the shell says `lpp: not found` after install:

1. Check PATH:

```bash
echo "$PATH"
ls -l "$HOME/.lpp/bin"
```

2. Add L++ to PATH:

```bash
export PATH="$HOME/.lpp/bin:$PATH"
```

3. Confirm architecture:

```bash
uname -m
file "$HOME/.lpp/bin/lpp"
```
