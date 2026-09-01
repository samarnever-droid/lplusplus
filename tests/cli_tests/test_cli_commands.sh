#!/usr/bin/env bash
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

# Ensure package directory parsing doesn't swallow file arguments
mkdir -p "$TEMP/mypkg/src"
cat > "$TEMP/mypkg/lpp.toml" <<'MANIFEST'
[package]
name = "mypkg"
version = "0.1.0"
MANIFEST
cat > "$TEMP/mypkg/src/main.lpp" <<'SRC'
def main():
    print(8)
SRC

# Verify package is recognized
"$LPP" run "$TEMP/mypkg" >/dev/null
echo "PASS PM run"

# Verify file doesn't trigger package logic
cat > "$TEMP/single.lpp" <<'SRC'
def main():
    print(9)
SRC

"$LPP" run "$TEMP/single.lpp" >/dev/null
echo "PASS source run"

# Verify build command on package
"$LPP" build "$TEMP/mypkg" >/dev/null
echo "PASS PM build"

# Verify check command on package
"$LPP" check "$TEMP/mypkg" >/dev/null
echo "PASS PM check"

# Verify check command on source
"$LPP" check "$TEMP/single.lpp" >/dev/null
echo "PASS source check"
