#!/usr/bin/env sh
# Verify the CLI command routing behaves correctly for source vs package manager commands.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

RUNNER=""
if [ "${LPP_EMULATOR:-0}" = "1" ] && [ -n "${WASM_RUNTIME:-}" ]; then
    RUNNER="$WASM_RUNTIME"
fi

# 1. Source check
cat > "$TEMP/script.lpp" <<'EOF'
def main():
    print("hello from script")
EOF

"$LPP" check "$TEMP/script.lpp" >/dev/null
echo "PASS source check"

# 2. PM check
mkdir -p "$TEMP/my_pkg/src"
cat > "$TEMP/my_pkg/lpp.toml" <<'EOF'
[package]
name = "my_pkg"
version = "0.1.0"
EOF
cat > "$TEMP/my_pkg/src/main.lpp" <<'EOF'
def main():
    print("hello from pm")
EOF

cd "$TEMP/my_pkg"
"$LPP" check >/dev/null
echo "PASS PM check"
cd "$ROOT"

# 3. Source run
if [ -n "$RUNNER" ]; then
    $RUNNER "$LPP" run "$TEMP/script.lpp" >/dev/null
else
    "$LPP" run "$TEMP/script.lpp" >/dev/null
fi
echo "PASS source run"

# 4. PM run
cd "$TEMP/my_pkg"
if [ -n "$RUNNER" ]; then
    $RUNNER "$LPP" run >/dev/null
else
    "$LPP" run >/dev/null
fi
echo "PASS PM run"
cd "$ROOT"

# 5. Source emit
"$LPP" emit "$TEMP/script.lpp" >/dev/null
obj_ext="o"
if [ "$(uname -s)" = "MINGW32_NT" ] || [ "$(uname -s)" = "MINGW64_NT" ] || [ "$(uname -s)" = "MSYS_NT" ]; then
    obj_ext="obj"
fi
[ -e "$TEMP/script.$obj_ext" ]
echo "PASS source emit"
