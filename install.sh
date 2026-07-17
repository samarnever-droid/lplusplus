#!/usr/bin/env sh
set -eu

PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
INSTALL_DIR=${LPP_INSTALL_DIR:-"$HOME/.lpp"}
BIN_DIR="$INSTALL_DIR/bin"
LIB_DIR="$INSTALL_DIR/lib"

printf '%s\n' "========================================================"
printf '%s\n' "                 L++ GLOBAL INSTALLER                   "
printf '%s\n' "========================================================"

printf '\n%s\n' "[1/4] Building release compiler..."
(cd "$PROJECT_DIR" && cargo build --release)

printf '\n%s\n' "[2/4] Preparing install directories..."
mkdir -p "$BIN_DIR" "$LIB_DIR"

printf '\n%s\n' "[3/4] Installing compiler and runtime files..."
cp "$PROJECT_DIR/target/release/lpp" "$BIN_DIR/lpp"
cp "$PROJECT_DIR/lpp_runtime.c" "$LIB_DIR/lpp_runtime.c"

if command -v cc >/dev/null 2>&1; then
    printf '%s\n' "  Precompiling runtime object with cc..."
    cc -O2 -c "$LIB_DIR/lpp_runtime.c" -o "$LIB_DIR/lpp_runtime.o" || true
fi

printf '\n%s\n' "[4/4] PATH guidance..."
printf '%s\n' "  Add this to your shell profile if needed:"
printf '  export PATH="%s:$PATH"\n' "$BIN_DIR"

printf '\n%s\n' "========================================================"
printf '%s\n' "             L++ INSTALLED. TRY: lpp -h                 "
printf '%s\n' "========================================================"
