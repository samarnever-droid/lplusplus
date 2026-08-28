#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir -p "$TEMP/pkg/src"
cat > "$TEMP/pkg/lpp.toml" <<'TOML'
[package]
name = "testpkg"
version = "0.1.0"
TOML

cat > "$TEMP/pkg/src/main.lpp" <<'LPP_SRC'
def main():
    print(1)
LPP_SRC

cat > "$TEMP/source_file.lpp" <<'LPP_SRC2'
def main():
    print(2)
LPP_SRC2

cd "$TEMP"

# 1. Package check command
(cd pkg && "$LPP" check >/dev/null)
echo "PASS: Package check command"

# 2. Package run command
(cd pkg && "$LPP" run >/dev/null)
echo "PASS: Package run command"

# 3. Source check command
"$LPP" check source_file.lpp >/dev/null
echo "PASS: Source check command"

# 4. Source run command
"$LPP" run source_file.lpp >/dev/null
echo "PASS: Source run command"

# 5. Source emit command
"$LPP" emit source_file.lpp >/dev/null
[ -f "source_file.o" ] || { echo "FAIL: source_file.o not found after emit"; exit 1; }
echo "PASS: Source emit command"

echo "ALL CLI TESTS PASS"
