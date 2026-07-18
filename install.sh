#!/usr/bin/env sh
set -eu

INSTALL_DIR=${LPP_INSTALL_DIR:-"$HOME/.lpp"}
BIN_DIR="$INSTALL_DIR/bin"
LIB_DIR="$INSTALL_DIR/lib"

printf '%s\n' "========================================================"
printf '%s\n' "                 L++ DOWNLOAD INSTALLER                 "
printf '%s\n' "========================================================"

printf '\n%s\n' "[1/3] Preparing directories..."
mkdir -p "$BIN_DIR" "$LIB_DIR"

printf '\n%s\n' "[2/3] Fetching runtime files..."
# Download lpp_runtime.c from the repository
curl -sSfL "https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/lpp_runtime.c" -o "$LIB_DIR/lpp_runtime.c"

# On Unix, build lpp from source
if command -v cargo >/dev/null 2>&1; then
    printf '%s\n' "  Cargo found. Building compiler from GitHub source repository..."
    TEMP_DIR=$(mktemp -d)
    git clone --depth 1 "https://github.com/samarnever-droid/lplusplus.git" "$TEMP_DIR" >/dev/null 2>&1
    (cd "$TEMP_DIR" && cargo build --release >/dev/null 2>&1)
    cp "$TEMP_DIR/target/release/lpp" "$BIN_DIR/lpp"
    rm -rf "$TEMP_DIR"
else
    printf '%s\n' "  Warning: cargo not found. Please install Rust and run cargo to compile lpp from source."
    exit 1
fi

if command -v cc >/dev/null 2>&1; then
    printf '%s\n' "  Precompiling runtime library with cc..."
    cc -O2 -c "$LIB_DIR/lpp_runtime.c" -o "$LIB_DIR/lpp_runtime.o" || true
fi

printf '\n%s\n' "[3/3] PATH guidance..."
printf '%s\n' "  Add this to your shell profile (e.g. ~/.bashrc or ~/.zshrc):"
printf '  export PATH="%s:$PATH"\n' "$BIN_DIR"

printf '\n%s\n' "========================================================"
printf '%s\n' "         L++ INSTALLED SUCCESSFULLY. TRY: lpp -h        "
printf '%s\n' "========================================================"
