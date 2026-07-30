#!/usr/bin/env sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER="$ROOT/target/release/lpp"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-llvm-smoke.XXXXXX")
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

for file in arith.lpp branches.lpp fib.lpp loop.lpp nested_calls.lpp \
            closure_test.lpp stack_owned_fields.lpp recursive_structures.lpp arena_return.lpp \
            test_map_kv.lpp vector_api.lpp vector_builtin.lpp; do
    case "$file" in
        arith.lpp) expected='15
5
50
2' ;;
        branches.lpp) expected='1
0
1' ;;
        fib.lpp) expected='55' ;;
        loop.lpp) expected='5050' ;;
        nested_calls.lpp) expected='120' ;;
        closure_test.lpp) expected='52' ;;
        stack_owned_fields.lpp) expected='owned-field
11
10
42
3
8
6
stack-owned-ok' ;;
        recursive_structures.lpp) expected='13
6
1
2' ;;
        arena_return.lpp) expected='1
2
arena-ok' ;;
        test_map_kv.lpp) expected='start test_string_keys
created map
put apple
put banana
100
200
2
1
start test_integer_keys
999
888' ;;
        vector_api.lpp) expected='11
12
23' ;;
        vector_builtin.lpp) expected='620204246048896' ;;
    esac
    src="$TMP/$file"
    cp "$ROOT/tests/$file" "$src"
    for linker in host direct; do
        "$COMPILER" "$src" --backend llvm --linker "$linker" >/dev/null 2>"$TMP/err"
        exe="${src%.lpp}"
        [ -x "$exe" ] || { cat "$TMP/err" >&2; exit 1; }
        got=$("$exe")
        if [ "$got" = "$expected" ]; then
            echo "PASS $file ($linker)"
        else
            echo "FAIL $file ($linker): expected '$expected', got '$got'" >&2
            exit 1
        fi
    done
done
echo 'LLVM smoke: PASS'
