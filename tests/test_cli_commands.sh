#!/bin/bash
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-tests.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

echo "def main():" > "$TEMP/test_cli.lpp"
echo "    print(1)" >> "$TEMP/test_cli.lpp"

cd "$TEMP"

echo "Running Test 1: lpp emit file.lpp"
"$LPP" emit test_cli.lpp >/dev/null
[ -e "test_cli.o" ]

echo "Running Test 2: lpp emit --target x86_64-linux-android file.lpp"
"$LPP" emit --target x86_64-linux-android test_cli.lpp >/dev/null
[ -e "test_cli.o" ]

echo "Running Test 3: lpp check --backend wasm file.lpp"
"$LPP" check --backend wasm test_cli.lpp >/dev/null

echo "Running Test 4: lpp run --linker host file.lpp"
"$LPP" run --linker host test_cli.lpp >/dev/null

echo "Running Test 5: lpp emit file.lpp -o custom_output.o"
"$LPP" emit test_cli.lpp -o custom_output.o >/dev/null
[ -e "custom_output.o" ]

echo "All 5 CLI tests passed successfully!"
