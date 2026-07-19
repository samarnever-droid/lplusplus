#!/usr/bin/env sh
# Phase M2 direct Mach-O runtime-free smoke test. Run on macOS CI only.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
LINKER="$ROOT/target/release/lpp-link"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-macho.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ "$(uname -s)" != "Darwin" ]; then
    echo "SKIP: direct Mach-O test requires macOS"
    exit 0
fi
if [ ! -x "$LPP" ] || [ ! -x "$LINKER" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp --bin lpp-link)
fi
cat > "$TEMP/direct.lpp" <<'EOF'
def main():
    x := 1
EOF
LPP_AOT=1 "$LPP" "$TEMP/direct.lpp" >/dev/null
"$LINKER" macho "$TEMP/direct.o" -o "$TEMP/direct"
file "$TEMP/direct" | grep -q "Mach-O"
"$TEMP/direct"
[ "$?" -eq 0 ]
echo "PASS direct Mach-O linker MVP"
