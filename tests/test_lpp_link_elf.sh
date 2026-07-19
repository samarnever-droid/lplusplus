#!/usr/bin/env sh
# Phase 2 direct-linker MVP integration test.
# Produces a Linux x86-64 ELF executable without invoking cc/gcc/clang at link
# time. The input intentionally has no runtime calls; runtime imports are the
# next linker expansion milestone.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
LINKER="$ROOT/target/release/lpp-link"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-link-elf.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi
if [ ! -x "$LPP" ] || [ ! -x "$LINKER" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp --bin lpp-link)
fi
cat > "$TEMP/no_runtime.lpp" <<'EOF'
def main():
    x := 1
EOF
LPP_AOT=1 "$LPP" "$TEMP/no_runtime.lpp" >/dev/null
"$LINKER" "$TEMP/no_runtime.o" -o "$TEMP/no_runtime"
[ "$("$TEMP/no_runtime"; echo $?)" = "0" ]
file "$TEMP/no_runtime" | grep -q "ELF 64-bit"
echo "PASS direct ELF linker MVP"
