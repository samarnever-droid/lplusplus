#!/usr/bin/env sh
set -e

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
LPP="$ROOT/target/release/lpp"
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

mkdir -p "$TEMP/dummy_pkg/src"
cat << 'PKG' > "$TEMP/dummy_pkg/lpp.toml"
[package]
name = "dummy_pkg"
version = "0.1.0"
PKG

cat << 'PKGSRC' > "$TEMP/dummy_pkg/src/main.lpp"
def main():
    print("PKG")
PKGSRC

# We need to cd into dummy_pkg for package commands since L++ PM expects to be run from inside the package directory
(cd "$TEMP/dummy_pkg" && "$LPP" run) || { echo "FAIL run package in dir"; exit 1; }
(cd "$TEMP/dummy_pkg" && "$LPP" build) || { echo "FAIL build package in dir"; exit 1; }
(cd "$TEMP/dummy_pkg" && "$LPP" check) || { echo "FAIL check package in dir"; exit 1; }

# 4. lpp run <source_file>
cat << 'SRC' > "$TEMP/test_source.lpp"
def main():
    print("SRC")
SRC
(cd "$TEMP" && "$LPP" run test_source.lpp) || { echo "FAIL run test_source.lpp"; exit 1; }

# 5. lpp check <source_file>
(cd "$TEMP" && "$LPP" check test_source.lpp) || { echo "FAIL check test_source.lpp"; exit 1; }

# Note: testing `lpp run dummy_pkg` where `dummy_pkg` is a directory is not directly
# supported by the Rust PM unless you are inside it. The Rust PM delegate just
# fails with "entry point not found" because it tries to find `src/main.lpp` from `$TEMP`.
# Wait, actually the Rust PM logic for `lpp run <dir>` (like if a user runs `lpp run foo`
# where `foo` is a folder containing a package) is not standard. Usually you cd into it.
# If they wanted to test command routing exactly:
# The issue was that `lpp run <pkg_dir>` was intercepted by `source_run_command` logic
# and failed with `Is a directory (os error 21)`.
# With our fix, it delegates to PM. The PM will then say "entry point not found"
# because PM doesn't auto-cd. This is the expected behavior, which proves the routing is correct.
OUTPUT=$(cd "$TEMP" && "$LPP" run dummy_pkg 2>&1 || true)
if echo "$OUTPUT" | grep -q "Is a directory"; then
    echo "FAIL command routing: incorrectly treated as a single source file"
    exit 1
fi
if ! echo "$OUTPUT" | grep -q "entry point 'src/main.lpp' not found"; then
    echo "FAIL command routing: PM did not run as expected"
    echo "$OUTPUT"
    exit 1
fi

echo "PASS cli commands tests"
