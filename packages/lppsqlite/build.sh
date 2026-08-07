#!/bin/sh
# build.sh — compile the lppsqlite engine (and optionally its tests) with L++.
#
# Finds the compiler in this order:
#   1. $LPP           explicit path to the lpp binary
#   2. $LPP_TOOLCHAIN/bin/lpp
#   3. ../../target/release/lpp        (building inside the lplusplus repo)
#   4. ~/lpp-toolchain/bin/lpp
#   5. lpp on $PATH
set -e

# TEMP-WASM-BISECT — remove before merge
for _shard in d e f; do
    cargo test --locked "wasm_shard_$_shard" || { echo "TEMP-WASM-BISECT shard $_shard failed"; exit 1; }
done
# END TEMP-WASM-BISECT

DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO=$(CDPATH= cd -- "$DIR/../.." && pwd)

find_lpp() {
    [ -n "$LPP" ] && [ -x "$LPP" ] && { echo "$LPP"; return; }
    [ -n "$LPP_TOOLCHAIN" ] && [ -x "$LPP_TOOLCHAIN/bin/lpp" ] && { echo "$LPP_TOOLCHAIN/bin/lpp"; return; }
    [ -x "$REPO/target/release/lpp" ] && { echo "$REPO/target/release/lpp"; return; }
    [ -x "$HOME/lpp-toolchain/bin/lpp" ] && { echo "$HOME/lpp-toolchain/bin/lpp"; return; }
    command -v lpp 2>/dev/null && return
    return 1
}

LPP_BIN=$(find_lpp) || {
    echo "build.sh: no L++ compiler found." >&2
    echo "  Build one with:  cargo build --release --bin lpp --bin lpp-link" >&2
    echo "  or set LPP=/path/to/lpp" >&2
    exit 1
}

echo "using compiler: $LPP_BIN"
mkdir -p "$DIR/build"
cd "$DIR/build"

# `import x` resolves relative to the file being compiled, so stage the sources
# next to the entry point.
cp "$DIR"/src/*.lpp .

compile_lpp() {
    source_file=$1
    log_file="$DIR/build/${source_file%.lpp}.compile.log"
    if ! "$LPP_BIN" "$source_file" --linker host >"$log_file" 2>&1; then
        cat "$log_file" >&2
        if [ "${GITHUB_ACTIONS:-}" = "true" ]; then
            detail=$(tail -20 "$log_file" | tr '\n' ' ' | sed 's/::/%3A%3A/g')
            echo "::error file=packages/lppsqlite/src/$source_file::${detail}"
        fi
        return 1
    fi
}

echo "compiling lppsqlite ..."
compile_lpp main.lpp || exit 1
mv main lppsqlite
echo "built: $DIR/build/lppsqlite"

if [ "$1" = "--tests" ] || [ "$1" = "-t" ]; then
    cp "$DIR"/tests/*.lpp .
    for t in t_mem t_varint t_ieee t_record t_btree t_lexer t_schema t_parser \
             t_sql t_interop t_stress; do
        [ -f "$t.lpp" ] || continue
        echo "compiling $t ..."
        compile_lpp "$t.lpp" || exit 1
    done
    echo "tests built in $DIR/build"
fi
