#!/usr/bin/env sh
# Verify core CLI package commands route correctly.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

cd "$TEMP"
# Create a dummy package to test package commands
"$LPP" new test_pkg >/dev/null
cd test_pkg

# Check that package commands run without interpreting as file commands
# 1. new (done above)
# 2. check
"$LPP" check >/dev/null
# 3. build
"$LPP" build >/dev/null
# 4. run
"$LPP" run >/dev/null
# 5. list
"$LPP" list >/dev/null

[ -f "LppData/build/release/test_pkg" ] || [ -f "LppData/build/release/test_pkg.exe" ]

echo "PASS cli command routing"
