#!/usr/bin/env bash
# Verify core package manager CLI commands and command routing.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

export LPP_EMULATOR=1

cd "$TEMP"
"$LPP" new test_pkg >/dev/null
if [ ! -d "test_pkg/src" ] || [ ! -f "test_pkg/lpp.toml" ] || [ ! -f "test_pkg/src/main.lpp" ]; then
    echo "FAIL: lpp new did not create expected files"
    return 1
fi

"$LPP" check test_pkg >/dev/null

OUTPUT=$("$LPP" run test_pkg)
if ! echo "$OUTPUT" | grep -q "Hello, world!"; then
    echo "FAIL: lpp run output was: $OUTPUT"
    return 1
fi

OUTPUT=$("$LPP" test_pkg)
if ! echo "$OUTPUT" | grep -q "Hello, world!"; then
    echo "FAIL: lpp pkg_dir output was: $OUTPUT"
    return 1
fi

if "$LPP" check non_existent_pkg 2>/dev/null; then
    echo "FAIL: expected check on non-existent package to fail"
    return 1
fi

echo "PASS cli command split"
