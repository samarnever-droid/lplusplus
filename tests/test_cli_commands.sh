#!/usr/bin/env sh
# Verify package/source command split and core PM commands.

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

mkdir -p "$TEMP/test_pkg/src"
cat > "$TEMP/test_pkg/lpp.toml" <<'EOF'
[package]
name = "test_pkg"
version = "0.1.0"
EOF

cat > "$TEMP/test_pkg/src/main.lpp" <<'EOF'
def main():
    print(42)
EOF

cd "$TEMP"

if [ "${LPP_EMULATOR:-0}" = "1" ]; then
    export LPP_EMULATOR=1
fi

echo "Test 1: lpp run <pkg_dir>"
LPP_EMULATOR=1 "$LPP" run test_pkg > out1.txt 2>&1 || true
if ! grep -q "42" out1.txt; then
    echo "FAIL: Test 1"
    cat out1.txt
    exit 1
fi
echo "PASS: Test 1"

echo "Test 2: lpp check <pkg_dir>"
"$LPP" check test_pkg > out2.txt 2>&1 || true
if grep -q "No input file specified" out2.txt || grep -q "Failed to read" out2.txt; then
    echo "FAIL: Test 2"
    cat out2.txt
    exit 1
fi
echo "PASS: Test 2"

cat > example.lpp <<'EOF'
def main():
    print(7)
EOF

echo "Test 3: lpp emit <file.lpp>"
"$LPP" emit example.lpp > out3.txt 2>&1 || true
if [ ! -f example.o ] && [ ! -f example.obj ]; then
    echo "FAIL: Test 3"
    cat out3.txt
    exit 1
fi
echo "PASS: Test 3"

echo "Test 4: lpp run <file.lpp>"
LPP_EMULATOR=1 "$LPP" run example.lpp > out4.txt 2>&1 || true
if ! grep -q "7" out4.txt; then
    echo "FAIL: Test 4"
    cat out4.txt
    exit 1
fi
echo "PASS: Test 4"

echo "Test 5: lpp check <file.lpp>"
"$LPP" check example.lpp > out5.txt 2>&1 || true
if grep -q "No input file specified" out5.txt; then
    echo "FAIL: Test 5"
    cat out5.txt
    exit 1
fi
echo "PASS: Test 5"

echo "All 5 CLI tests passed"
