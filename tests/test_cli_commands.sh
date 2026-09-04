#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d)
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

cd "$TEMP"
echo "Test 1: new"
"$LPP" new mypkg
cd mypkg

echo "Test 2: check"
"$LPP" check

echo "Test 3: build"
"$LPP" build

echo "Test 4: run"
"$LPP" run

echo "Test 5: test"
mkdir tests
cat > tests/test_dummy.lpp <<EOF
def main():
    print("test pass")
EOF
LPP_EMULATOR=1 "$LPP" test

echo "PASS CLI commands"
