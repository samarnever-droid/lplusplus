#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

FAIL=0

# 1. Package dir 'run'
mkdir "$TEMP/myapp"
cat > "$TEMP/myapp/lpp.toml" <<'TOML'
[package]
name = "myapp"
version = "0.1.0"
TOML
mkdir "$TEMP/myapp/src"
cat > "$TEMP/myapp/src/main.lpp" <<'LPP'
def main():
    print(42)
LPP

OUTPUT=$("$LPP" run "$TEMP/myapp" 2>&1 || true)
if echo "$OUTPUT" | grep -q "Is a directory"; then
    echo "FAIL: 'run <dir>' did not route to package manager."
    FAIL=1
fi

# 2. Package dir 'check'
OUTPUT=$("$LPP" check "$TEMP/myapp" 2>&1 || true)
if echo "$OUTPUT" | grep -q "Is a directory"; then
    echo "FAIL: 'check <dir>' did not route to package manager."
    FAIL=1
fi

# 3. Source file 'run'
cat > "$TEMP/mysource.lpp" <<'LPP'
def main():
    print(100)
LPP
OUTPUT=$("$LPP" run "$TEMP/mysource.lpp" 2>&1 || true)
if ! echo "$OUTPUT" | grep -q "100"; then
    echo "FAIL: 'run <file>' did not execute source file. Output: $OUTPUT"
    FAIL=1
fi

# 4. Source file 'check'
OUTPUT=$("$LPP" check "$TEMP/mysource.lpp" 2>&1 || true)
if ! echo "$OUTPUT" | grep -q "L++ check: OK"; then
    echo "FAIL: 'check <file>' did not execute source file check. Output: $OUTPUT"
    FAIL=1
fi

# 5. Source file 'emit'
OUTPUT=$("$LPP" emit "$TEMP/mysource.lpp" 2>&1 || true)
if [ ! -e "$TEMP/mysource.o" ]; then
    echo "FAIL: 'emit <file>' did not produce output."
    FAIL=1
fi

if [ "$FAIL" -eq 1 ]; then
    echo "SOME TESTS FAILED"
    exit 1
else
    echo "PASS cli tests"
fi
