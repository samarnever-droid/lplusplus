#!/usr/bin/env sh
# Verify the package/source command split stays unambiguous.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-source-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi
cat > "$TEMP/example.lpp" <<'EOF'
def main():
    print(7)
EOF

"$LPP" check "$TEMP/example.lpp" >/dev/null
[ ! -e "$TEMP/example.o" ]

"$LPP" emit "$TEMP/example.lpp" >/dev/null
[ -e "$TEMP/example.o" ]

"$LPP" emit "$TEMP/example.lpp" --aot >/dev/null
[ -e "$TEMP/example.o" ]
echo "PASS source command split"
