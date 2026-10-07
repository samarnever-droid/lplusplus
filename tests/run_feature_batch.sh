#!/usr/bin/env sh
# Four-feature vertical parity and sanitizer gate.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER="$ROOT/target/release/lpp"
CC=${CC:-cc}
LLVM_CC=${LPP_LLVM_CC:-clang}
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-feature-batch.XXXXXX")
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

[ -x "$COMPILER" ] || { echo "release compiler missing: run cargo build --release" >&2; exit 2; }
command -v "$CC" >/dev/null 2>&1 || { echo "C compiler '$CC' unavailable" >&2; exit 2; }
command -v "$LLVM_CC" >/dev/null 2>&1 || { echo "LLVM compiler '$LLVM_CC' unavailable" >&2; exit 2; }

expected() {
    case "$1" in
        tuple_scalars) printf '42\n1' ;;
        tuple_managed) printf '7\nKhati' ;;
        tuple_nested) printf '3\nthree\n9' ;;
        tuple_struct_managed) printf '5\nowned' ;;
        variadic_strings) printf 'items\none\ntwo\nthree' ;;
        variadic_ints) printf '20' ;;
        variadic_empty) printf 'empty\n0' ;;
        str_slice_zero_copy) printf '3\nh\nhop' ;;
        list_slice_zero_copy) printf '2\n20\n30' ;;
        list_slice_managed) printf 'one' ;;
        slice_known_reader) printf '4' ;;
        slice_to_owned) printf 'copy' ;;
        async_immediate) printf 'async immediate' ;;
        async_await_chain) printf 'ready' ;;
        async_task_drop) printf 'dropped' ;;
        async_double_await) printf 'twice\ntwice' ;;
        feature_batch_combined) printf 'executor\n16\nhop' ;;
        *) return 1 ;;
    esac
}

run_case() {
    name=$1
    want=$(expected "$name")
    src="$TMP/$name.lpp"
    cp "$ROOT/tests/$name.lpp" "$src"

    # Cranelift object + full host runtime.
    LPP_AOT=1 LPP_AOT_ONLY=1 "$COMPILER" "$src" >/dev/null
    "$CC" "$TMP/$name.o" "$ROOT/lpp_runtime.c" -o "$TMP/$name.cl-host" -pthread -lm
    got=$("$TMP/$name.cl-host")
    [ "$got" = "$want" ] || { echo "FAIL $name (Cranelift host): '$got'" >&2; exit 1; }

    # Cranelift + freestanding direct runtime.
    rm -f "$TMP/$name.o" "$TMP/$name"
    "$COMPILER" "$src" --linker direct >/dev/null
    got=$("$TMP/$name")
    [ "$got" = "$want" ] || { echo "FAIL $name (Cranelift direct): '$got'" >&2; exit 1; }

    # LLVM + both link paths.
    for linker in host direct; do
        rm -f "$TMP/$name.o" "$TMP/$name"
        LPP_LLVM_CC="$LLVM_CC" "$COMPILER" "$src" --backend llvm --linker "$linker" >/dev/null
        got=$("$TMP/$name")
        [ "$got" = "$want" ] || { echo "FAIL $name (LLVM $linker): '$got'" >&2; exit 1; }
    done
    echo "PASS $name (Cranelift/LLVM, host/direct)"
}

for name in \
    tuple_scalars tuple_managed tuple_nested tuple_struct_managed \
    variadic_strings variadic_ints variadic_empty \
    str_slice_zero_copy list_slice_zero_copy list_slice_managed slice_known_reader slice_to_owned \
    async_immediate async_await_chain async_task_drop async_double_await \
    feature_batch_combined
do
    run_case "$name"
done

reject_case() {
    name=$1
    diagnostic=$2
    src="$TMP/$name.lpp"
    cp "$ROOT/tests/$name.lpp" "$src"
    rm -f "$TMP/$name" "$TMP/$name.o"
    "$COMPILER" "$src" >"$TMP/$name.stdout" 2>"$TMP/$name.stderr" || true
    [ ! -e "$TMP/$name" ] && [ ! -e "$TMP/$name.o" ] || {
        echo "FAIL $name: rejected source emitted code" >&2; exit 1;
    }
    grep -Fq "$diagnostic" "$TMP/$name.stderr" || {
        echo "FAIL $name: missing diagnostic '$diagnostic'" >&2
        cat "$TMP/$name.stderr" >&2
        exit 1
    }
    echo "PASS $name (rejected)"
}

reject_case tuple_bad_arity "Tuple expressions require arity 2..=4"
reject_case variadic_bad_position "Variadic rest parameter must be the final parameter"
reject_case variadic_bad_type "expects Int, got Str"
reject_case slice_return_rejected "borrowed slice cannot be returned"
reject_case slice_capture_rejected "cannot be captured by a closure"
reject_case slice_spawn_rejected "cannot be captured by a closure"
reject_case slice_source_reassign_rejected "cannot reassign 'text' while a borrowed slice view is live"
reject_case async_blocking_rejected "reaches a blocking call without an adapter"
reject_case async_task_capture_rejected "single-thread confined"

# One combined sanitizer binary covers tuple/list child destruction, the rest
# list handoff, borrowed view checks, task environment/result ownership, and
# executor teardown. Targeted task-drop and double-await cases cover cancellation
# and idempotent polling/result retention.
for name in feature_batch_combined tuple_struct_managed async_task_drop async_double_await list_slice_managed; do
    src="$TMP/$name.lpp"
    cp "$ROOT/tests/$name.lpp" "$src"
    rm -f "$TMP/$name.o"
    LPP_AOT=1 LPP_AOT_ONLY=1 "$COMPILER" "$src" >/dev/null
    "$CC" -O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined \
        "$TMP/$name.o" "$ROOT/lpp_runtime.c" -o "$TMP/$name.asan" -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 "$TMP/$name.asan" >/dev/null
    echo "PASS $name (ASan/UBSan)"
done

"$CC" -O1 -g -fsanitize=thread "$TMP/feature_batch_combined.o" \
    "$ROOT/lpp_runtime.c" -o "$TMP/feature_batch_combined.tsan" -pthread -lm
TSAN_OPTIONS=halt_on_error=1 "$TMP/feature_batch_combined.tsan" >/dev/null
echo "PASS feature_batch_combined (TSan)"
echo "Feature batch: PASS"
