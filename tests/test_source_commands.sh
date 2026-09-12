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

# A package directory must not be mistaken for a single source file.
mkdir -p "$TEMP/pkg_test/src"
cat > "$TEMP/pkg_test/lpp.toml" <<'EOF2'
[package]
name = "pkg_test"
version = "0.1.0"
entry = "src/main.lpp"
EOF2
cat > "$TEMP/pkg_test/src/main.lpp" <<'EOF2'
def main():
    print(42)
EOF2

LPP_HOME="$ROOT" LPP_LINKER=host "$LPP" run "$TEMP/pkg_test" > "$TEMP/run.out"
grep -q 42 "$TEMP/run.out"

echo "PASS directory vs source command split"
