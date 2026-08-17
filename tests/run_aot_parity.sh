#!/usr/bin/env sh
# Verify that the supported L++ subset has identical C and Cranelift-AOT output.
# Requirements: cargo, cc (or gcc/clang), and a POSIX shell.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
MANIFEST="$ROOT/tests/aot_parity.tsv"
COMPILER="$ROOT/target/release/lpp"
LINKER="$ROOT/target/release/lpp-link"
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

echo "[L++] Building release compiler and linker..."
(cd "$ROOT" && cargo build --release --bin lpp --bin lpp-link)

# Compile each runtime once.  Recompiling the large compatibility runtime for
# every corpus entry was the dominant cost of this action.
HOST_RUNTIME_OBJ="$TMP/lpp_runtime_host.o"
"$CC" -std=c11 -Wall -Wextra -Wno-unused-function -Wno-unused-variable -Wno-unused-parameter \
    -c "$ROOT/lpp_runtime.c" -o "$HOST_RUNTIME_OBJ" -pthread
DIRECT_RUNTIME_OBJ=""
if [ "$(uname -s)" = "Linux" ] && [ "$(uname -m)" = "x86_64" ] && [ -x "$LINKER" ]; then
    DIRECT_RUNTIME_OBJ="$TMP/lpp_runtime_min.o"
    "$CC" -Os -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
        -fno-reorder-blocks-and-partition -c "$ROOT/runtime/linux_x86_64_min.c" \
        -o "$DIRECT_RUNTIME_OBJ"
fi

run_native_aot() {
    src=$1
    base=$2
    LPP_AOT=1 "$COMPILER" "$src" --aot >/dev/null
    obj_file="${src%.lpp}.o"
    exe="$TMP/${base}.aot.exe"
    [ -f "$obj_file" ] || { echo "AOT backend produced no object file" >&2; return 1; }
    "$CC" -std=c11 -Wall -Wextra -Wno-unused-function -Wno-unused-variable -Wno-unused-parameter "$obj_file" "$HOST_RUNTIME_OBJ" -o "$exe" -pthread -lm
    "$exe"
}

run_direct_link() {
    src=$1
    base=$2
    exe="${src%.lpp}"
    rm -f "$exe" "${exe}.exe"
    "$COMPILER" "$src" --linker direct >/dev/null
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

# Compile independent programs concurrently.  The old serial loop spent more
# than the CI job's 20-minute budget on the full ownership corpus even though
# every case has an isolated source/object/executable path. Keep the default
# at four workers and allow slower machines to opt down to one.
RESULT_DIR="$TMP/results"
mkdir -p "$RESULT_DIR"
PARITY_JOBS=${LPP_PARITY_JOBS:-4}
case "$PARITY_JOBS" in
    ''|*[!0-9]*|0) PARITY_JOBS=1 ;;
esac

run_case() {
    file=$(printf '%s' "$1" | tr -d '\r')
    expected=$(printf '%s' "$2" | tr -d '\r')
    src="$TMP/$file"
    base=${file%.lpp}
    result="$RESULT_DIR/$base.result"
    cp "$ROOT/tests/$file" "$src"

    if aot_output=$(run_native_aot "$src" "$base") && direct_output=$(run_direct_link "$src" "$base"); then
        wanted=$(printf '%b' "$expected")
        if [ "$aot_output" = "$wanted" ] && [ "$direct_output" = "$wanted" ]; then
            printf 'PASS %s\n' "$file" > "$result"
        else
            {
                printf 'FAIL %s: backend output mismatch\n' "$file"
                printf '  expected: %s\n  AOT:      %s\n  Direct:   %s\n' "$wanted" "$aot_output" "$direct_output"
            } > "$result"
        fi
    else
        printf 'FAIL %s: compile, link, or execution failed\n' "$file" > "$result"
    fi
}

pids=""
running=0
while IFS='|' read -r file expected; do
    case "$file" in ''|\#*) continue ;; esac
    run_case "$file" "$expected" &
    pids="$pids $!"
    running=$((running + 1))
    if [ "$running" -ge "$PARITY_JOBS" ]; then
        for pid in $pids; do wait "$pid" || true; done
        pids=""
        running=0
    fi
done < "$MANIFEST"
for pid in $pids; do wait "$pid" || true; done

for result in "$RESULT_DIR"/*.result; do
    cat "$result"
    if grep -q '^PASS ' "$result"; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
done

# SAFETY-CONTRACT CHANGE: the two ARC-cycle cases used to be *rejection*
# contracts -- "AOT must refuse a strong ownership cycle". They are now positive
# cases in the manifest above (cycle_broken_node.lpp, cycle_broken_list.lpp),
# because analysis::cyclebreak demotes one edge of every cycle to non-owning, so
# no owning cycle can be built and the structures are reclaimed normally.
# Leak-freedom is preserved and was re-verified under AddressSanitizer with
# 50 000 genuine runtime cycles; what changed is that trees, linked lists and
# parent pointers are now expressible.
for rejected_case in \
    "aot_reject_mut_closure:Cannot mutate captured variable"
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
