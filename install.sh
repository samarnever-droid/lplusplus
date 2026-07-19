#!/usr/bin/env sh
set -eu

PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
INSTALL_DIR=${LPP_INSTALL_DIR:-"$HOME/.lpp"}
BIN_DIR="$INSTALL_DIR/bin"
LIB_DIR="$INSTALL_DIR/lib"
VERSION=${LPP_VERSION:-v0.1.2}

case "$(uname -s):$(uname -m)" in
  Linux:x86_64|Linux:amd64)
    RELEASE_TARGET="lpp-linux-x86_64"
    ;;
  Darwin:arm64)
    RELEASE_TARGET="lpp-macos-arm64"
    ;;
  Darwin:x86_64)
    RELEASE_TARGET="lpp-macos-x86_64"
    ;;
  *)
    RELEASE_TARGET=""
    ;;
esac
RELEASE_URL="https://github.com/samarnever-droid/lplusplus/releases/download/$VERSION/${RELEASE_TARGET}.tar.gz"

printf '%s\n' "========================================================"
printf '%s\n' "                 L++ GLOBAL INSTALLER                   "
printf '%s\n' "========================================================"

mkdir -p "$BIN_DIR" "$LIB_DIR"

install_release() {
    [ -n "$RELEASE_TARGET" ] || return 1
    command -v curl >/dev/null 2>&1 || return 1
    command -v tar >/dev/null 2>&1 || return 1
    temp=$(mktemp -d "${TMPDIR:-/tmp}/lpp-release.XXXXXX")
    trap 'rm -rf "$temp"' EXIT HUP INT TERM
    printf '%s\n' "[1/3] Downloading L++ $VERSION release..."
    if ! curl -fsSL "$RELEASE_URL" -o "$temp/lpp.tar.gz"; then
        return 1
    fi
    tar -xzf "$temp/lpp.tar.gz" -C "$temp"
    root="$temp/$RELEASE_TARGET"
    [ -x "$root/bin/lpp" ] || return 1
    printf '%s\n' "[2/3] Installing compiler, linker, and packaged runtimes..."
    cp "$root/bin/lpp" "$BIN_DIR/lpp"
    cp "$root/bin/lpp-link" "$BIN_DIR/lpp-link"
    cp "$root/lib/"* "$LIB_DIR/"
    rm -rf "$temp"
    trap - EXIT HUP INT TERM
    return 0
}

install_source() {
    command -v cargo >/dev/null 2>&1 || {
        printf '%s\n' "Rust/Cargo is required for source installation. Use the release installer path or install Rust." >&2
        exit 1
    }
    printf '%s\n' "[1/3] Building L++ compiler and linker from source..."
    (cd "$PROJECT_DIR" && cargo build --release --bin lpp --bin lpp-link)
    printf '%s\n' "[2/3] Packaging local compiler and runtime objects..."
    cp "$PROJECT_DIR/target/release/lpp" "$BIN_DIR/lpp"
    cp "$PROJECT_DIR/target/release/lpp-link" "$BIN_DIR/lpp-link"
    cp "$PROJECT_DIR/lpp_runtime.c" "$LIB_DIR/lpp_runtime.c"
    if command -v cc >/dev/null 2>&1; then
        cc -O2 -fPIC -c "$LIB_DIR/lpp_runtime.c" -o "$LIB_DIR/lpp_runtime.o"
        if [ "$(uname -s):$(uname -m)" = "Linux:x86_64" ]; then
            cc -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
                -c "$PROJECT_DIR/runtime/linux_x86_64_min.c" -o "$LIB_DIR/lpp_runtime_min.o" || true
        fi
    fi
}

if [ "${LPP_FROM_SOURCE:-0}" = "1" ]; then
    install_source
elif install_release; then
    printf '%s\n' "[3/3] Release installation complete."
else
    printf '%s\n' "Release asset unavailable; falling back to local source installation." >&2
    install_source
    printf '%s\n' "[3/3] Source installation complete."
fi

printf '%s\n' "  Add this to your shell profile if needed:"
printf '  export PATH="%s:$PATH"\n' "$BIN_DIR"
printf '%s\n' "Installed commands: lpp, lpp-link"
