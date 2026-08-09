#!/usr/bin/env sh
# Verify the WebAssembly backend: every program in tests/wasm/cases compiles
# to a valid wasm32-wasi module and produces the expected stdout under a WASM
# runtime; every program in tests/wasm/reject fails with a clear diagnostic.
#
# Requirements: cargo (to build lpp) and a WebAssembly runtime — wasmtime by
# default, override with LPP_WASM_RUNTIME (e.g. wasmer, node).
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER="$ROOT/target/release/lpp"
RT=${LPP_WASM_RUNTIME:-wasmtime}
CASES="$ROOT/tests/wasm/cases"
REJECT="$ROOT/tests/wasm/reject"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-wasm.XXXXXX")
PASS=0
FAIL=0

cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM

say_ok()   { printf 'ok   %s\n' "$1"; PASS=$((PASS + 1)); }
say_fail() { printf 'FAIL %s\n' "$1"; FAIL=$((FAIL + 1)); }

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to build the compiler" >&2
    exit 2
fi
if ! command -v "$RT" >/dev/null 2>&1; then
    echo "WebAssembly runtime '$RT' not found (set LPP_WASM_RUNTIME)" >&2
    exit 2
fi

if [ ! -x "$COMPILER" ]; then
    echo "[L++] Building release compiler..."
    (cd "$ROOT" && cargo build --release --bin lpp)
fi

run_module() {
    # $1 = .wasm file. Node's WASI CLI differs from wasmtime-like runtimes.
    case "$RT" in
        *node*) node "$ROOT/tests/wasm/node_run.mjs" "$1" ;;
        *)      "$RT" "$1" ;;
    esac
}

# ── Positive cases ───────────────────────────────────────────────────────────
for src in "$CASES"/*.lpp; do
    name=$(basename "$src" .lpp)
    work="$TMP/$name"
    mkdir -p "$work"
    cp "$src" "$work/$name.lpp"
    if ! "$COMPILER" "$work/$name.lpp" --target wasm32-wasi >"$work/compile.log" 2>&1; then
        printf '\n%s\n' "--- compiler log for $name:"
        tail -25 "$work/compile.log" >&2
        say_fail "$name (compiler rejected input): $(tail -1 "$work/compile.log")"
        continue
    fi
    if [ ! -f "$work/$name.wasm" ]; then
        say_fail "$name (no .wasm output written)"
        continue
    fi
    # WebAssembly magic header: \0asm 1.0
    if ! head -c 8 "$work/$name.wasm" | od -An -tx1 | grep -q "00 61 73 6d 01 00 00 00"; then
        say_fail "$name (bad wasm magic header)"
        continue
    fi
    # Optional canned stdin for input() cases; otherwise EOF immediately.
    if [ -f "$CASES/$name.stdin" ]; then
        stdin_file="$CASES/$name.stdin"
    else
        stdin_file=/dev/null
    fi
    if ! run_module "$work/$name.wasm" >"$work/out.txt" 2>"$work/run.log" <"$stdin_file"; then
        # Forensics for blind CI: module structure, runtime trace, partial stdout.
        printf '\n%s\n' "--- runtime log for $name:"
        tail -15 "$work/run.log" >&2
        printf '%s\n' "--- partial stdout for $name:" >&2
        head -c 800 "$work/out.txt" >&2 || true
        printf '\n' >&2
        if command -v python3 >/dev/null 2>&1; then
            printf '%s\n' "--- wasm probe for $name:" >&2
            python3 "$ROOT/tests/wasm/wasm_probe.py" "$work/$name.wasm" >&2 || true
            # If the runtime named a byte offset (decode errors), dump a hex
            # window around it — that is usually the exact bug site.
            OFF=$(grep -o 'offset [0-9][0-9]*' "$work/run.log" | head -1 | grep -o '[0-9][0-9]*' || true)
            if [ -n "${OFF:-}" ]; then
                printf '%s\n' "--- hex window around offset $OFF for $name:" >&2
                python3 "$ROOT/tests/wasm/wasm_probe.py" "$work/$name.wasm" --window "$OFF" 112 >&2 || true
            fi
        fi
        say_fail "$name (runtime error): $(tail -1 "$work/run.log")"
        continue
    fi
    if ! diff -u "$CASES/$name.expected" "$work/out.txt" >"$work/diff.txt" 2>&1; then
        printf '\n%s\n' "--- output mismatch for $name:"
        cat "$work/diff.txt"
        printf '%s\n' "--- raw stdout bytes (od -c) for $name:" >&2
        od -c "$work/out.txt" | head -20 >&2 || true
        say_fail "$name"
        continue
    fi
    say_ok "$name"
done

# ── Reject cases: must fail with a clear WebAssembly diagnostic ──────────────
if [ -d "$REJECT" ]; then
    for src in "$REJECT"/*.lpp; do
        name=$(basename "$src")
        work="$TMP/$name"
        mkdir -p "$work"
        cp "$src" "$work/"
        if "$COMPILER" "$work/$name" --target wasm32-wasi >"$work/compile.log" 2>&1; then
            say_fail "$name (unsupported program compiled successfully)"
            continue
        fi
        if ! grep -qi "WebAssembly" "$work/compile.log"; then
            say_fail "$name (error mentions no WebAssembly cause): $(tail -1 "$work/compile.log")"
            continue
        fi
        say_ok "$name"
    done
fi

# ── Alias triples still take the wasm route ──────────────────────────────────
work="$TMP/alias"
mkdir -p "$work"
printf 'def main():\n    print(7)\n' >"$work/alias.lpp"
for triple in wasm32-wasip1 wasm32-unknown-unknown; do
    if "$COMPILER" "$work/alias.lpp" --target "$triple" >"$work/$triple.log" 2>&1 \
        && [ -f "$work/alias.wasm" ] \
        && [ "$(run_module "$work/alias.wasm")" = "7" ]; then
        say_ok "alias-triple $triple"
    else
        say_fail "alias-triple $triple: $(tail -1 "$work/$triple.log" 2>/dev/null)"
    fi
    rm -f "$work/alias.wasm"
done

# --backend wasm without a triple means wasm32-wasi.
if "$COMPILER" "$work/alias.lpp" --backend wasm >"$work/backend-wasm.log" 2>&1 \
    && [ "$(run_module "$work/alias.wasm")" = "7" ]; then
    say_ok "--backend wasm"
else
    say_fail "--backend wasm: $(tail -1 "$work/backend-wasm.log" 2>/dev/null)"
fi

echo "────────────────────────────────────────"
echo "wasm backend tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
