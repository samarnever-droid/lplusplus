#!/usr/bin/env bash
# Focused LeakSanitizer regression for c_memory's stale-context tombstone.
# The tombstone remains readable after c_memory_destroy, then the runtime
# reclaims it at process shutdown. Both the ordinary and stale-handle paths
# are exercised; LSan remains enabled and no safety assertion is weakened.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LPP="${LPP:-$ROOT/target/debug/lpp}"
CC="${CC:-cc}"
if [[ ! -x "$LPP" ]]; then
    echo "bindgen LSan gate: compiler is not executable: $LPP" >&2
    exit 2
fi
if ! command -v "$CC" >/dev/null 2>&1; then
    echo "bindgen LSan gate: C compiler not found: $CC" >&2
    exit 2
fi

WORK=$(mktemp -d "${TMPDIR:-/tmp}/lpp-bindgen-lsan.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
cp "$ROOT/packages/lpp-bindgen/src/c_memory.lpp" "$WORK/c_memory.lpp"

cat > "$WORK/context_ok.lpp" <<'EOF'
import c_memory

def main():
    memory := c_memory_new(4)
    pointer := c_malloc(memory, 4)
    c_store_u32(pointer, 42)
    print(c_load_u32(pointer))
    c_memory_destroy(memory)
EOF

cat > "$WORK/context_stale.lpp" <<'EOF'
import c_memory

def main():
    memory := c_memory_new(4)
    pointer := c_malloc(memory, 4)
    c_memory_destroy(memory)
    print(c_load_u8(pointer))
EOF

compile_and_link() {
    local case_name="$1"
    (
        cd "$WORK"
        LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" "$case_name.lpp" >/dev/null
    )
    "$CC" -O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined \
        "$WORK/$case_name.o" "$ROOT/lpp_runtime.c" \
        -o "$WORK/$case_name.asan" -pthread -lm
}

compile_and_link context_ok
ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 \
UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1 \
    "$WORK/context_ok.asan" > "$WORK/context_ok.out"
if [[ "$(cat "$WORK/context_ok.out")" != "42" ]]; then
    echo "bindgen LSan gate: unexpected clean-context output" >&2
    cat "$WORK/context_ok.out" >&2
    exit 1
fi

compile_and_link context_stale
set +e
ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 \
UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1 \
    "$WORK/context_stale.asan" > "$WORK/context_stale.out" 2>&1
status=$?
set -e
if [[ "$status" -ne 101 ]]; then
    echo "bindgen LSan gate: stale context must exit 101 (got $status)" >&2
    cat "$WORK/context_stale.out" >&2
    exit 1
fi
if ! grep -Fq 'C2-MEM-CONTEXT-DESTROYED' "$WORK/context_stale.out"; then
    echo "bindgen LSan gate: missing stale-context diagnostic" >&2
    cat "$WORK/context_stale.out" >&2
    exit 1
fi
if grep -Eqi 'AddressSanitizer|LeakSanitizer|runtime error:' "$WORK/context_stale.out"; then
    echo "bindgen LSan gate: sanitizer reported an error" >&2
    cat "$WORK/context_stale.out" >&2
    exit 1
fi

echo 'PASS bindgen context destroy/stale-handle ASan+LSan+UBSan'
