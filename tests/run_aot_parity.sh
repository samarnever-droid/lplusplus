#!/usr/bin/env sh
# Verify that the supported L++ subset has identical C and Cranelift-AOT output.
# Requirements: cargo, cc (or gcc/clang), and a POSIX shell.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
MANIFEST="$ROOT/tests/aot_parity.tsv"
COMPILER="$ROOT/target/release/lpp"
CC=${CC:-cc}
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-aot-parity.XXXXXX")
PASS=0
FAIL=0

cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to run AOT parity tests" >&2
    exit 2
fi
if ! command -v "$CC" >/dev/null 2>&1; then
    echo "C compiler '$CC' is required to link test programs" >&2
    exit 2
fi

if [ ! -x "$COMPILER" ]; then
    echo "[L++] Building release compiler..."
    (cd "$ROOT" && cargo build --release)
fi

run_c_backend() {
    src=$1
    base=$2
    "$COMPILER" "$src" >/dev/null
    c_file="${src%.lpp}.c"
    exe="$TMP/${base}.c.exe"
    [ -f "$c_file" ] || { echo "C backend produced no C file" >&2; return 1; }
    "$CC" -std=c11 -Wall -Wextra -Wno-unused-function -Wno-unused-variable -Wno-unused-parameter "$c_file" -o "$exe" -pthread -lm
    "$exe"
}

run_aot_backend() {
    src=$1
    base=$2
    LPP_AOT=1 "$COMPILER" "$src" >/dev/null
    obj_file="${src%.lpp}.o"
    exe="$TMP/${base}.aot.exe"
    [ -f "$obj_file" ] || { echo "AOT backend produced no object file" >&2; return 1; }
    "$CC" -std=c11 -Wall -Wextra -Wno-unused-function -Wno-unused-variable -Wno-unused-parameter "$obj_file" "$ROOT/lpp_runtime.c" -o "$exe" -pthread -lm
    "$exe"
}

check_rejected_aot() {
    test_name=$1
    expected_diagnostic=$2
    src="$TMP/${test_name}.lpp"
    cp "$ROOT/tests/${test_name}.lpp" "$src"
    rm -f "${src%.lpp}.o"
    # The current CLI reports diagnostics to stderr but historically returns 0,
    # so object-file absence is the reliable rejection criterion.
    LPP_AOT=1 "$COMPILER" "$src" >"$TMP/${test_name}.stdout" 2>"$TMP/${test_name}.stderr" || true
    if [ -e "${src%.lpp}.o" ]; then
        echo "FAIL $test_name: AOT emitted an object for rejected source" >&2
        return 1
    fi
    if ! grep -Fq "$expected_diagnostic" "$TMP/${test_name}.stderr"; then
        echo "FAIL $test_name: expected diagnostic '$expected_diagnostic'" >&2
        cat "$TMP/${test_name}.stderr" >&2
        return 1
    fi
    echo "PASS $test_name"
}

while IFS='|' read -r file expected; do
    case "$file" in ''|\#*) continue ;; esac
    src="$TMP/$file"
    cp "$ROOT/tests/$file" "$src"
    base=${file%.lpp}

    if c_output=$(run_c_backend "$src" "$base") && aot_output=$(run_aot_backend "$src" "$base"); then
        wanted=$(printf '%b' "$expected")
        if [ "$c_output" = "$wanted" ] && [ "$aot_output" = "$wanted" ]; then
            echo "PASS $file"
            PASS=$((PASS + 1))
        else
            echo "FAIL $file: backend output mismatch" >&2
            printf '  expected: %s\n  C:        %s\n  AOT:      %s\n' "$wanted" "$c_output" "$aot_output" >&2
            FAIL=$((FAIL + 1))
        fi
    else
        echo "FAIL $file: compile, link, or execution failed" >&2
        FAIL=$((FAIL + 1))
    fi
done < "$MANIFEST"

for rejected_case in \
    "aot_reject_arc_cycle:ARC cannot reclaim ownership cycles" \
    "aot_reject_list_arc_cycle:ARC cannot reclaim ownership cycles"
do
    test_name=${rejected_case%%:*}
    expected_diagnostic=${rejected_case#*:}
    if check_rejected_aot "$test_name" "$expected_diagnostic"; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
done

echo "AOT parity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
