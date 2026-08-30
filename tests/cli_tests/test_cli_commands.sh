#!/usr/bin/env sh
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

    mkdir -p "$TEMP/mypkg/src"
    cat > "$TEMP/mypkg/lpp.toml" <<'MANIFEST'
[package]
name = "mypkg"
version = "0.1.0"
MANIFEST
    cat > "$TEMP/mypkg/src/main.lpp" <<'MAIN'
def main():
    print_str("from pkg\n")
MAIN

    # Test 1: package directory check
    (cd "$TEMP" && "$LPP" check mypkg) >/dev/null 2>&1
    echo "PASS pkg dir check"

    # Test 2: package directory run
    (cd "$TEMP" && "$LPP" run mypkg) >/dev/null 2>&1
    echo "PASS pkg dir run"

    # Test 3: source file check
    cat > "$TEMP/script.lpp" <<'SCRIPT'
def main():
    print_str("script\n")
SCRIPT
    "$LPP" check "$TEMP/script.lpp" >/dev/null 2>&1
    echo "PASS source script check"

    # Test 4: source file run
    "$LPP" run "$TEMP/script.lpp" >/dev/null 2>&1
    echo "PASS source script run"

    # Test 5: custom entry point without .lpp check
    cat > "$TEMP/bare_script" <<'BARE'
def main():
    print_str("bare\n")
BARE
    "$LPP" check "$TEMP/bare_script" >/dev/null 2>&1
    echo "PASS bare script check"

    echo "ALL TESTS PASSED"
fi
