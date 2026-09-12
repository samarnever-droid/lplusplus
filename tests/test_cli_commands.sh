#!/usr/bin/env sh
# Verify CLI command routing for packages vs source files.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
LPP="$ROOT/target/debug/lpp"
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --bin lpp >/dev/null)
fi

TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

cd "$TEMP"

# 1. PM init
"$LPP" init mypkg >/dev/null

# 2. PM run on directory (needs LPP_EMULATOR=1 for host execution)
OUTPUT=$(LPP_EMULATOR=1 LPP_LINKER=host "$LPP" run mypkg 2>/dev/null | tail -1)
if [ "$OUTPUT" != "Hello from L++ project!" ]; then
    echo "Fail PM run on dir: $OUTPUT"
    exit 1
fi

# 3. PM check on directory
LPP_EMULATOR=1 "$LPP" check mypkg >/dev/null

cat > script_no_ext <<'INNER'
def main():
    print(42)
INNER

# 4. Source check on file without .lpp
LPP_EMULATOR=1 "$LPP" check script_no_ext >/dev/null

# 5. Source run on file without .lpp
OUTPUT=$(LPP_EMULATOR=1 LPP_LINKER=host "$LPP" run script_no_ext 2>/dev/null | tail -1)
if [ "$OUTPUT" != "42" ]; then
    echo "Fail source run on file without .lpp: $OUTPUT"
    exit 1
fi

echo "PASS cli commands"
