#!/usr/bin/env sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP=${LPP:-lpp}

command -v "$LPP" >/dev/null 2>&1 || [ -x "$LPP" ] || {
    echo "L++ compiler unavailable: $LPP" >&2
    exit 2
}

cd "$ROOT"
rm -rf generated/sqlite3 generated/zlib src/main src/main.exe src/main.o
"$LPP" src/main.lpp --linker host >/dev/null
EXE=src/main
[ -x "$EXE" ] || EXE=src/main.exe
[ -x "$EXE" ] || { echo 'c2lpp executable was not produced' >&2; exit 1; }

C2LPP_HEADER=fixtures/sqlite3_api.h \
C2LPP_NAME=sqlite3 C2LPP_LIB=sqlite3 C2LPP_OUT=generated/sqlite3 \
C2LPP_CPP=0 "$EXE" >/dev/null

SQL=generated/sqlite3/src/bindings.lpp
grep -Fq 'const SQLITE_OK = 0' "$SQL"
grep -Fq 'def sqlite3_open(filename: Str, ppDb: Int) -> Int' "$SQL"
grep -Fq 'def sqlite3_errmsg(db: Int) -> Str' "$SQL"
grep -Fq 'def sqlite3_exec(db: Int, sql: Str, callback: Int, context: Int, error_message: Int) -> Int' "$SQL"
grep -Fq '<stddef.h>' generated/sqlite3/c2lpp.dependencies.txt
"$LPP" "$SQL" --check >/dev/null

echo 'PASS SQLite header -> L++ native package'

C2LPP_HEADER=fixtures/zlib_api.h \
C2LPP_NAME=zlib C2LPP_LIB=z C2LPP_OUT=generated/zlib \
C2LPP_CPP=0 "$EXE" >/dev/null

ZLIB=generated/zlib/src/bindings.lpp
grep -Fq 'const Z_OK = 0' "$ZLIB"
grep -Fq 'def zlibVersion() -> Str' "$ZLIB"
grep -Fq 'def compress(dest: Int, destLen: Int, source: Int, sourceLen: Int) -> Int' "$ZLIB"
grep -Fq 'def compressBound(sourceLen: Int) -> Int' "$ZLIB"
grep -Fq 'link "z"' "$ZLIB"
"$LPP" "$ZLIB" --check >/dev/null

echo 'PASS zlib header -> L++ native package'

# Phase 2 audited scalar C subset -> pure L++ source.
rm -rf generated/scalar
C2LPP_MODE=translate C2LPP_SOURCE=fixtures/scalar_algorithms.c \
C2LPP_NAME=scalar_algorithms C2LPP_OUT=generated/scalar "$EXE" >/dev/null
TRANSLATED=generated/scalar/src/translated.lpp
grep -Fq 'def sum_to(n: Int) -> Int:' "$TRANSLATED"
grep -Fq 'while i < n:' "$TRANSLATED"
grep -Fq 'i = i + 1' "$TRANSLATED"
grep -Fq 'for i in range(0, n):' "$TRANSLATED"
grep -Fq 'values := [2, 4, 6, 8]' "$TRANSLATED"
grep -Fq 'list_set(values, 2, 10)' "$TRANSLATED"
! grep -Fq 'c2lpp phase2 unsupported:' "$TRANSLATED"
"$LPP" "$TRANSLATED" --check >/dev/null
cat > generated/scalar/smoke.lpp <<'EOF'
import translated

def main():
    print(sum_to(10))
    print(sum_for(10))
    print(absolute_value(-7))
    print(clamp(50, 0, 10))
    print(array_score())
EOF
(cd generated/scalar && "$LPP" smoke.lpp --linker direct >/dev/null)
RESULT=$(generated/scalar/smoke)
[ "$RESULT" = '45
45
7
10
24' ]
if command -v cc >/dev/null 2>&1; then
    cc tests/scalar_reference.c -o generated/scalar/c-reference
    C_RESULT=$(generated/scalar/c-reference)
    [ "$RESULT" = "$C_RESULT" ]
fi
echo 'PASS Phase 2 scalar C -> pure L++ translation/native equivalence smoke'

rm -rf generated/unsupported
C2LPP_MODE=translate C2LPP_SOURCE=fixtures/unsupported_pointer.c \
C2LPP_NAME=unsupported_sample C2LPP_OUT=generated/unsupported C2LPP_STRICT=1 \
"$EXE" >/dev/null
UNSUPPORTED=$(sed -n 's/^unsupported_constructs=//p' generated/unsupported/c2lpp.translation-report.txt)
[ "$UNSUPPORTED" -ge 2 ]
grep -Fq 'c2lpp phase2 unsupported' generated/unsupported/src/translated.lpp
"$LPP" generated/unsupported/src/translated.lpp --check >/dev/null
echo "PASS Phase 2 unsupported-code quarantine/report ($UNSUPPORTED markers)"

# Optional real-system end-to-end gates. They prove that generated packages
# compile, link, and call the installed native libraries rather than only
# matching fixture text.
if [ -f /usr/include/sqlite3.h ]; then
    rm -rf generated/sqlite3-system
    C2LPP_HEADER=/usr/include/sqlite3.h \
    C2LPP_NAME=sqlite3_system C2LPP_LIB=sqlite3 C2LPP_OUT=generated/sqlite3-system \
    C2LPP_CPP=1 "$EXE" >/dev/null
    defs=$(grep -c '^    def ' generated/sqlite3-system/bindings.lpp)
    [ "$defs" -ge 200 ]
    cat > generated/sqlite3-system/smoke.lpp <<'EOF'
import bindings

def main():
    print(sqlite3_libversion())
    print(sqlite3_libversion_number())
EOF
    (cd generated/sqlite3-system && "$LPP" smoke.lpp --linker host >/dev/null && ./smoke >/dev/null)
    echo "PASS system SQLite header/native smoke ($defs declarations)"
fi

if [ -f /usr/include/zlib.h ]; then
    rm -rf generated/zlib-system
    C2LPP_HEADER=/usr/include/zlib.h \
    C2LPP_NAME=zlib_system C2LPP_LIB=z C2LPP_OUT=generated/zlib-system \
    C2LPP_CPP=1 "$EXE" >/dev/null
    defs=$(grep -c '^    def ' generated/zlib-system/bindings.lpp)
    [ "$defs" -ge 50 ]
    cat > generated/zlib-system/smoke.lpp <<'EOF'
import bindings

def main():
    print(zlibVersion())
EOF
    (cd generated/zlib-system && "$LPP" smoke.lpp --linker host >/dev/null && ./smoke >/dev/null)
    echo "PASS system zlib header/native smoke ($defs declarations)"
fi

echo 'c2lpp tests: PASS'
