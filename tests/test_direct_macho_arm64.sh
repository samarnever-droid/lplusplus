#!/usr/bin/env sh
# Apple Silicon macOS rejects static MH_EXECUTE images. Until M3 adds dynamic
# libSystem imports, lpp-link must reject this mode clearly instead of emitting
# a binary that the OS kills with SIGKILL.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
LINKER="$ROOT/target/release/lpp-link"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-macho-arm64.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    echo "SKIP: ARM64 Mach-O policy test requires Apple Silicon macOS"
    exit 0
fi
if [ ! -x "$LPP" ] || [ ! -x "$LINKER" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp --bin lpp-link)
fi
cat > "$TEMP/direct.lpp" <<'SRC'
def main():
    x := 1
SRC
LPP_AOT=1 "$LPP" "$TEMP/direct.lpp" >/dev/null
if "$LINKER" macho-arm64 "$TEMP/direct.o" -o "$TEMP/direct" >"$TEMP/out" 2>"$TEMP/err"; then
    echo "FAIL: static ARM64 Mach-O unexpectedly linked" >&2
    exit 1
fi
[ ! -e "$TEMP/direct" ]
grep -q "dynamic libSystem imports are required" "$TEMP/err"
echo "PASS ARM64 Mach-O policy rejection"
