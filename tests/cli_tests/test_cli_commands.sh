#!/usr/bin/env bash
# Verify that source commands vs package commands are routed correctly.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
if [ ! -x "$LPP" ]; then
    LPP="$ROOT/target/debug/lpp"
fi

TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-tests.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

# Create a single source file
cat > "$TEMP/single_file.lpp" <<'EOF'
def main():
    print("from file")
EOF

# Create a package directory
mkdir -p "$TEMP/test_pkg/src"
cat > "$TEMP/test_pkg/lpp.toml" <<'EOF'
[package]
name = "test_pkg"
version = "0.1.0"
entry = "src/main.lpp"
EOF
cat > "$TEMP/test_pkg/src/main.lpp" <<'EOF'
def main():
    print("from package")
EOF

echo "Running tests..."

# 1. 'run' on a single source file
# Should run as a source command, execute, and exit.
OUT=$("$LPP" run "$TEMP/single_file.lpp" | tail -n1)
if [ "$OUT" != "from file" ]; then
    echo "Test 1 failed: Expected 'from file', got '$OUT'"
    exit 1
fi
echo "Test 1 passed"

# 2. 'run' on a package directory
# Should run as a package command. The output is usually via the PM.
OUT=$(cd "$TEMP/test_pkg" && "$LPP" run | tail -n1)
if [ "$OUT" != "from package" ]; then
    echo "Test 2 failed: Expected 'from package', got '$OUT'"
    exit 1
fi
echo "Test 2 passed"

# 3. 'check' on a single source file
# Should run as a source command.
"$LPP" check "$TEMP/single_file.lpp" >/dev/null
echo "Test 3 passed"

# 4. 'check' on a package directory
# Should run as a package command.
(cd "$TEMP/test_pkg" && "$LPP" check >/dev/null)
echo "Test 4 passed"

# 5. 'emit' on a single source file
# Should run as a source command and emit an object file.
"$LPP" emit "$TEMP/single_file.lpp" >/dev/null
if [ ! -f "$TEMP/single_file.o" ] && [ ! -f "$TEMP/single_file.obj" ]; then
    echo "Test 5 failed: object file not emitted for single source file"
    exit 1
fi
echo "Test 5 passed"

echo "All 5 CLI tests passed successfully!"
