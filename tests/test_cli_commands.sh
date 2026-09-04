#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

echo "Running L++ CLI tests..."

# 1. Version command
"$LPP" --version | grep "L++ Compiler" >/dev/null
echo "Test 1 passed (version)"

# 2. Help command
"$LPP" --help | grep "Usage" >/dev/null || "$LPP" --help | grep "Options:" >/dev/null
echo "Test 2 passed (help)"

# 3. Create a test program
cat > "$TEMP/main.lpp" <<'INNER_EOF'
def main():
    print(42)
INNER_EOF

# 4. Check command
"$LPP" check "$TEMP/main.lpp" >/dev/null
echo "Test 3 passed (check source)"

# 5. Compile and run single file
"$LPP" run "$TEMP/main.lpp" | grep "42" >/dev/null
echo "Test 4 passed (run source)"

# 6. Invalid command (should error or fail gracefully)
if "$LPP" unknown_command >/dev/null 2>&1; then
    echo "Test 5 failed: unknown_command succeeded"
    exit 1
else
    echo "Test 5 passed (unknown_command failed gracefully)"
fi

echo "All tests passed!"
cleanup
