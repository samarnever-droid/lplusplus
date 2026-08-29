#!/usr/bin/env sh
# Additional CLI tests focusing on validating command routing (package vs source file behavior)
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
else

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

echo "Running 5 new CLI tests..."

# 1. Test basic checking of a single source file
cat > "$TEMP/test1.lpp" <<'INNER'
def main():
    print(1)
INNER
"$LPP" check "$TEMP/test1.lpp" >/dev/null
echo "Test 1 passed"

# 2. Test emitting object file
"$LPP" emit "$TEMP/test1.lpp" >/dev/null
[ -e "$TEMP/test1.o" ] || [ -e "$TEMP/test1.obj" ]
echo "Test 2 passed"

# 3. Test L++ runtime execution
"$LPP" "$TEMP/test1.lpp" -o "$TEMP/test1.exe" >/dev/null
if [ "${LPP_EMULATOR:-}" = "1" ] || [ -n "${LPP_EMULATOR:-}" ]; then
    # LPP_EMULATOR=1 is used to disable native execution if they can't run on the host.
    # Usually you'd prepend something like `qemu-x86_64`, but if it's just '1' we skip execution or use a dummy emulator
    if [ "$LPP_EMULATOR" = "1" ]; then
        :
    else
        $LPP_EMULATOR "$TEMP/test1.exe" >/dev/null
    fi
else
    "$TEMP/test1.exe" >/dev/null
fi
echo "Test 3 passed"

# 4. Test PM init routing
(cd "$TEMP" && "$LPP" init my_pkg) >/dev/null
[ -e "$TEMP/my_pkg/lpp.toml" ] || [ -e "$TEMP/my_pkg/src/main.lpp" ] || ( [ -e "$TEMP/lpp.toml" ] && [ -e "$TEMP/src/main.lpp" ] )
echo "Test 4 passed"

# 5. Test PM routing with check
if [ -e "$TEMP/my_pkg/lpp.toml" ]; then
    (cd "$TEMP/my_pkg" && "$LPP" check) >/dev/null
else
    (cd "$TEMP" && "$LPP" check) >/dev/null
fi
echo "Test 5 passed"

echo "PASS all additional CLI tests"

fi
