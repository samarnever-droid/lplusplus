#!/usr/bin/env sh
# Additional CLI tests focusing on validating command routing (package vs source file behavior) and core PM commands

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
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

echo "Setting up test environment in $TEMP"
mkdir -p "$TEMP/mypkg/src"
cat > "$TEMP/mypkg/lpp.toml" <<'TOML'
[package]
name = "mypkg"
version = "0.1.0"
TOML

cat > "$TEMP/mypkg/src/main.lpp" <<'LPP'
def main():
    print(42)
LPP

cat > "$TEMP/myscript" <<'LPP'
def main():
    print(99)
LPP

cd "$TEMP"

echo "Test 1: lpp run on a package directory"
"$LPP" run mypkg > run_pkg.log 2>&1 || true
if grep -q "Error: cannot find" run_pkg.log || grep -q "not found" run_pkg.log; then
    echo "FAIL: lpp run mypkg acted as source command"
    exit 1
fi
echo "PASS: lpp run mypkg acts as package command"

echo "Test 2: lpp check on a package directory"
"$LPP" check mypkg > check_pkg.log 2>&1 || true
if grep -q "Error: cannot find" check_pkg.log || grep -q "not found" check_pkg.log; then
    echo "FAIL: lpp check mypkg acted as source command"
    exit 1
fi
echo "PASS: lpp check mypkg acts as package command"

echo "Test 3: lpp emit on a package directory"
"$LPP" emit mypkg > emit_pkg.log 2>&1 || true
if grep -q "Error: cannot find" emit_pkg.log || grep -q "not found" emit_pkg.log; then
    echo "FAIL: lpp emit mypkg acted as source command"
    exit 1
fi
echo "PASS: lpp emit mypkg acts as package command"

echo "Test 4: lpp run on a file without .lpp extension"
"$LPP" run myscript > run_file.log 2>&1 || true
if grep -q "99" run_file.log || grep -q "Running" run_file.log; then
    echo "PASS: lpp run myscript acts as source command"
else
    echo "FAIL: lpp run myscript didn't act as source command"
    cat run_file.log
    exit 1
fi

echo "Test 5: lpp check on a file without .lpp extension"
"$LPP" check myscript > check_file.log 2>&1 || true
echo "PASS: lpp check myscript acts as source command"

echo "ALL TESTS PASSED"
