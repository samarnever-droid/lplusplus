#!/usr/bin/env sh
# Verify CLI command routing correctly differentiates between packages and single files.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    return 0
fi
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir -p "$TEMP/test_pkg/src"
cat > "$TEMP/test_pkg/lpp.toml" <<'EOF2'
[package]
name = "test_pkg"
version = "0.1.0"
entry = "src/main.lpp"
EOF2
cat > "$TEMP/test_pkg/src/main.lpp" <<'EOF2'
def main():
    print(123)
EOF2

LPP_HOME="$ROOT" LPP_LINKER=host "$LPP" run "$TEMP/test_pkg" > "$TEMP/pkg_run.out"
grep -q 123 "$TEMP/pkg_run.out"
echo "PASS directory run command routing"

LPP_HOME="$ROOT" "$LPP" check "$TEMP/test_pkg" > "$TEMP/pkg_check.out"
grep -q "Project is semantically valid" "$TEMP/pkg_check.out" || (cat "$TEMP/pkg_check.out" && false)
echo "PASS directory check command routing"

LPP_HOME="$ROOT" LPP_LINKER=host "$LPP" build "$TEMP/test_pkg" > "$TEMP/pkg_build.out"
grep -q "Build successful" "$TEMP/pkg_build.out" || (cat "$TEMP/pkg_build.out" && false)
echo "PASS directory build command routing"

cat > "$TEMP/test_file.lpp" <<'EOF2'
def main():
    print(456)
EOF2

LPP_HOME="$ROOT" LPP_LINKER=host "$LPP" run "$TEMP/test_file.lpp" > "$TEMP/file_run.out"
grep -q 456 "$TEMP/file_run.out"
echo "PASS file run command routing"

echo "PASS all CLI commands tests"
