#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir -p "$TEMP/my_pkg/src"
cat > "$TEMP/my_pkg/lpp.toml" <<'TOML'
[package]
name = "my_pkg"
version = "0.1.0"
TOML

cat > "$TEMP/my_pkg/src/main.lpp" <<'CODE'
def main():
    print(42)
CODE

cat > "$TEMP/my_source.lpp" <<'CODE'
def main():
    print(1337)
CODE

# Test 1: run on a source file
OUTPUT=$(LPP_EMULATOR=1 "$LPP" run "$TEMP/my_source.lpp" 2>&1)
if ! echo "$OUTPUT" | grep -q "1337"; then
    echo "FAIL: lpp run on source file"
    exit 1
fi
echo "PASS: lpp run on source file"

# Test 2: run on a package directory (must cd into it for PM to find lpp.toml)
OUTPUT=$(cd "$TEMP/my_pkg" && LPP_EMULATOR=1 "$LPP" run 2>&1)
if ! echo "$OUTPUT" | grep -q "42"; then
    echo "FAIL: lpp run on package directory"
    exit 1
fi
echo "PASS: lpp run on package directory"

# Test 3: check on a source file
LPP_EMULATOR=1 "$LPP" check "$TEMP/my_source.lpp" >/dev/null
echo "PASS: lpp check on source file"

# Test 4: check on a package directory
(cd "$TEMP/my_pkg" && LPP_EMULATOR=1 "$LPP" check >/dev/null)
echo "PASS: lpp check on package directory"

# Test 5: emit on a source file
LPP_EMULATOR=1 "$LPP" emit "$TEMP/my_source.lpp" >/dev/null
if [ ! -f "$TEMP/my_source.o" ] && [ ! -f "$TEMP/my_source.obj" ] && [ ! -f "$TEMP/my_source.wasm" ]; then
    echo "FAIL: lpp emit on source file didn't produce object"
    exit 1
fi
echo "PASS: lpp emit on source file"

echo "All 5 CLI tests passed"
