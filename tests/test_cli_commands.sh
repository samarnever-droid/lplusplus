#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
fi
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir -p "$TEMP/my_pkg"
cd "$TEMP/my_pkg"

cat > "lpp.toml" <<'TOML'
[package]
name = "my_pkg"
version = "0.1.0"
TOML

mkdir -p "src"
cat > "src/main.lpp" <<'LPP'
def main():
    print(42)
LPP

FAIL_COUNT=0

# 1. Package dir target: lpp run <pkg_dir>
"$LPP" run "$TEMP/my_pkg" > "$TEMP/run_pkg.out" 2>&1 || true
if ! grep -q "42" "$TEMP/run_pkg.out"; then
    echo "FAIL: lpp run pkg_dir failed. Output:"
    cat "$TEMP/run_pkg.out"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# 2. Package dir target: lpp check <pkg_dir>
"$LPP" check "$TEMP/my_pkg" > "$TEMP/check_pkg.out" 2>&1 || true
if ! grep -q "check: OK" "$TEMP/check_pkg.out"; then
    echo "FAIL: lpp check pkg_dir failed. Output:"
    cat "$TEMP/check_pkg.out"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# 3. Source target: lpp run <src_file>
"$LPP" run "$TEMP/my_pkg/src/main.lpp" > "$TEMP/run_src.out" 2>&1 || true
if ! grep -q "42" "$TEMP/run_src.out"; then
    echo "FAIL: lpp run src_file failed. Output:"
    cat "$TEMP/run_src.out"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# 4. Source target: lpp check <src_file>
"$LPP" check "$TEMP/my_pkg/src/main.lpp" > "$TEMP/check_src.out" 2>&1 || true
if ! grep -q "OK" "$TEMP/check_src.out"; then
    echo "FAIL: lpp check src_file failed. Output:"
    cat "$TEMP/check_src.out"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# 5. Emit command: lpp emit <src_file>
"$LPP" emit "$TEMP/my_pkg/src/main.lpp" > "$TEMP/emit.out" 2>&1 || true
if [ ! -e "$TEMP/my_pkg/src/main.o" ] && [ ! -e "$TEMP/my_pkg/src/main.obj" ]; then
    echo "FAIL: lpp emit src_file failed to generate object file. Output:"
    cat "$TEMP/emit.out"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

if [ "$FAIL_COUNT" -gt 0 ]; then
    echo "Tests failed: $FAIL_COUNT"
    exit 1
else
    echo "PASS cli commands tests"
fi
