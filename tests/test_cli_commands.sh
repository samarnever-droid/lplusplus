#!/usr/bin/env sh
# Verify the package/source command split stays unambiguous.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
fi

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir "$TEMP/pkg"
cat > "$TEMP/pkg/lpp.toml" <<'INNER_EOF'
[package]
name = "pkg"
version = "0.1.0"
INNER_EOF

mkdir "$TEMP/pkg/src"
cat > "$TEMP/pkg/src/main.lpp" <<'INNER_EOF'
def main():
    print(7)
INNER_EOF

# Test 1: lpp check on package directory
echo "Test 1"
(cd "$TEMP" && "$LPP" check pkg >/dev/null)
echo "PASS 1"

# Test 2: lpp run on package directory
echo "Test 2"
(cd "$TEMP" && LPP_EMULATOR=1 "$LPP" run pkg >/dev/null)
echo "PASS 2"

# Test 3: lpp check on specific file without .lpp (e.g. symlink or weird name)
echo "Test 3"
cat > "$TEMP/testfile" <<'INNER_EOF'
def main():
    print(8)
INNER_EOF
(cd "$TEMP" && "$LPP" check testfile >/dev/null)
echo "PASS 3"

# Test 4: lpp run on specific file without .lpp
echo "Test 4"
(cd "$TEMP" && LPP_EMULATOR=1 "$LPP" run testfile >/dev/null)
echo "PASS 4"

# Test 5: lpp emit on specific file
echo "Test 5"
(cd "$TEMP" && "$LPP" emit testfile >/dev/null)
[ -e "$TEMP/testfile.o" ]
echo "PASS 5"

echo "PASS all 5 CLI tests"
rm -rf "$TEMP"
