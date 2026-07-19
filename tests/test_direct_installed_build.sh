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
    (cd "$ROOT" && cargo build --release --bin lpp --bin lpp-link)
fi
mkdir -p "$TEMP/install/bin" "$TEMP/install/lib" "$TEMP/work"
cp "$LPP" "$TEMP/install/bin/lpp"
cp "$LINKER" "$TEMP/install/bin/lpp-link"
"$CC" -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
    -c "$ROOT/runtime/linux_x86_64_min.c" -o "$TEMP/install/lib/lpp_runtime_min.o"

cd "$TEMP/work"
"$TEMP/install/bin/lpp" new direct_install_demo >/dev/null
cd direct_install_demo
LPP_LINKER=direct "$TEMP/install/bin/lpp" build >/dev/null
OUTPUT=$(target/release/direct_install_demo)
[ "$OUTPUT" = "Hello from L++ project!" ]
echo "PASS installed direct linker build"
