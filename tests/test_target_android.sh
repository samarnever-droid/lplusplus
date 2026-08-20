#!/usr/bin/env sh
# Android / Termux target-triple support.
#   * `--target aarch64-linux-android` emits a real AArch64 ELF object (cross
#     compilation), independent of the build host arch.
#   * `--list-targets` lists the known Android/Termux triples.
#   * host targets still build and run normally.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER=${LPP:-$ROOT/target/release/lpp}
[ -x "$COMPILER" ] || COMPILER="$ROOT/target/debug/lpp"
[ -x "$COMPILER" ] || { echo "lpp compiler not found; set LPP" >&2; exit 2; }
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-target-android.XXXXXX")
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

cat > "$TMP/prog.lpp" <<'EOF'
def main():
    print("hello-android")
EOF

# 1. --list-targets prints Android triples.
"$COMPILER" --list-targets | grep -q "aarch64-linux-android"
"$COMPILER" --list-targets | grep -q "armv7-linux-androideabi"
echo "PASS --list-targets advertises Android/Termux triples"

# 2. Cross-compile to an AArch64 ELF object (written as prog.o next to source).
(cd "$TMP" && "$COMPILER" prog.lpp --target aarch64-linux-android --emit-object >/dev/null 2>&1)
OBJ="$TMP/prog.o"
[ -f "$OBJ" ] || { echo "FAIL: no aarch64 object emitted" >&2; exit 1; }
if command -v readelf >/dev/null 2>&1; then
    readelf -h "$OBJ" | grep -q "AArch64" || { echo "FAIL: object is not AArch64" >&2; exit 1; }
fi
echo "PASS --target aarch64-linux-android emits an AArch64 ELF object"

# 3. Host target still builds and runs.
(cd "$TMP" && "$COMPILER" prog.lpp --target x86_64-unknown-linux-gnu --linker host >/dev/null 2>&1)
BIN="$TMP/prog"
[ -x "$BIN" ] || { echo "FAIL: host target binary not produced" >&2; exit 1; }
OUT=$("$BIN")
[ "$OUT" = "hello-android" ] || { echo "FAIL: host binary output '$OUT' != hello-android" >&2; exit 1; }
echo "PASS host target still builds and runs"

echo "c2lpp android/termux target tests: PASS"
