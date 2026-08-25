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
echo "PASS source command split"

# Test that directories are routed to the package manager, not treated as source files.
PKG_DIR="$TEMP/dummy_pkg"
mkdir -p "$PKG_DIR/src"
cat > "$PKG_DIR/lpp.toml" <<'INNER'
[package]
name = "dummy_pkg"
version = "0.1.0"
INNER
cat > "$PKG_DIR/src/main.lpp" <<'INNER'
def main():
    print(1)
INNER

# We capture output to verify package manager actions (which usually print "Building project")
OUT_RUN=$(cd "$PKG_DIR" && "$LPP" run 2>&1 || true)
if ! echo "$OUT_RUN" | grep -q "Building"; then
    echo "FAIL: lpp run on package directory did not route to package manager"
    exit 1
fi

OUT_CHECK=$(cd "$PKG_DIR" && "$LPP" check 2>&1 || true)
if ! echo "$OUT_CHECK" | grep -q "Checking"; then
    echo "FAIL: lpp check on package directory did not route to package manager"; echo "$OUT_CHECK"
    exit 1
fi

# Create a source file without .lpp extension
FILE_NO_EXT="$TEMP/source_no_ext"
cat > "$FILE_NO_EXT" <<'INNER'
def main():
    print(42)
INNER

"$LPP" run "$FILE_NO_EXT" >/dev/null
"$LPP" check "$FILE_NO_EXT" >/dev/null
"$LPP" emit "$FILE_NO_EXT" >/dev/null
[ -e "$TEMP/source_no_ext.o" ]

echo "PASS directory vs file split"
