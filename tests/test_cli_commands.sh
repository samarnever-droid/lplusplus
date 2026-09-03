#!/usr/bin/env sh
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

# 1. Package creation (lpp new)
(cd "$TEMP" && "$LPP" new mypkg)
[ -d "$TEMP/mypkg/src" ]
[ -f "$TEMP/mypkg/lpp.toml" ]

# 2. Package build (lpp build)
(cd "$TEMP/mypkg" && "$LPP" build)
if [ ! -f "$TEMP/mypkg/LppData/build/release/mypkg" ] && [ ! -f "$TEMP/mypkg/LppData/build/release/mypkg.exe" ]; then
    echo "Build failed to produce executable"
    exit 1
fi

# 3. Package check (lpp check)
(cd "$TEMP/mypkg" && "$LPP" check)

# 4. Package run (lpp run)
(cd "$TEMP/mypkg" && LPP_EMULATOR=1 "$LPP" run)

# 5. Invalid command
if "$LPP" invalid_cmd 2>/dev/null; then
    echo "FAIL: invalid command should fail"
    exit 1
fi

echo "PASS CLI commands"