#!/usr/bin/env sh
# Verify the package/source command split and cli routing logic.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir -p "$TEMP/test_pkg/src"
cat > "$TEMP/test_pkg/lpp.toml" <<'INNEREOF'
[package]
name = "test_pkg"
version = "0.1.0"
INNEREOF
cat > "$TEMP/test_pkg/src/main.lpp" <<'INNEREOF'
def main():
    print(1)
INNEREOF

echo "Running CLI tests..."

(cd "$TEMP/test_pkg" && "$LPP" run src/main.lpp) >/dev/null
echo "PASS run source file"

(cd "$TEMP/test_pkg" && "$LPP" run .) >/dev/null
echo "PASS run package dir (dot)"

(cd "$TEMP/test_pkg" && "$LPP" run) >/dev/null
echo "PASS run package dir (implicit)"

(cd "$TEMP/test_pkg" && "$LPP" check src/main.lpp) >/dev/null
echo "PASS check source file"

(cd "$TEMP/test_pkg" && "$LPP" check .) >/dev/null
echo "PASS check package dir (dot)"

(cd "$TEMP/test_pkg" && "$LPP" check) >/dev/null
echo "PASS check package dir (implicit)"

(cd "$TEMP/test_pkg" && "$LPP" emit src/main.lpp) >/dev/null
echo "PASS emit source file"

# Add package tests
cat > "$TEMP/test_pkg/src/lib.lpp" <<'INNEREOF'
def test_lib_func() -> Int:
    return 42
INNEREOF

(cd "$TEMP/test_pkg" && "$LPP" build .) >/dev/null
echo "PASS build package dir (dot)"

(cd "$TEMP/test_pkg" && "$LPP" build) >/dev/null
echo "PASS build package dir (implicit)"
