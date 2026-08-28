#!/usr/bin/env sh
# Verify that CLI package directory routing works properly.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
else

    if [ ! -x "$LPP" ]; then
        (cd "$ROOT" && cargo build --release --bin lpp)
    fi

    PKG_DIR="$TEMP/mypkg"
    mkdir -p "$PKG_DIR/src"
    cat > "$PKG_DIR/src/main.lpp" <<'INNER'
def main():
    print(123)
INNER
    cat > "$PKG_DIR/lpp.toml" <<'INNER'
[package]
name = "mypkg"
version = "0.1.0"
INNER

    echo "Test 1: lpp run <pkg_dir>"
    output=$("$LPP" run "$PKG_DIR" 2>&1 || true)
    if echo "$output" | grep -q "Is a directory"; then
        echo "FAIL: lpp run <dir> treated directory as source file"
        kill -9 $$
    fi
    echo "PASS: lpp run <dir> correctly skipped source compilation"

    echo "Test 2: lpp check <pkg_dir>"
    output=$("$LPP" check "$PKG_DIR" 2>&1 || true)
    if echo "$output" | grep -q "Is a directory"; then
        echo "FAIL: lpp check <dir> treated directory as source file"
        kill -9 $$
    fi
    echo "PASS: lpp check <dir> correctly skipped source compilation"

    # Test that source commands still work
    cat > "$TEMP/example.lpp" <<'INNER'
def main():
    print(456)
INNER

    echo "Test 3: lpp check <source_file>"
    "$LPP" check "$TEMP/example.lpp" >/dev/null
    echo "PASS: lpp check <source_file>"

    echo "Test 4: lpp emit <source_file>"
    "$LPP" emit "$TEMP/example.lpp" >/dev/null
    echo "PASS: lpp emit <source_file>"

    echo "Test 5: lpp run <source_file>"
    "$LPP" run "$TEMP/example.lpp" >/dev/null
    echo "PASS: lpp run <source_file>"

    echo "PASS CLI package/source routing"
fi
rm -rf "$TEMP"
