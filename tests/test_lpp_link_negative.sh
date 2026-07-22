#!/usr/bin/env sh
# High-level negative tests for lpp-link. Unsupported input must fail clearly;
# it must never emit a partially linked executable.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
LINKER="$ROOT/target/release/lpp-link"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-link-negative.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi
if [ ! -x "$LPP" ] || [ ! -x "$LINKER" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp --bin lpp-link)
fi

# Malformed data must be rejected before output creation.
printf 'not an ELF object\n' > "$TEMP/bad.o"
if "$LINKER" "$TEMP/bad.o" -o "$TEMP/bad" >"$TEMP/bad.out" 2>"$TEMP/bad.err"; then
    echo "FAIL: malformed object unexpectedly linked" >&2
    exit 1
fi
[ ! -e "$TEMP/bad" ]
grep -q "not an x86-64 ELF relocatable object\|parse" "$TEMP/bad.err"

# A program with a runtime import must not link without its runtime object.
cat > "$TEMP/needs_runtime.lpp" <<'EOF'
def main():
    print(7)
EOF
"$LPP" emit "$TEMP/needs_runtime.lpp" --aot >/dev/null
if "$LINKER" "$TEMP/needs_runtime.o" -o "$TEMP/missing_runtime" >"$TEMP/missing.out" 2>"$TEMP/missing.err"; then
    echo "FAIL: unresolved runtime import unexpectedly linked" >&2
    exit 1
fi
[ ! -e "$TEMP/missing_runtime" ]
grep -q "unresolved GOT symbol\|unresolved external relocation" "$TEMP/missing.err"
echo "PASS direct ELF linker negative cases"
