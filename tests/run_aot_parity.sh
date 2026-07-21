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

run_native_aot() {
    src=$1
    base=$2
    LPP_AOT=1 "$COMPILER" "$src" --aot >/dev/null
    obj_file="${src%.lpp}.o"
    exe="$TMP/${base}.aot.exe"
    [ -f "$obj_file" ] || { echo "AOT backend produced no object file" >&2; return 1; }
    "$CC" -std=c11 -Wall -Wextra -Wno-unused-function -Wno-unused-variable -Wno-unused-parameter "$obj_file" "$ROOT/lpp_runtime.c" -o "$exe" -pthread -lm
    "$exe"
}

run_direct_link() {
    src=$1
    base=$2
    "$COMPILER" "$src" >/dev/null
    exe="${src%.lpp}"
    if [ -f "${exe}.exe" ]; then
        exe="${exe}.exe"
    fi
    [ -f "$exe" ] || { echo "Direct link produced no executable" >&2; return 1; }
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

    if aot_output=$(run_native_aot "$src" "$base") && direct_output=$(run_direct_link "$src" "$base"); then
        wanted=$(printf '%b' "$expected")
        if [ "$aot_output" = "$wanted" ] && [ "$direct_output" = "$wanted" ]; then
            echo "PASS $file"
            PASS=$((PASS + 1))
        else
            echo "FAIL $file: backend output mismatch" >&2
            printf '  expected: %s\n  AOT:      %s\n  Direct:   %s\n' "$wanted" "$aot_output" "$direct_output" >&2
            FAIL=$((FAIL + 1))
        fi
    else
        echo "FAIL $file: compile, link, or execution failed" >&2
        FAIL=$((FAIL + 1))
    fi
done < "$MANIFEST"

for rejected_case in \
    "aot_reject_arc_cycle:ARC cannot reclaim ownership cycles" \
    "aot_reject_list_arc_cycle:ARC cannot reclaim ownership cycles" \
    "aot_reject_mut_closure:not supported safely yet"
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
