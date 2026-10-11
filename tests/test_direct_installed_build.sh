#!/usr/bin/env sh
# End-user direct-link integration test: installed lpp + packaged min runtime
# must build a project with LPP_LINKER=direct and no host final linker.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP="$ROOT/target/release/lpp"
LINKER="$ROOT/target/release/lpp-link"
CC=${CC:-cc}
TEMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-direct-install.XXXXXX")
cleanup() { rm -rf "$TEMP"; }
trap cleanup EXIT HUP INT TERM

if ! command -v cargo >/dev/null 2>&1 || ! command -v "$CC" >/dev/null 2>&1; then
    echo "SKIP: requires cargo and $CC"
    exit 0
fi
if [ ! -x "$LPP" ] || [ ! -x "$LINKER" ]; then
    (cd "$ROOT" && cargo build --release --locked --bin lpp --bin lpp-link)
fi
if [ ! -f "$ROOT/target/release/liblpp_runtime.so" ] && [ ! -f "$ROOT/target/debug/liblpp_runtime.so" ]; then
    (cd "$ROOT" && cargo build --release --locked -p lpp-runtime)
fi
mkdir -p "$TEMP/install/bin" "$TEMP/install/lib" "$TEMP/work"
cp "$LPP" "$TEMP/install/bin/lpp"
cp "$LINKER" "$TEMP/install/bin/lpp-link"
if [ -f "$ROOT/target/release/liblpp_runtime.so" ]; then
    cp "$ROOT/target/release/liblpp_runtime.so" "$TEMP/install/lib/"
elif [ -f "$ROOT/target/debug/liblpp_runtime.so" ]; then
    cp "$ROOT/target/debug/liblpp_runtime.so" "$TEMP/install/lib/"
fi
"$CC" -Os -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
    -fno-reorder-blocks-and-partition \
    -c "$ROOT/runtime/linux_x86_64_min.c" -o "$TEMP/install/lib/lpp_runtime_min.o"

cd "$TEMP/work"
"$TEMP/install/bin/lpp" new direct-install-demo >/dev/null
cd direct-install-demo
LPP_LINKER=direct "$TEMP/install/bin/lpp" build >/dev/null
BINARY=$(find . \( -name "direct-install-demo" -o -name "direct_install_demo" \) -type f -executable 2>/dev/null | head -1)
OUTPUT=$("$BINARY" 2>/dev/null)
[ "$OUTPUT" = "Hello from L++ project!" ] || [ "$OUTPUT" = "Hello from L++" ]
echo "PASS installed direct linker build"
