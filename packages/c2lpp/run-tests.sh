#!/bin/sh
set -eu
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO=$(CDPATH= cd -- "$DIR/../.." 2>/dev/null && pwd || true)

if [ -n "${LPP:-}" ] && [ -x "$LPP" ]; then
    LPP_BIN=$LPP
elif [ -n "$REPO" ] && [ -x "$REPO/target/release/lpp" ]; then
    LPP_BIN=$REPO/target/release/lpp
else
    LPP_BIN=$(command -v lpp) || { echo 'run-tests.sh: no L++ compiler found' >&2; exit 2; }
fi

LPP="$LPP_BIN" sh "$DIR/tests/run.sh"
