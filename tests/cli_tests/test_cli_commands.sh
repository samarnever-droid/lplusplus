#!/usr/bin/env sh
# Verify CLI routing: 'run', 'check', and 'emit' against both package directories and single files.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/debug/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    echo "LPP compiler not found at $LPP"
    # return error code
    (exit 1)
fi

# Setup test package
PKG_DIR="$TEMP/test_pkg"
mkdir -p "$PKG_DIR/src"
cat > "$PKG_DIR/lpp.toml" <<TOML
[package]
name = "test_pkg"
version = "0.1.0"
TOML
cat > "$PKG_DIR/src/main.lpp" <<CODE
def main():
    print(123)
CODE

# Setup single file
SRC_FILE="$TEMP/single.lpp"
cat > "$SRC_FILE" <<CODE
def main():
    print(456)
CODE

echo "--- Test 1: lpp run <pkg_dir> ---"
# L++ PM uses the current working directory or absolute paths
cd "$PKG_DIR"
OUT=$("$LPP" run 2>&1)
cd "$ROOT"
if ! echo "$OUT" | grep -q "123"; then
    echo "FAIL: lpp run pkg_dir did not output 123. Got: $OUT"
    (exit 1)
fi
echo "PASS 1"

echo "--- Test 2: lpp check <pkg_dir> ---"
cd "$PKG_DIR"
"$LPP" check >/dev/null
cd "$ROOT"
echo "PASS 2"

echo "--- Test 3: lpp run <file.lpp> ---"
OUT=$("$LPP" run "$SRC_FILE" 2>&1)
if ! echo "$OUT" | grep -q "456"; then
    echo "FAIL: lpp run file.lpp did not output 456. Got: $OUT"
    (exit 1)
fi
echo "PASS 3"

echo "--- Test 4: lpp check <file.lpp> ---"
"$LPP" check "$SRC_FILE" >/dev/null
echo "PASS 4"

echo "--- Test 5: lpp emit <file.lpp> ---"
"$LPP" emit "$SRC_FILE" >/dev/null
OBJ_FILE="${SRC_FILE%.lpp}.o"
if [ ! -f "$OBJ_FILE" ]; then
    echo "FAIL: lpp emit did not generate object file"
    (exit 1)
fi
echo "PASS 5"

echo "All 5 CLI tests passed!"
