#!/usr/bin/env sh
# ARM64 direct Mach-O safety policy test. Production macOS rejects static ARM64
# executables; lpp-link must fail clearly rather than emit an OS-killed binary.
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
cat > "$TEMP/direct.lpp" <<'EOF'
def main():
    x := 1
EOF
LPP_AOT=1 "$LPP" "$TEMP/direct.lpp" >/dev/null
if "$LINKER" macho-arm64 "$TEMP/direct.o" -o "$TEMP/direct" >"$TEMP/out" 2>"$TEMP/err"; then
    echo "FAIL: static ARM64 Mach-O was emitted despite macOS policy" >&2
    exit 1
fi
grep -Fq 'dynamic libSystem imports are required' "$TEMP/err"
test ! -e "$TEMP/direct"
echo "PASS ARM64 Mach-O policy rejection"
