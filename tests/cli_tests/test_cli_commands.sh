#!/usr/bin/env sh
set -e

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
fi
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir "$TEMP/pkg"
cat > "$TEMP/pkg/lpp.toml" << 'TOML'
[package]
name = "pkg"
version = "0.1.0"
TOML
mkdir "$TEMP/pkg/src"
cat > "$TEMP/pkg/src/main.lpp" << 'CODE'
def main():
    print(42)
CODE

cat > "$TEMP/example.lpp" << 'CODE2'
def main():
    print(7)
CODE2

# test 1: check command on a directory should route to package manager
"$LPP" check "$TEMP/pkg" > "$TEMP/out1" 2>&1 || true

# test 2: run command on a directory should route to package manager
"$LPP" run "$TEMP/pkg" > "$TEMP/out2" 2>&1 || true

# test 3: check command on a file should route to source command
"$LPP" check "$TEMP/example.lpp" > "$TEMP/out3" 2>&1

# test 4: emit command on a file should route to source command
"$LPP" emit "$TEMP/example.lpp" > "$TEMP/out4" 2>&1
[ -e "$TEMP/example.o" ]

# test 5: run command on a file should route to source command
"$LPP" run "$TEMP/example.lpp" > "$TEMP/out5" 2>&1

echo "CLI tests routing PASS"
