#!/usr/bin/env sh
set -eu

if [ "$#" -ne 1 ]; then
    printf '%s\n' "Usage: ./run.sh <file.lpp>" >&2
    exit 1
fi

FILE=$1
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
COMPILER="$SCRIPT_DIR/target/release/lpp"
RUNTIME_SRC="$SCRIPT_DIR/lpp_runtime.c"

if [ ! -f "$FILE" ]; then
    printf "Source file '%s' not found.\n" "$FILE" >&2
    exit 1
fi

if [ ! -x "$COMPILER" ]; then
    printf '%s\n' "[L++] Building release compiler..."
    (cd "$SCRIPT_DIR" && cargo build --release)
fi

printf '[L++] Compiling %s to object file...\n' "$FILE"
LPP_AOT=1 BENCHMARK=1 "$COMPILER" "$FILE"

OBJ_FILE=${FILE%.*}.o
EXE_FILE=${FILE%.*}
if [ ! -f "$OBJ_FILE" ]; then
    printf "Object file was not generated at '%s'.\n" "$OBJ_FILE" >&2
    exit 1
fi

if command -v cc >/dev/null 2>&1; then
    printf '%s\n' "[L++] Linking with cc..."
    cc -O2 "$OBJ_FILE" "$RUNTIME_SRC" -o "$EXE_FILE" -pthread
elif command -v gcc >/dev/null 2>&1; then
    printf '%s\n' "[L++] Linking with gcc..."
    gcc -O2 "$OBJ_FILE" "$RUNTIME_SRC" -o "$EXE_FILE" -pthread
elif command -v clang >/dev/null 2>&1; then
    printf '%s\n' "[L++] Linking with clang..."
    clang -O2 "$OBJ_FILE" "$RUNTIME_SRC" -o "$EXE_FILE" -pthread
else
    printf '%s\n' "No supported C compiler found. Install cc, gcc, or clang." >&2
    exit 1
fi

rm -f "$OBJ_FILE"
printf '%s\n\n' "[L++] Running compiled program:"
"$EXE_FILE"
STATUS=$?
rm -f "$EXE_FILE"
exit $STATUS
