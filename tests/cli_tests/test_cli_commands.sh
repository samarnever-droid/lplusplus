#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/debug/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --bin lpp)
fi

mkdir "$TEMP/pkg"
cat > "$TEMP/pkg/lpp.toml" <<'TOML'
[package]
name = "pkg"
version = "0.1.0"
entry = "src/main.lpp"
TOML
mkdir "$TEMP/pkg/src"
cat > "$TEMP/pkg/src/main.lpp" <<'LPP'
def main():
    print(1)
LPP

cat > "$TEMP/example.lpp" <<'LPP'
def main():
    print(2)
LPP

# Test 1: check source
"$LPP" check "$TEMP/example.lpp" >/dev/null

# Test 2: run source
LPP_EMULATOR=1 "$LPP" run "$TEMP/example.lpp" >/dev/null

# Test 3: emit source
"$LPP" emit "$TEMP/example.lpp" >/dev/null

# Test 4: run package
(
  cd "$TEMP/pkg"
  LPP_EMULATOR=1 "$LPP" run >/dev/null
)

# Test 5: check package
(
  cd "$TEMP/pkg"
  LPP_EMULATOR=1 "$LPP" check >/dev/null
)

echo "PASS 5 CLI tests"
