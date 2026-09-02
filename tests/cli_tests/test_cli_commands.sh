#!/usr/bin/env sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-tests.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

echo "Running tests in emulator mode"

mkdir -p "$TEMP/my_pkg"
cat > "$TEMP/my_pkg/lpp.toml" << 'TOML'
[package]
name = "my_pkg"
version = "0.1.0"
TOML

mkdir -p "$TEMP/my_pkg/src"
cat > "$TEMP/my_pkg/src/main.lpp" << 'LPP'
def main():
    print(42)
LPP

"$LPP" run "$TEMP/my_pkg" > /dev/null
echo "CLI test 1 passed"

"$LPP" check "$TEMP/my_pkg" > /dev/null
echo "CLI test 2 passed"

mkdir -p "$TEMP/my_pkg2"
cat > "$TEMP/my_pkg2/lpp.toml" << 'TOML'
[package]
name = "my_pkg2"
version = "0.1.0"
TOML

mkdir -p "$TEMP/my_pkg2/src"
cat > "$TEMP/my_pkg2/src/main.lpp" << 'LPP'
def main():
    print(1)
LPP

"$LPP" run "$TEMP/my_pkg2" > /dev/null
echo "CLI test 3 passed"

"$LPP" check "$TEMP/my_pkg2" > /dev/null
echo "CLI test 4 passed"

cat > "$TEMP/my_file.lpp" << 'LPP'
def main():
    print(2)
LPP

"$LPP" run "$TEMP/my_file.lpp" > /dev/null
echo "CLI test 5 passed"

echo "PASS all CLI tests"
