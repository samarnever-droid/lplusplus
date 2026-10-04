#!/usr/bin/env sh
# Canonical list-element policy parity across frontend, MIR, and both backends.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER="$ROOT/target/release/lpp"
CC=${CC:-cc}
LLVM_CC=${LPP_LLVM_CC:-clang}
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-list-policy.XXXXXX")
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

[ -x "$COMPILER" ] || { echo "release compiler missing" >&2; exit 2; }
command -v "$CC" >/dev/null 2>&1 || { echo "C compiler '$CC' unavailable" >&2; exit 2; }
command -v "$LLVM_CC" >/dev/null 2>&1 || { echo "LLVM compiler '$LLVM_CC' unavailable" >&2; exit 2; }

expected() {
    case "$1" in
        bool_print) printf '1\n0' ;;
        list_bool_for) printf '2' ;;
        list_bool_slice) printf '1' ;;
        list_char) printf '97\n98' ;;
        list_float_set) printf '2.500000' ;;
        list_nested_managed) printf '9' ;;
        list_set_self_alias) printf 'same' ;;
        list_closures) printf '42' ;;
        *) return 1 ;;
    esac
}

for name in \
    bool_print list_bool_for list_bool_slice list_char list_float_set list_nested_managed \
    list_set_self_alias list_closures
do
    want=$(expected "$name")
    src="$TMP/$name.lpp"
    cp "$ROOT/tests/$name.lpp" "$src"
    for backend in cranelift llvm; do
        for linker in host direct; do
            rm -f "$TMP/$name" "$TMP/$name.o"
            if [ "$backend" = llvm ]; then
                LPP_LLVM_CC="$LLVM_CC" "$COMPILER" "$src" --backend llvm --linker "$linker" >/dev/null
            else
                "$COMPILER" "$src" --linker "$linker" >/dev/null
            fi
            got=$("$TMP/$name")
            [ "$got" = "$want" ] || {
                echo "FAIL $name ($backend/$linker): expected '$want', got '$got'" >&2
                exit 1
            }
        done
    done
    echo "PASS $name (Cranelift/LLVM, host/direct)"
done

for name in list_vector_rejected variadic_vector_rejected list_set_bad_type; do
    src="$TMP/$name.lpp"
    cp "$ROOT/tests/$name.lpp" "$src"
    "$COMPILER" "$src" >"$TMP/$name.stdout" 2>"$TMP/$name.stderr" || true
    [ ! -e "$TMP/$name" ] && [ ! -e "$TMP/$name.o" ] || {
        echo "FAIL $name: rejected source emitted code" >&2; exit 1;
    }
    grep -Fq 'cannot safely store element type' "$TMP/$name.stderr" \
        || grep -Fq 'not supported safely' "$TMP/$name.stderr" \
        || grep -Fq 'expects Int, got Str' "$TMP/$name.stderr" || {
            echo "FAIL $name: missing safe list-element diagnostic" >&2
            cat "$TMP/$name.stderr" >&2
            exit 1
        }
    echo "PASS $name (rejected)"
done

for name in \
    list_bool_for list_bool_slice list_float_set list_nested_managed \
    list_set_self_alias list_closures
do
    src="$TMP/$name.lpp"
    rm -f "$TMP/$name.o"
    LPP_AOT=1 LPP_AOT_ONLY=1 "$COMPILER" "$src" >/dev/null
    "$CC" -O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined \
        "$TMP/$name.o" "$ROOT/lpp_runtime.c" -o "$TMP/$name.asan" -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 "$TMP/$name.asan" >/dev/null
    echo "PASS $name (ASan/UBSan)"
done

echo 'List policy regressions: PASS'
