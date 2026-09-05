#!/usr/bin/env sh
# Verify L++ CLI command routing for package vs source files.
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

# 1. Source `check` command on a single .lpp file.
cat > "$TEMP/source_check.lpp" <<'INNER_EOF'
def main():
    print(1)
INNER_EOF
"$LPP" check "$TEMP/source_check.lpp" >/dev/null
echo "PASS source check"

# 2. Source `run` command on a single .lpp file.
cat > "$TEMP/source_run.lpp" <<'INNER_EOF'
def main():
    print(2)
INNER_EOF
"$LPP" run "$TEMP/source_run.lpp" >/dev/null
echo "PASS source run"

# 3. Package `init` command to initialize a new project.
(cd "$TEMP" && "$LPP" init my_pkg) >/dev/null
[ -d "$TEMP/my_pkg" ]
[ -f "$TEMP/my_pkg/lpp.toml" ]
[ -f "$TEMP/my_pkg/src/main.lpp" ]
echo "PASS package init"

# 4. Package `check` command on a valid project directory.
(cd "$TEMP" && "$LPP" check my_pkg) >/dev/null
echo "PASS package check"

# 5. Package `run` command on a valid project directory.
(cd "$TEMP" && "$LPP" run my_pkg) >/dev/null
echo "PASS package run"

echo "PASS all cli command tests"