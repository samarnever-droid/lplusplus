#!/usr/bin/env sh
# Regression gate for string-loop ARC pressure and generic enum payload layout.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER="$ROOT/target/release/lpp"
CC=${CC:-cc}
LLVM_CC=${LPP_LLVM_CC:-clang}
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-hard-memory.XXXXXX")
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

[ -x "$COMPILER" ] || { echo "release compiler missing" >&2; exit 2; }
command -v "$CC" >/dev/null 2>&1 || { echo "C compiler '$CC' unavailable" >&2; exit 2; }

run_with_timeout() {
    executable=$1
    if command -v timeout >/dev/null 2>&1; then
        timeout 30 "$executable"
    else
        "$executable"
    fi
}

# The exact 100,000-iteration reproducer must complete through both runtime
# paths; only fixed marker lines are checked because elapsed milliseconds vary.
for linker in host direct; do
    src="$TMP/memory_hard_stress.lpp"
    cp "$ROOT/tests/memory_hard_stress.lpp" "$src"
    rm -f "$TMP/memory_hard_stress" "$TMP/memory_hard_stress.o"
    "$COMPILER" "$src" --linker "$linker" >/dev/null
    output=$(run_with_timeout "$TMP/memory_hard_stress")
    printf '%s\n' "$output" | grep -Fq '[Phase 1 Complete]'
    printf '%s\n' "$output" | grep -Fq 'MEMORY STRESS TEST PASSED cleanly in ms:'
    echo "PASS memory_hard_stress ($linker)"
done

# Leak/UAF/double-release coverage for all 100,000 iterations.
src="$TMP/memory_hard_stress.lpp"
rm -f "$TMP/memory_hard_stress.o"
LPP_AOT=1 LPP_AOT_ONLY=1 "$COMPILER" "$src" >/dev/null
"$CC" -O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined \
    "$TMP/memory_hard_stress.o" "$ROOT/lpp_runtime.c" \
    -o "$TMP/memory_hard_stress.asan" -pthread -lm
ASAN_OPTIONS=detect_leaks=1:halt_on_error=1
export ASAN_OPTIONS
run_with_timeout "$TMP/memory_hard_stress.asan" >/dev/null
unset ASAN_OPTIONS
echo 'PASS memory_hard_stress (ASan/UBSan)'

# Generic enum parameters, constructors, managed payload match bindings, and
# generated destructors must work on both backends and both link paths.
for backend in cranelift llvm; do
    if [ "$backend" = llvm ] && ! command -v "$LLVM_CC" >/dev/null 2>&1; then
        echo "SKIP generic_enum_payloads (LLVM compiler '$LLVM_CC' unavailable)"
        continue
    fi
    for linker in host direct; do
        src="$TMP/generic_enum_payloads.lpp"
        cp "$ROOT/tests/generic_enum_payloads.lpp" "$src"
        rm -f "$TMP/generic_enum_payloads" "$TMP/generic_enum_payloads.o"
        if [ "$backend" = llvm ]; then
            LPP_LLVM_CC="$LLVM_CC" "$COMPILER" "$src" --backend llvm --linker "$linker" >/dev/null
        else
            "$COMPILER" "$src" --linker "$linker" >/dev/null
        fi
        output=$("$TMP/generic_enum_payloads")
        [ "$output" = 'OK: generic enum' ] || {
            echo "FAIL generic_enum_payloads ($backend/$linker): $output" >&2
            exit 1
        }
        echo "PASS generic_enum_payloads ($backend/$linker)"
    done
done

echo 'Hard memory regressions: PASS'
