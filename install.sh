#!/usr/bin/env sh
# ==============================================================================
# L++ Compiler & Toolchain Fast Installer (Linux & macOS)
# Hosted on Cloudflare Edge CDN: https://lplusplus.bond/install.sh
# ==============================================================================
set -eu

INSTALL_DIR="${LPP_INSTALL_DIR:-"$HOME/.lpp"}"
BIN_DIR="$INSTALL_DIR/bin"
LIB_DIR="$INSTALL_DIR/lib"
VERSION="${LPP_VERSION:-v4.7.0}"

case "$VERSION" in
  latest|v*) ;;
  *) VERSION="v$VERSION" ;;
esac

ARCH="$(uname -m)"
OS="$(uname -s)"
RELEASE_TARGET=""

case "$OS:$ARCH" in
  Linux:x86_64|Linux:amd64)
    RELEASE_TARGET="lpp-linux-x86_64"
    ;;
  Linux:aarch64|Linux:arm64)
    RELEASE_TARGET="lpp-linux-aarch64"
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
LATEST_URL="https://github.com/samarnever-droid/lplusplus/releases/latest/download/${RELEASE_TARGET}.tar.gz"

printf '%s\n' "\033[1;32m========================================================\033[0m"
printf '%s\n' "\033[1;32m       L++ Compiler & Toolchain Global Installer        \033[0m"
printf '%s\n' "\033[1;32m========================================================\033[0m"

mkdir -p "$BIN_DIR" "$LIB_DIR"

try_download() {
    [ -n "$RELEASE_TARGET" ] || return 1
    command -v curl >/dev/null 2>&1 || return 1
    command -v tar >/dev/null 2>&1 || return 1
    
    temp=$(mktemp -d "${TMPDIR:-/tmp}/lpp-install.XXXXXX")
    trap 'rm -rf "$temp"' EXIT HUP INT TERM
    
    printf '%s\n' "[1/3] Downloading L++ $VERSION binary asset from CDN..."
    if ! curl -fsSL "$RELEASE_URL" -o "$temp/lpp.tar.gz" 2>/dev/null; then
        if ! curl -fsSL "$LATEST_URL" -o "$temp/lpp.tar.gz" 2>/dev/null; then
            return 1
        fi
    fi
    
    tar -xzf "$temp/lpp.tar.gz" -C "$temp"
    root="$temp/$RELEASE_TARGET"
    if [ ! -f "$root/bin/lpp" ] && [ -f "$temp/bin/lpp" ]; then
        root="$temp"
    fi
    
    [ -f "$root/bin/lpp" ] || return 1
    chmod +x "$root/bin/lpp"
    if [ -f "$root/bin/lpp-link" ]; then chmod +x "$root/bin/lpp-link"; fi
    
    printf '%s\n' "[2/3] Installing binary components to $BIN_DIR..."
    cp "$root/bin/lpp" "$BIN_DIR/lpp"
    if [ -f "$root/bin/lpp-link" ]; then cp "$root/bin/lpp-link" "$BIN_DIR/lpp-link"; fi
    if [ -d "$root/lib" ]; then cp -r "$root/lib/"* "$LIB_DIR/" 2>/dev/null || true; fi
    if [ -d "$root/pm" ]; then cp -r "$root/pm" "$INSTALL_DIR/pm" 2>/dev/null || true; fi
    
    rm -rf "$temp"
    trap - EXIT HUP INT TERM
    return 0
}

try_cargo_install() {
    if ! command -v cargo >/dev/null 2>&1; then
        printf '%s\n' "\033[1;31mError: Rust/Cargo is required when prebuilt binaries are unavailable.\033[0m" >&2
        printf '%s\n' "Install Rust via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
        return 1
    fi
    printf '%s\n' "[1/3] Compiling L++ toolchain from official repository..."
    cargo install --git https://github.com/samarnever-droid/lplusplus --root "$INSTALL_DIR" --force --bin lpp --bin lpp-link
    return 0
}

if try_download; then
    printf '%s\n' "\033[1;32m[3/3] ✓ Prebuilt release installation complete.\033[0m"
else
    printf '%s\n' "\033[1;33mPrebuilt binary unavailable for $OS:$ARCH, installing via Cargo...\033[0m"
    try_cargo_install
    printf '%s\n' "\033[1;32m[3/3] ✓ Source compilation installation complete.\033[0m"
fi

# Wire PATH in shell profile
ADD_PATH="export PATH=\"$BIN_DIR:\$PATH\""
for profile in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
    if [ -f "$profile" ] && ! grep -q "$BIN_DIR" "$profile"; then
        printf '\n# L++ Compiler Toolchain\n%s\n' "$ADD_PATH" >> "$profile"
    fi
done

printf '\n\033[1;32mInstallation Success!\033[0m\n'
printf 'Binary Path: %s/lpp\n' "$BIN_DIR"
printf 'Run: \033[1;36mexport PATH="%s:$PATH"\033[0m (or restart your shell)\n' "$BIN_DIR"
printf 'Verify: \033[1;36mlpp --help\033[0m or \033[1;36mlpp upgrade --check\033[0m\n'
