#!/usr/bin/env sh
set -u

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-run.XXXXXX")
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM

cd "$TMP"

cat << 'INNEREOF' > script.lpp
def main():
    print(42)
INNEREOF

OUT=$(LPP_EMULATOR=1 "$LPP" run script.lpp | grep 42)
if [ "$OUT" != "42" ]; then
    echo "FAIL: lpp run failed for .lpp extension"
fi

cat << 'INNEREOF' > script
def main():
    print(43)
INNEREOF

OUT=$(LPP_EMULATOR=1 "$LPP" run script | grep 43)
if [ "$OUT" != "43" ]; then
    echo "FAIL: lpp run failed for no extension"
fi

OUT=$(LPP_EMULATOR=1 "$LPP" check script.lpp | grep OK)
if [ "$OUT" != "L++ check: OK" ]; then
    echo "FAIL: lpp check failed for .lpp extension"
fi

OUT=$(LPP_EMULATOR=1 "$LPP" check script | grep OK)
if [ "$OUT" != "L++ check: OK" ]; then
    echo "FAIL: lpp check failed for no extension"
fi

OUT=$(LPP_EMULATOR=1 "$LPP" emit script | grep emitted)
if [ -z "$OUT" ]; then
    echo "FAIL: lpp emit failed for no extension"
fi

echo "PASS 5 lpp cli commands"
