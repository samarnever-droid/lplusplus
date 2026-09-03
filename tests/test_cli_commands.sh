#!/usr/bin/env sh
# Verify various CLI source/package command routing and behaviors.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

echo "Setting up files..."
cat > "$TEMP/source1.lpp" <<'INNEREOF'
def main():
    print(1)
INNEREOF

# File without extension
cat > "$TEMP/source2" <<'INNEREOF'
def main():
    print(2)
INNEREOF

# Package directory
mkdir -p "$TEMP/pkg1/src"
cat > "$TEMP/pkg1/lpp.toml" <<'INNEREOF'
[package]
name = "pkg1"
version = "1.0.0"
INNEREOF
cat > "$TEMP/pkg1/src/main.lpp" <<'INNEREOF'
def main():
    print(3)
INNEREOF

echo "Test 1: check command with .lpp file"
"$LPP" check "$TEMP/source1.lpp" >/dev/null

echo "Test 2: run command with .lpp file"
"$LPP" run "$TEMP/source1.lpp" >/dev/null

echo "Test 3: check command with file without .lpp extension"
"$LPP" check "$TEMP/source2" >/dev/null

echo "Test 4: run command with file without .lpp extension"
"$LPP" run "$TEMP/source2" >/dev/null

echo "Test 5: check package directory"
(cd "$TEMP/pkg1" && "$LPP" check >/dev/null)

echo "PASS all CLI commands"
