#!/usr/bin/env sh
# Phase 2 direct-linker MVP integration test.
# Produces Linux x86-64 ELF executables without invoking cc/gcc/clang at final
# link time. The freestanding runtime object provides syscall-backed integer
# output and validates GOT runtime-import resolution.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
LINKER="$ROOT/target/release/lpp-link"
CC=${CC:-cc}
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-link-elf.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1 || ! command -v "$CC" >/dev/null 2>&1; then
    echo "SKIP: requires cargo and $CC"
    exit 0
fi
if [ ! -x "$LPP" ] || [ ! -x "$LINKER" ]; then
    (cd "$ROOT" && cargo build --release --bin lpp --bin lpp-link)
fi

# Runtime-free smoke executable.
cat > "$TEMP/no_runtime.lpp" <<'EOF'
def main():
    x := 1
EOF
LPP_AOT=1 "$LPP" "$TEMP/no_runtime.lpp" >/dev/null
"$LINKER" "$TEMP/no_runtime.o" -o "$TEMP/no_runtime"
[ "$("$TEMP/no_runtime"; echo $?)" = "0" ]
file "$TEMP/no_runtime" | grep -q "ELF 64-bit"

# Freestanding runtime import: lpp_print_int is resolved through a merged GOT.
"$CC" -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
    -c "$ROOT/runtime/linux_x86_64_min.c" -o "$TEMP/lpp_runtime_min.o"
LPP_AOT=1 "$LPP" "$ROOT/benchmarks/bench_fib.lpp" >/dev/null
cp "$ROOT/benchmarks/bench_fib.o" "$TEMP/bench_fib.o"
"$LINKER" "$TEMP/bench_fib.o" "$TEMP/lpp_runtime_min.o" -o "$TEMP/fib"
[ "$("$TEMP/fib")" = "9227465" ]

# Read-only data: string literal is merged and resolved through GOTPCREL.
cat > "$TEMP/string.lpp" <<'EOF'
def main():
    print_str("hello linker")
EOF
LPP_AOT=1 "$LPP" "$TEMP/string.lpp" >/dev/null
"$LINKER" "$TEMP/string.o" "$TEMP/lpp_runtime_min.o" -o "$TEMP/string"
[ "$("$TEMP/string")" = "hello linker" ]
echo "PASS direct ELF linker MVP with freestanding runtime and rodata"
