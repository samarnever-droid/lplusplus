#!/usr/bin/env sh
# Verify core PM commands and routing
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
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

echo "Testing basic package initialization..."
cd "$TEMP"
"$LPP" init my_pkg
[ -f lpp.toml ]
[ -f src/main.lpp ]

echo "Testing package run command..."
OUT=$(LPP_EMULATOR=1 "$LPP" run 2>&1)
echo "$OUT" | grep -q "Hello" || { echo "Failed to run package"; exit 1; }

echo "Testing package check command..."
"$LPP" check >/dev/null

echo "Testing lpp version..."
"$LPP" version >/dev/null

echo "Testing lpp help..."
"$LPP" help >/dev/null

echo "PASS CLI commands"
