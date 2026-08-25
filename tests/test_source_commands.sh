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

mkdir -p "$TEMP/project"
cat > "$TEMP/project/lpp.toml" <<'INNEREOF'
[package]
name = "test_pkg"
version = "0.1.0"
entry = "src/main.lpp"
INNEREOF

mkdir -p "$TEMP/project/src"
cat > "$TEMP/project/src/main.lpp" <<'INNEREOF'
def main():
    print(42)
INNEREOF

(cd "$TEMP/project" && "$LPP" run >/dev/null)
[ -e "$TEMP/project/LppData/build/release/output" ] || [ -e "$TEMP/project/LppData/build/release/test_pkg" ] || [ -e "$TEMP/project/LppData/build/release/output.exe" ] || [ -e "$TEMP/project/LppData/build/release/test_pkg.exe" ]
echo "PASS package run command"

mkdir -p "$TEMP/project2"
cat > "$TEMP/project2/lpp.toml" <<'INNEREOF'
[package]
name = "test_pkg2"
version = "0.1.0"
entry = "src/main.lpp"
INNEREOF

mkdir -p "$TEMP/project2/src"
cat > "$TEMP/project2/src/main.lpp" <<'INNEREOF'
def main():
    print(42)
INNEREOF

(cd "$TEMP/project2" && "$LPP" check >/dev/null)
echo "PASS package check command"

mkdir -p "$TEMP/test.lpp"
cat > "$TEMP/test.lpp/lpp.toml" <<'INNEREOF'
[package]
name = "test_pkg_dot_lpp"
version = "0.1.0"
entry = "src/main.lpp"
INNEREOF

mkdir -p "$TEMP/test.lpp/src"
cat > "$TEMP/test.lpp/src/main.lpp" <<'INNEREOF'
def main():
    print(123)
INNEREOF

(cd "$TEMP/test.lpp" && "$LPP" run >/dev/null)
echo "PASS package run with .lpp folder name"

(cd "$TEMP/test.lpp" && "$LPP" check >/dev/null)
echo "PASS package check with .lpp folder name"

(cd "$TEMP/test.lpp" && "$LPP" build >/dev/null)
echo "PASS package build with .lpp folder name"
