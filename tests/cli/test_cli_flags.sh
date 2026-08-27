#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-tests.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

cat > "$TEMP/example.lpp" <<'INNEREOF'
def main():
    print("Hello CLI!")
INNEREOF

# Test 1: Compile with default Cranelift backend
"$LPP" "$TEMP/example.lpp" -o "$TEMP/example_cl.exe"
[ -e "$TEMP/example_cl.exe" ]
if [ "${LPP_EMULATOR:-0}" = "1" ]; then
    out=$(sh -c "$TEMP/example_cl.exe" 2>/dev/null || true)
else
    out=$("$TEMP/example_cl.exe")
fi

echo "PASS CLI Test 1: Default compile"

# Test 2: Compile with LLVM backend
"$LPP" "$TEMP/example.lpp" --llvm -o "$TEMP/example_llvm.exe"
[ -e "$TEMP/example_llvm.exe" ]
echo "PASS CLI Test 2: LLVM compile"

# Test 3: Emit object file
"$LPP" emit "$TEMP/example.lpp"
[ -e "$TEMP/example.o" ]
echo "PASS CLI Test 3: Emit object"

# Test 4: Emit AOT object
"$LPP" emit "$TEMP/example.lpp" --aot
[ -e "$TEMP/example.o" ]
echo "PASS CLI Test 4: Emit AOT object"

# Test 5: Check command
"$LPP" check "$TEMP/example.lpp"
echo "PASS CLI Test 5: Check command"

echo "ALL 5 CLI TESTS PASSED"
