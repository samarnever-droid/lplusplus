#!/usr/bin/env sh
# Verify the package/source command split stays unambiguous.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-source-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi
cat > "$TEMP/example.lpp" <<'EOF'
def main():
    print(7)
EOF

"$LPP" check "$TEMP/example.lpp" >/dev/null
[ ! -e "$TEMP/example.o" ]

"$LPP" emit "$TEMP/example.lpp" >/dev/null
[ -e "$TEMP/example.o" ]

"$LPP" emit "$TEMP/example.lpp" --aot >/dev/null
[ -e "$TEMP/example.o" ]

# Setup a mock package directory
mkdir -p "$TEMP/my_pkg/src"
cat > "$TEMP/my_pkg/lpp.toml" <<'TOML'
[package]
name = "my_pkg"
version = "0.1.0"
TOML
cat > "$TEMP/my_pkg/src/main.lpp" <<'LPP'
def main():
    print("hello from pkg")
LPP

# 1. Test lpp run <pkg_dir> routes to package manager
output=$("$LPP" run "$TEMP/my_pkg" 2>&1 || true)
if echo "$output" | grep -q "Is a directory"; then
    echo "FAIL: lpp run treated package directory as a source file"
    exit 1
fi
echo "PASS lpp run <pkg_dir> correctly routes to PM"

# 2. Test lpp check <pkg_dir> routes to package manager
output=$("$LPP" check "$TEMP/my_pkg" 2>&1 || true)
if echo "$output" | grep -q "Is a directory"; then
    echo "FAIL: lpp check treated package directory as a source file"
    exit 1
fi
echo "PASS lpp check <pkg_dir> correctly routes to PM"

# 3. Test lpp emit <pkg_dir> fails appropriately because it requires a single file
output=$("$LPP" emit "$TEMP/my_pkg" 2>&1 || true)
if ! echo "$output" | grep -q "Is a directory"; then
    echo "FAIL: lpp emit should reject package directories. Output: $output"
    exit 1
fi
echo "PASS lpp emit <pkg_dir> correctly rejected"

# 4. Test lpp check <missing_file.lpp> routes to source checker and fails
output=$("$LPP" check "$TEMP/missing.lpp" 2>&1 || true)
if ! echo "$output" | grep -q "No such file or directory"; then
    echo "FAIL: lpp check missing.lpp should error properly. Output: $output"
    exit 1
fi
echo "PASS lpp check <missing.lpp>"

# 5. Test lpp emit <missing.lpp>
output=$("$LPP" emit "$TEMP/missing.lpp" 2>&1 || true)
if ! echo "$output" | grep -q "Failed to read"; then
    echo "FAIL: lpp emit missing.lpp should error on read. Output: $output"
    exit 1
fi
echo "PASS lpp emit <missing.lpp>"

echo "PASS source command split"
