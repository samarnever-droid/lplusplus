#!/bin/sh
# run-tests.sh — build and run the compresslpp test suite.
#
#   ./run-tests.sh          unit suites + python cross-verification
#   ./run-tests.sh --unit   unit suites only
#   ./run-tests.sh --diff   cross-verification only
set -e

DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BUILD="$DIR/build"
MODE=${1:-all}

"$DIR/build.sh" --tests >/dev/null

cd "$BUILD"

if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 is required to generate reference fixtures" >&2
    exit 1
fi
python3 "$DIR/tests/fixtures.py" >/dev/null

FAIL=0

if [ "$MODE" = "all" ] || [ "$MODE" = "--unit" ]; then
    echo "== unit suites =="
    for t in t_inflate t_deflate t_zip t_tar t_gzip; do
        [ -x "./$t" ] || continue
        OUT=$(./"$t" 2>&1) || true
        P=$(echo "$OUT" | grep -c '^PASS' || true)
        F=$(echo "$OUT" | grep -c '^FAIL' || true)
        if [ "$F" -gt 0 ]; then
            echo "FAIL  $t  ($F failed, $P passed)"
            echo "$OUT" | grep '^FAIL' | sed 's/^/      /'
            FAIL=1
        else
            echo "PASS  $t  ($P checks)"
        fi
    done
fi

if [ "$MODE" = "all" ] || [ "$MODE" = "--diff" ]; then
    echo
    echo "== cross-verification against python zlib/zipfile/tarfile/gzip =="
    python3 "$DIR/tests/verify.py" || FAIL=1
fi

exit $FAIL
