#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

echo "Testing CLI commands routing..."

# 1. Package command (run) - shouldn't try to parse a file named "run" or directory
mkdir -p "$TEMP/test_pkg/src"
cat > "$TEMP/test_pkg/lpp.toml" << 'TOML'
[package]
name = "test_pkg"
version = "0.1.0"
TOML
cat > "$TEMP/test_pkg/src/main.lpp" << 'CODE'
def main():
    print(1)
CODE
(cd "$TEMP/test_pkg" && "$LPP" run >/dev/null)
echo "PASS run package"

# 2. Source command with .lpp extension
cat > "$TEMP/example.lpp" << 'CODE'
def main():
    print(2)
CODE
"$LPP" run "$TEMP/example.lpp" >/dev/null
echo "PASS run source"

# 3. Source command with path that is a file without extension
cat > "$TEMP/no_ext" << 'CODE'
def main():
    print(3)
CODE
"$LPP" run "$TEMP/no_ext" >/dev/null
echo "PASS run source without extension"

# 4. Package directory path for run command
(cd "$TEMP" && "$LPP" run test_pkg >/dev/null)
echo "PASS run package by path"

# 5. Package directory path for check command
(cd "$TEMP" && "$LPP" check test_pkg >/dev/null)
echo "PASS check package by path"

# 6. Package directory path for build command
(cd "$TEMP" && "$LPP" build test_pkg >/dev/null)
echo "PASS build package by path"

echo "ALL TESTS PASSED"
