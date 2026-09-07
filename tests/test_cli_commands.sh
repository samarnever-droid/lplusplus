#!/usr/bin/env bash
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-test.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

# Setup mock single file source
cat > "$TEMP/source_file.lpp" <<OUT
def main():
    print(1)
OUT

# Setup mock package directory
mkdir -p "$TEMP/mock_pkg/src"
cat > "$TEMP/mock_pkg/lpp.toml" <<OUT
[package]
name = "mock_pkg"
version = "0.1.0"
OUT

cat > "$TEMP/mock_pkg/src/main.lpp" <<OUT
def main():
    print(2)
OUT

echo "Running 5 CLI tests..."

# 1. lpp run <file.lpp> routes to source run (should create executable in temp, not use pm)
"$LPP" run "$TEMP/source_file.lpp" > "$TEMP/run_file.out" 2>&1 || true
if ! grep -q "1" "$TEMP/run_file.out"; then
    echo "FAIL: lpp run <file.lpp> did not execute correctly"
    cat "$TEMP/run_file.out"
    exit 1
fi
echo "PASS: lpp run <file.lpp>"

# 2. lpp run <pkg_dir> routes to PM run
"$LPP" run "$TEMP/mock_pkg" > "$TEMP/run_pkg.out" 2>&1 || true
if ! grep -q "2" "$TEMP/run_pkg.out"; then
    echo "FAIL: lpp run <pkg_dir> did not execute correctly"
    cat "$TEMP/run_pkg.out"
    exit 1
fi
echo "PASS: lpp run <pkg_dir>"

# 3. lpp check <file.lpp> routes to source check
"$LPP" check "$TEMP/source_file.lpp" > "$TEMP/check_file.out" 2>&1 || true
if grep -q "L++ Package Manager" "$TEMP/check_file.out"; then
    echo "FAIL: lpp check <file.lpp> routed to PM"
    cat "$TEMP/check_file.out"
    exit 1
fi
echo "PASS: lpp check <file.lpp>"

# 4. lpp check <pkg_dir> routes to PM check
"$LPP" check "$TEMP/mock_pkg" > "$TEMP/check_pkg.out" 2>&1 || true
if grep -q "Expected EOF" "$TEMP/check_pkg.out" || grep -q "error" "$TEMP/check_pkg.out"; then
   echo "FAIL: lpp check <pkg_dir> failed"
   cat "$TEMP/check_pkg.out"
   exit 1
fi
echo "PASS: lpp check <pkg_dir>"

# 5. lpp emit <file.lpp> routes to source emit
"$LPP" emit "$TEMP/source_file.lpp" > "$TEMP/emit_file.out" 2>&1 || true
if [ ! -e "$TEMP/source_file.o" ]; then
    echo "FAIL: lpp emit <file.lpp> did not produce .o file"
    exit 1
fi
echo "PASS: lpp emit <file.lpp>"

echo "ALL 5 CLI TESTS PASSED"
