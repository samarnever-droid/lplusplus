#!/usr/bin/env bash
# Restore the build toolchain after a sandbox reset (system state is not
# persisted across resets — only the workspace is). Run from anywhere:
#   bash scripts/restore_toolchain.sh
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

# 1. rustup/rustc/cargo (the reset also drops exec bits on ~/.cargo/bin)
chmod +x "$HOME"/.cargo/bin/* 2>/dev/null || true
if ! cargo --version >/dev/null 2>&1; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  chmod +x "$HOME"/.cargo/bin/* 2>/dev/null || true
  export PATH="$HOME/.cargo/bin:$PATH"
fi
# rust-toolchain.toml pins 1.98.0
if ! rustc --version | grep -q '1\.98\.0'; then
  rustup toolchain install 1.98.0 --profile minimal
fi
# llc (used by the clang shim below)
rustup component add llvm-tools --toolchain 1.98.0 || true

# 2. System tools. clang is NOT installable in this sandbox (no root for
#    apt), so instead of clang we rely on the persisted shim at
#    /home/user/.tools/bin/clang (llc-based; see that file's header).
command -v cc  >/dev/null 2>&1 || { echo "cc missing — cannot proceed" >&2; exit 1; }
command -v node >/dev/null 2>&1 || echo "note: node missing — wasm execution gates will skip/fail"
command -v rg   >/dev/null 2>&1 || echo "note: rg missing — use grep"

# 3. Make sure the clang shim is present and linked to llc.
TOOLSBIN="/home/user/.tools/bin"
if [ ! -x "$TOOLSBIN/clang" ]; then
  mkdir -p "$TOOLSBIN"
  cat > "$TOOLSBIN/clang" <<'SHIM'
#!/bin/sh
# clang shim: maps `clang -c -w -O0 in.ll -o out.o` (the only invocation
# the L++ LLVM backend makes) onto llc. Refuses anything else loudly.
set -eu
LLC="$(dirname "$0")/llc"
in_ll=""
out=""
expect_out=0
for a in "$@"; do
  if [ "$expect_out" = "1" ]; then out="$a"; expect_out=0; continue; fi
  case "$a" in
    -o) expect_out=1 ;;
    -c|-w|-O0|-O1|-O2|-O3) ;;
    *.ll) in_ll="$a" ;;
    *) echo "clang shim: unsupported argument '$a' (only '-c -w -O0 in.ll -o out.o')" >&2; exit 2 ;;
  esac
done
if [ -z "$in_ll" ] || [ -z "$out" ]; then
  echo "clang shim: unsupported invocation (only '-c -w -O0 in.ll -o out.o')" >&2
  exit 2
fi
exec "$LLC" -filetype=obj -O0 -relocation-model=pic "$in_ll" -o "$out"
SHIM
  chmod +x "$TOOLSBIN/clang"
fi
LLTBIN=$(ls -d "$HOME"/.rustup/toolchains/1.98.0-*/lib/rustlib/x86_64-unknown-linux-gnu/bin 2>/dev/null | head -1 || true)
if [ -n "$LLTBIN" ] && [ -x "$LLTBIN/llc" ]; then
  ln -sf "$LLTBIN/llc" "$TOOLSBIN/llc"
fi

echo "toolchain ready: $(cargo --version) / $(rustc --version | awk '{print $2}')"
echo "export PATH=\"$TOOLSBIN:\$HOME/.cargo/bin:\$PATH\""
