#!/usr/bin/env sh
set -eu

PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
INSTALL_DIR=${LPP_INSTALL_DIR:-"$HOME/.lpp"}
BIN_DIR="$INSTALL_DIR/bin"
LIB_DIR="$INSTALL_DIR/lib"
VERSION=${LPP_VERSION:-latest}
case "$VERSION" in
  latest|v*) ;;
  *) VERSION="v$VERSION" ;;
esac

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
ASSET_NAME="${RELEASE_TARGET}.tar.gz"
if [ "$VERSION" = "latest" ]; then
  RELEASE_BASE_URL="https://github.com/samarnever-droid/lplusplus/releases/latest/download"
else
  RELEASE_BASE_URL="https://github.com/samarnever-droid/lplusplus/releases/download/$VERSION"
fi
RELEASE_URL="$RELEASE_BASE_URL/$ASSET_NAME"
CHECKSUM_URL="$RELEASE_BASE_URL/SHA256SUMS"

printf '%s\n' "========================================================"
printf '%s\n' "                 L++ GLOBAL INSTALLER                   "
printf '%s\n' "========================================================"

mkdir -p "$BIN_DIR" "$LIB_DIR"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        return 1
    fi
}

install_release() {
    [ -n "$RELEASE_TARGET" ] || return 1
    command -v curl >/dev/null 2>&1 || return 1
    command -v tar >/dev/null 2>&1 || return 1
    temp=$(mktemp -d "${TMPDIR:-/tmp}/lpp-release.XXXXXX")
    trap 'rm -rf "$temp"' EXIT HUP INT TERM
    printf '%s\n' "[1/3] Downloading L++ $VERSION release asset and checksum manifest..."
    if ! curl -fsSL "$RELEASE_URL" -o "$temp/$ASSET_NAME"; then
        return 1
    fi
    if ! curl -fsSL "$CHECKSUM_URL" -o "$temp/SHA256SUMS"; then
        printf '%s\n' "ERROR: release has no SHA256SUMS manifest; refusing an unverified install." >&2
        return 1
    fi

    expected=$(awk -v asset="$ASSET_NAME" '$2 == asset || $2 == "*" asset { print $1; exit }' "$temp/SHA256SUMS")
    case "$expected" in
        ''|*[!0-9A-Fa-f]* )
            printf '%s\n' "ERROR: SHA256SUMS has no valid digest for $ASSET_NAME." >&2
            return 1
            ;;
    esac
    [ "${#expected}" -eq 64 ] || {
        printf '%s\n' "ERROR: invalid SHA-256 length for $ASSET_NAME." >&2
        return 1
    }
    actual=$(sha256_file "$temp/$ASSET_NAME") || {
        printf '%s\n' "ERROR: sha256sum or shasum is required to verify release assets." >&2
        return 1
    }
    if [ "$(printf '%s' "$actual" | tr 'A-F' 'a-f')" != "$(printf '%s' "$expected" | tr 'A-F' 'a-f')" ]; then
        printf '%s\n' "ERROR: SHA-256 verification failed for $ASSET_NAME." >&2
        return 1
    fi

    # Every member must be unique, remain under the expected package root, and
    # contain no parent traversal. Links and special files are forbidden so
    # extraction cannot redirect a later member outside the temporary tree.
    if ! tar -tzf "$temp/$ASSET_NAME" > "$temp/archive-paths"; then
        printf '%s\n' "ERROR: release archive cannot be listed safely." >&2
        return 1
    fi
    if awk -v root="$RELEASE_TARGET/" '
        index($0, root) != 1 { bad=1 }
        seen[$0]++ { bad=1 }
        { count=split($0, part, "/"); for (i=1; i<=count; i++) if (part[i] == "..") bad=1 }
        END { exit bad ? 0 : 1 }
    ' "$temp/archive-paths"; then
        printf '%s\n' "ERROR: release archive contains an unsafe or duplicate path." >&2
        return 1
    fi
    if tar -tvzf "$temp/$ASSET_NAME" | awk '
        substr($1, 1, 1) != "-" && substr($1, 1, 1) != "d" { bad=1 }
        END { exit bad ? 0 : 1 }
    '; then
        printf '%s\n' "ERROR: release archive contains a link or special file." >&2
        return 1
    fi

    if ! tar --no-same-owner --no-same-permissions -k -xzf "$temp/$ASSET_NAME" -C "$temp"; then
        printf '%s\n' "ERROR: verified release archive extraction failed." >&2
        return 1
    fi
    root="$temp/$RELEASE_TARGET"
    [ -d "$root" ] && [ ! -L "$root" ] || return 1
    [ -d "$root/lib" ] && [ ! -L "$root/lib" ] || return 1
    [ -f "$root/bin/lpp" ] && [ ! -L "$root/bin/lpp" ] && [ -x "$root/bin/lpp" ] || return 1
    [ -f "$root/bin/lpp-link" ] && [ ! -L "$root/bin/lpp-link" ] && [ -x "$root/bin/lpp-link" ] || return 1
    printf '%s\n' "[2/3] Installing verified compiler, linker, and packaged runtimes..."
    cp "$root/bin/lpp" "$BIN_DIR/lpp"
    cp "$root/bin/lpp-link" "$BIN_DIR/lpp-link"
    cp -r "$root/lib/"* "$LIB_DIR/"
    if [ -d "$root/pm" ]; then rm -rf "$INSTALL_DIR/pm"; cp -r "$root/pm" "$INSTALL_DIR/pm"; fi
    if [ -d "$root/registry" ]; then rm -rf "$INSTALL_DIR/registry"; cp -r "$root/registry" "$INSTALL_DIR/registry"; fi
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
    (cd "$PROJECT_DIR" && cargo build --release --locked --features all-arch --bin lpp --bin lpp-link)
    printf '%s\n' "[2/3] Packaging local compiler and runtime objects..."
    cp "$PROJECT_DIR/target/release/lpp" "$BIN_DIR/lpp"
    cp "$PROJECT_DIR/target/release/lpp-link" "$BIN_DIR/lpp-link"
    cp "$PROJECT_DIR/lpp_runtime.c" "$LIB_DIR/lpp_runtime.c"
    if [ -d "$PROJECT_DIR/pm" ]; then rm -rf "$INSTALL_DIR/pm"; cp -r "$PROJECT_DIR/pm" "$INSTALL_DIR/pm"; fi
    if [ -d "$PROJECT_DIR/registry" ]; then rm -rf "$INSTALL_DIR/registry"; cp -r "$PROJECT_DIR/registry" "$INSTALL_DIR/registry"; fi
    if [ -d "$PROJECT_DIR/runtime" ]; then
        cp -r "$PROJECT_DIR/runtime" "$LIB_DIR/runtime"
    fi
    if command -v cc >/dev/null 2>&1; then
        if ! cc -O2 -fPIC -c "$LIB_DIR/lpp_runtime.c" -o "$LIB_DIR/lpp_runtime.o"; then
            printf '%s\n' "ERROR: failed to compile lpp_runtime.c (host runtime)." >&2
            exit 1
        fi
        if [ "$(uname -s):$(uname -m)" = "Linux:x86_64" ]; then
            # Flags must match release.yml and pm.rs auto-rebuild so every install
            # path produces the same freestanding runtime object.
            if ! cc -Os -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
                    -fno-reorder-blocks-and-partition -DLPP_FREESTANDING \
                    -c "$PROJECT_DIR/runtime/linux_x86_64_min.c" -o "$LIB_DIR/lpp_runtime_min.o"; then
                printf '%s\n' "ERROR: failed to compile linux_x86_64_min.c (direct-link runtime)." >&2
                printf '%s\n' "       '--linker direct' builds will not work until this is fixed." >&2
                exit 1
            fi
        fi
    else
        printf '%s\n' "WARNING: no C compiler (cc) found; runtime objects were not prebuilt." >&2
        printf '%s\n' "         The first '--linker direct' build will need gcc to compile them." >&2
    fi
}

if [ "${LPP_FROM_SOURCE:-0}" = "1" ]; then
    install_source
elif install_release; then
    printf '%s\n' "[3/3] Release installation complete."
else
    printf '%s\n' "ERROR: verified release installation failed; no automatic source fallback was attempted." >&2
    printf '%s\n' "       From a trusted source checkout, set LPP_FROM_SOURCE=1 explicitly." >&2
    exit 1
fi

INSTALLED_VERSION="$($BIN_DIR/lpp -v 2>/dev/null || true)"

printf '%s\n' ""
printf '%s\n' "Installed commands: lpp, lpp-link"
printf '%s\n' "Requested release: $VERSION"
if [ -n "${RELEASE_TARGET:-}" ]; then
    printf '%s\n' "Release asset: $RELEASE_TARGET"
    printf '%s\n' "Download URL: $RELEASE_URL"
fi
if [ -n "$INSTALLED_VERSION" ]; then
    printf '%s\n' "Installed version: $INSTALLED_VERSION"
else
    printf '%s\n' "Installed version: unable to execute $BIN_DIR/lpp"
fi
printf '%s\n' "Install path: $INSTALL_DIR"
printf '%s\n' "Add this to your shell profile if needed:"
printf '  export PATH="%s:$PATH"\n' "$BIN_DIR"
