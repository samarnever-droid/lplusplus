#!/usr/bin/env sh
# Phase 1 native-linker roadmap integration test.
# Verifies an installed lpp can build/link using only lpp_runtime.o, with no
# lpp_runtime.c present in the installation or project directory.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER="$ROOT/target/release/lpp"
CC=${CC:-cc}
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-packaged-runtime.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1 || ! command -v "$CC" >/dev/null 2>&1; then
    echo "SKIP: requires cargo and $CC"
    exit 0
fi

if [ ! -x "$COMPILER" ]; then
    (cd "$ROOT" && cargo build --release)
fi
mkdir -p "$TEMP/install/bin" "$TEMP/install/lib" "$TEMP/work"
cp "$COMPILER" "$TEMP/install/bin/lpp"
"$CC" -O2 -fPIC -c "$ROOT/lpp_runtime.c" -o "$TEMP/install/lib/lpp_runtime.o" -pthread

cd "$TEMP/work"
"$TEMP/install/bin/lpp" new packaged_runtime_demo >/dev/null
cd packaged_runtime_demo
# Do not copy lpp_runtime.c: success proves the object path was selected.
"$TEMP/install/bin/lpp" build >/dev/null
OUTPUT=$(target/release/packaged_runtime_demo)
[ "$OUTPUT" = "Hello from L++ project!" ]
echo "PASS packaged runtime object linking"
