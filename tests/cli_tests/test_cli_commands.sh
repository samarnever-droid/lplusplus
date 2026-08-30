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

mkdir -p "$TEMP/my_pkg"
cat > "$TEMP/my_pkg/lpp.toml" <<'INNER'
[package]
name = "my_pkg"
version = "0.1.0"
INNER
mkdir -p "$TEMP/my_pkg/src"
cat > "$TEMP/my_pkg/src/main.lpp" <<'INNER'
def main():
    print(42)
INNER

cd "$TEMP"
cat > "$TEMP/dummy.lpp" <<'INNER'
def main():
    print("dummy")
INNER

# Ensure emulator fallback if needed
export LPP_EMULATOR=1

"$LPP" run dummy.lpp >/dev/null
echo "lpp run <file> PASSED"

"$LPP" check dummy.lpp >/dev/null
echo "lpp check <file> PASSED"

"$LPP" emit dummy.lpp >/dev/null
echo "lpp emit <file> PASSED"

set +e
output_run=$("$LPP" run my_pkg 2>&1)
output_check=$("$LPP" check my_pkg 2>&1)
set -e

# It should route to PM and fail with PM's error ("entry point 'src/main.lpp' not found")
# instead of trying to compile a directory
echo "$output_run" | grep -q "entry point 'src/main.lpp' not found"
echo "lpp run <directory> PASSED"

echo "$output_check" | grep -q "entry point 'src/main.lpp' not found"
echo "lpp check <directory> PASSED"

echo "PASS CLI commands"
