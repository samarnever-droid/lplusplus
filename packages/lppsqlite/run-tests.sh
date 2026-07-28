#!/bin/sh
# run-tests.sh — build and run the whole lppsqlite test suite.
#
#   ./run-tests.sh            unit suites + differential tests
#   ./run-tests.sh --unit     unit suites only
#   ./run-tests.sh --diff     differential tests only (needs python3)
set -e

DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BUILD="$DIR/build"

# build.sh locates the compiler ($LPP, $LPP_TOOLCHAIN, the repo's
# target/release, ~/lpp-toolchain, or PATH).

MODE=${1:-all}

if [ "$MODE" = "all" ] || [ "$MODE" = "--unit" ]; then
    "$DIR/build.sh" --tests >/dev/null
fi

FAIL=0

if [ "$MODE" = "all" ] || [ "$MODE" = "--unit" ]; then
    echo "== unit suites =="
    cd "$BUILD"
    rm -f sql_test.db stress.db bt_test.db sc_test.db lppdb_test.db 2>/dev/null || true

    # t_interop reads a database produced by the real sqlite3 library.
    if command -v python3 >/dev/null 2>&1; then
        rm -f from_real_sqlite.db
        python3 - <<'PY'
import sqlite3
c = sqlite3.connect('from_real_sqlite.db')
c.execute('CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, nickname TEXT,'
          ' height REAL, balance INTEGER, bignum INTEGER, data BLOB)')
c.executemany('INSERT INTO people VALUES (?,?,?,?,?,?,?)', [
    (1, 'Ada', None, 1.68, 500, 10, b'\x01\x02\x03'),
    (2, 'Grace', 'Amazing', 1.70, 850, 20, b'GG'),
    (3, 'Alan', 'Turing', 1.75, -250, 30, None),
    (4, 'Edsger', None, 1.80, 0, 9007199254740993, None),
    (5, 'Ștefan', 'ștef', 1.66, 250, 40, None),
    (6, 'Kurt', 'K', 1.72, -250, 50, None)])
c.execute('CREATE TABLE wide (k INTEGER PRIMARY KEY, val TEXT)')
c.executemany('INSERT INTO wide VALUES (?,?)', [(i, f'row-{i}') for i in range(500)])
c.execute('CREATE TABLE bigrow (id INTEGER PRIMARY KEY, blobtext TEXT)')
c.execute('INSERT INTO bigrow VALUES (1, ?)', ('A' * 9000,))
c.commit(); c.close()
PY
    fi

    for t in t_mem t_varint t_ieee t_record t_btree t_lexer t_schema t_parser \
             t_sql t_interop t_stress; do
        [ -x "./$t" ] || continue
        OUT=$(./"$t" 2>&1) || true
        echo "$OUT" | grep -E '^(PASS|FAIL)' || echo "$OUT" | tail -1
        echo "$OUT" | grep -q '^FAIL' && FAIL=1
    done
    cd "$DIR"
fi

if [ "$MODE" = "all" ] || [ "$MODE" = "--diff" ]; then
    if command -v python3 >/dev/null 2>&1; then
        echo
        echo "== differential vs real sqlite3 =="
        "$DIR/build.sh" >/dev/null
        python3 "$DIR/tests/difftest.py" || FAIL=1
    else
        echo "python3 not found; skipping differential tests" >&2
    fi
fi

exit $FAIL
