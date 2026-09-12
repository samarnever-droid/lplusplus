#!/usr/bin/env sh
# Verify that lpp appropriately handles CLI arguments in various scenarios.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-tests.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "SKIP: requires cargo"
    exit 0
fi

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir -p "$TEMP/test_pkg/src"
cat > "$TEMP/test_pkg/lpp.toml" <<EOF
[package]
name = "test_pkg"
version = "0.1.0"
EOF

cat > "$TEMP/test_pkg/src/main.lpp" <<EOF
def main():
    print("hello from pkg")
EOF

# 1) lpp run with a package directory
(
    cd "$TEMP"
    "$LPP" run test_pkg > run_pkg_out.txt
    if ! grep -q "hello from pkg" run_pkg_out.txt; then
        echo "FAIL: lpp run test_pkg did not execute package correctly."
        exit 1
    fi
)

# 2) lpp check with a package directory
(
    cd "$TEMP"
    if ! "$LPP" check test_pkg >/dev/null; then
        echo "FAIL: lpp check test_pkg failed."
        exit 1
    fi
)

# 3) lpp emit with a package directory
# emit is only for single source files. passing a directory should either trigger a PM error or fall back to single file emit which would fail on a dir
# To align with PM behavior, lpp emit isn't a valid PM command so it might fall through. Wait, actually `emit` just isn't processed if the argument is a directory and not a file.
# So it falls through to PM if it matched, but `emit` isn't a PM command. It should fail to find the file or print usage.
(
    cd "$TEMP"
    set +e
    "$LPP" emit test_pkg > emit_pkg_out.txt 2>&1
    RET=$?
    set -e
    # Since emit is not a PM command, it should drop to normal compile but fail because it's a directory
    if [ $RET -eq 0 ]; then
        echo "FAIL: lpp emit test_pkg should have failed or not done single file compile."
        exit 1
    fi
)

cat > "$TEMP/single_file_no_ext" <<EOF
def main():
    print("hello from single file")
EOF

# 4) lpp run with a single source file (no .lpp extension)
(
    cd "$TEMP"
    "$LPP" run single_file_no_ext > run_single_out.txt
    if ! grep -q "hello from single file" run_single_out.txt; then
        echo "FAIL: lpp run single_file_no_ext did not execute correctly."
        exit 1
    fi
)

# 5) lpp check with a single source file (no .lpp extension)
(
    cd "$TEMP"
    if ! "$LPP" check single_file_no_ext >/dev/null; then
        echo "FAIL: lpp check single_file_no_ext failed."
        exit 1
    fi
)

echo "PASS CLI commands routing tests"
