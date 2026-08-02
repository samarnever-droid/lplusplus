#!/bin/sh
set -eu
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO=$(CDPATH= cd -- "$DIR/../.." 2>/dev/null && pwd || true)

find_lpp() {
    [ -n "${LPP:-}" ] && [ -x "$LPP" ] && { echo "$LPP"; return; }
    [ -n "${LPP_TOOLCHAIN:-}" ] && [ -x "$LPP_TOOLCHAIN/bin/lpp" ] && { echo "$LPP_TOOLCHAIN/bin/lpp"; return; }
    [ -n "$REPO" ] && [ -x "$REPO/target/release/lpp" ] && { echo "$REPO/target/release/lpp"; return; }
    command -v lpp 2>/dev/null || return 1
}

LPP_BIN=$(find_lpp) || { echo 'build.sh: no L++ compiler found' >&2; exit 2; }
rm -rf "$DIR/build"
mkdir -p "$DIR/build"
cp "$DIR"/src/*.lpp "$DIR/build/"
mkdir -p "$DIR/build/backends/sqlite/src"
cp "$DIR"/backends/sqlite/src/*.lpp "$DIR/build/backends/sqlite/src/"
(cd "$DIR/build" && "$LPP_BIN" main.lpp --linker host >/dev/null && mv main c2lpp)
echo "built: $DIR/build/c2lpp"
