#!/usr/bin/env sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
REPO=$(CDPATH= cd -- "$ROOT/../.." && pwd)
LPP=${LPP:-lpp}
SQLITE_AUDIT=0
if [ "${1:-}" = "--sqlite-audit" ]; then
    SQLITE_AUDIT=1
elif [ "$#" -gt 0 ]; then
    echo "unknown test option: $1" >&2
    exit 2
fi

command -v "$LPP" >/dev/null 2>&1 || [ -x "$LPP" ] || {
    echo "L++ compiler unavailable: $LPP" >&2
    exit 2
}

cd "$ROOT"
rm -rf generated/sqlite3 generated/zlib src/main src/main.exe src/main.o c2lpp.json
"$LPP" src/main.lpp --linker host >/dev/null
EXE=src/main
[ -x "$EXE" ] || EXE=src/main.exe
[ -x "$EXE" ] || { echo 'c2lpp executable was not produced' >&2; exit 1; }

write_config() {
    mode=$1
    input_path=$2
    manifest_path=$3
    package_name=$4
    library_name=$5
    output_path=$6
    strict_value=$7
    preprocess_value=$8
    source_version=${9:-}
    source_sha256=${10:-}
    cat > c2lpp.json <<EOF
{
  "schema": "c2lpp-project",
  "schema_version": 1,
  "mode": "$mode",
  "input": "$input_path",
  "manifest": "$manifest_path",
  "name": "$package_name",
  "library": "$library_name",
  "output": "$output_path",
  "strict": $strict_value,
  "preprocess": $preprocess_value,
  "compiler": "cc",
  "source_version": "$source_version",
  "source_sha256": "$source_sha256"
}
EOF
}

# Strict JSON schema: unknown, duplicate, and mistyped settings must fail before
# any translation work begins.
cat > c2lpp.json <<'EOF'
{"schema":"c2lpp-project","schema_version":1,"mode":"bindings","input":"fixtures/sqlite3_api.h","unknown":1}
EOF
if "$EXE" > generated-config-error.txt 2>&1; then
    echo 'unknown JSON config key unexpectedly succeeded' >&2
    exit 1
fi
grep -Fq 'C2-CONFIG-UNKNOWN-KEY:unknown' generated-config-error.txt
cat > c2lpp.json <<'EOF'
{"schema":"c2lpp-project","schema_version":1,"mode":"bindings","mode":"audit","input":"fixtures/sqlite3_api.h"}
EOF
if "$EXE" > generated-config-error.txt 2>&1; then
    echo 'duplicate JSON config key unexpectedly succeeded' >&2
    exit 1
fi
grep -Fq 'C2-CONFIG-DUPLICATE-KEY:mode' generated-config-error.txt
cat > c2lpp.json <<'EOF'
{"schema":"c2lpp-project","schema_version":1,"mode":"bindings","input":"fixtures/sqlite3_api.h","strict":"true"}
EOF
if "$EXE" > generated-config-error.txt 2>&1; then
    echo 'mistyped JSON config value unexpectedly succeeded' >&2
    exit 1
fi
grep -Fq 'C2-CONFIG-TYPE:strict' generated-config-error.txt
rm -f generated-config-error.txt
echo 'PASS strict versioned JSON config validation'

# Curated pure-L++ SQLite backend: functional substitution, explicitly not a
# sqlite3.c translation. Generated package is standalone and contains no C/FFI.
rm -rf generated/sqlite-backend
write_config sqlite-backend fixtures/sqlite3_api.h '' sqlite_backend sqlite_backend generated/sqlite-backend true false
"$EXE" >/dev/null
BACKEND_REPORT=generated/sqlite-backend/c2lpp.translation-report.txt
BACKEND_LPP=generated/sqlite-backend/translated.lpp
grep -Fq 'functional_backend=1' "$BACKEND_REPORT"
grep -Fq 'curated_backend_substitution=1' "$BACKEND_REPORT"
grep -Fq 'source_translation_complete=0' "$BACKEND_REPORT"
grep -Fq 'extern_blocks=0' "$BACKEND_REPORT"
grep -Fq 'native_links=0' "$BACKEND_REPORT"
grep -Fq 'vendored_modules=20' "$BACKEND_REPORT"
! grep -R -Fq 'extern "C"' generated/sqlite-backend --include='*.lpp'
! grep -R -Fq 'link "' generated/sqlite-backend --include='*.lpp'
if find generated/sqlite-backend -type f \( -name '*.c' -o -name '*.h' \) | grep -q .; then
    echo 'curated backend unexpectedly contains C source/header' >&2
    exit 1
fi
BACKEND_CHECK=$("$LPP" "$BACKEND_LPP" --check 2>&1)
printf '%s\n' "$BACKEND_CHECK" | grep -Fq 'L++ check: OK'
cat > generated/sqlite-backend/smoke.lpp <<'EOF'
import translated

def run(db: Int, sql: Str) -> Void:
    result := sqlite_native_exec(db, sql)
    if sqlite_native_error(result) != 0:
        print_str(sqlite_native_error_message(result))
        exit(3)
    sqlite_native_result_free(result)

def main():
    delete_file("curated.db")
    db := sqlite_native_open("curated.db")
    run(db, "CREATE TABLE items (id INTEGER PRIMARY KEY, value INTEGER)")
    run(db, "INSERT INTO items VALUES (1, 10), (2, 20), (3, 30)")
    result := sqlite_native_exec(db, "SELECT COUNT(*), SUM(value) FROM items")
    print_str(sqlite_native_cell(result, 0, 0))
    print_str(sqlite_native_cell(result, 0, 1))
    sqlite_native_result_free(result)
    sqlite_native_close(db)
EOF
(cd generated/sqlite-backend && "$LPP" smoke.lpp --linker host >/dev/null && ./smoke > smoke.out)
[ "$(cat generated/sqlite-backend/smoke.out)" = '3
60' ]
if command -v python3 >/dev/null 2>&1; then
    python3 - <<'PY'
import sqlite3
p='generated/sqlite-backend/curated.db'
db=sqlite3.connect(p)
assert db.execute('pragma integrity_check').fetchone()[0]=='ok'
assert db.execute('select count(*),sum(value) from items').fetchone()==(3,60)
db.close()
PY
fi
echo 'PASS curated standalone pure-L++ SQLite backend CRUD/integrity (not source translation)'

write_config bindings fixtures/sqlite3_api.h '' sqlite3 sqlite3 generated/sqlite3 false false
"$EXE" >/dev/null

SQL=generated/sqlite3/src/bindings.lpp
grep -Fq 'const SQLITE_OK = 0' "$SQL"
grep -Fq 'def sqlite3_open(filename: Str, ppDb: Int) -> Int' "$SQL"
grep -Fq 'def sqlite3_errmsg(db: Int) -> Str' "$SQL"
grep -Fq 'def sqlite3_exec(db: Int, sql: Str, callback: Int, context: Int, error_message: Int) -> Int' "$SQL"
grep -Fq '<stddef.h>' generated/sqlite3/c2lpp.dependencies.txt
NORMALIZED_CONFIG=generated/sqlite3/c2lpp.config.normalized.json
grep -Fq '"schema": "c2lpp-project"' "$NORMALIZED_CONFIG"
grep -Fq '"schema_version": 1' "$NORMALIZED_CONFIG"
grep -Fq '"preprocess": false' "$NORMALIZED_CONFIG"
cp "$NORMALIZED_CONFIG" generated/sqlite3/config-first.json
"$EXE" >/dev/null
cmp generated/sqlite3/config-first.json "$NORMALIZED_CONFIG"
"$LPP" "$SQL" --check >/dev/null

echo 'PASS SQLite header -> L++ native package + deterministic JSON config'

write_config bindings fixtures/zlib_api.h '' zlib z generated/zlib false false
"$EXE" >/dev/null

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
write_config translate fixtures/scalar_algorithms.c '' scalar_algorithms scalar_algorithms generated/scalar false false
"$EXE" >/dev/null
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

# Token parser -> typed normalized IR -> pure L++ emitter. Every function in
# this fixture must be accepted, compiled, executed, and matched to native C.
rm -rf generated/typed-scalar
write_config translate-ir fixtures/typed_scalar.c '' typed_scalar typed_scalar generated/typed-scalar true false
"$EXE" >/dev/null
TYPED=generated/typed-scalar/src/translated.lpp
TYPED_IR=generated/typed-scalar/c2lpp.normalized-ir.txt
TYPED_REPORT=generated/typed-scalar/c2lpp.translation-report.txt
grep -Fq 'engine=typed-normal-ir-v1' "$TYPED_REPORT"
grep -Fq 'functions_total=3' "$TYPED_REPORT"
grep -Fq 'functions_translated=3' "$TYPED_REPORT"
grep -Fq 'functions_rejected=0' "$TYPED_REPORT"
grep -Fq 'schema|c2lpp-normal-ir-v1' "$TYPED_IR"
grep -Fq 'function|add_scaled|int' "$TYPED_IR"
grep -Fq 'stmt|local|product|int|binary|*|name|a|name|b' "$TYPED_IR"
grep -Fq 'function|call_chain|int' "$TYPED_IR"
! grep -Fq 'c2lpp typed unsupported' "$TYPED"
"$LPP" "$TYPED" --check >/dev/null
cat > generated/typed-scalar/smoke.lpp <<'EOF'
import translated

def main():
    print(add_scaled(7, 3))
    print(bit_mix(9))
    print(call_chain(5))
EOF
(cd generated/typed-scalar && "$LPP" smoke.lpp --linker direct >/dev/null)
TYPED_RESULT=$(generated/typed-scalar/smoke)
[ "$TYPED_RESULT" = '28
39
38' ]
if command -v cc >/dev/null 2>&1; then
    cc tests/typed_scalar_reference.c -o generated/typed-scalar/c-reference
    TYPED_C_RESULT=$(generated/typed-scalar/c-reference)
    [ "$TYPED_RESULT" = "$TYPED_C_RESULT" ]
fi
echo 'PASS typed token parser/normalized IR/L++ emission/native equivalence'

rm -rf generated/function-sweep
write_config sweep fixtures/typed_scalar.c '' function_sweep function_sweep generated/function-sweep true false
"$EXE" >/dev/null
SWEEP_REPORT=generated/function-sweep/c2lpp.function-sweep-report.txt
SWEEP_LPP=generated/function-sweep/src/translated.lpp
grep -Fq 'total_functions=3' "$SWEEP_REPORT"
grep -Fq 'eligible_functions=3' "$SWEEP_REPORT"
grep -Fq 'translated_functions=3' "$SWEEP_REPORT"
grep -Fq 'rejected_functions=0' "$SWEEP_REPORT"
! grep -Fq 'extern "C"' "$SWEEP_LPP"
"$LPP" "$SWEEP_LPP" --check >/dev/null
echo 'PASS automatic simple-function semantic sweep'

rm -rf generated/pointer-sweep
write_config sweep fixtures/pointer_signatures.c '' pointer_sweep pointer_sweep generated/pointer-sweep true false
"$EXE" >/dev/null
POINTER_SWEEP_REPORT=generated/pointer-sweep/c2lpp.function-sweep-report.txt
POINTER_SWEEP_LPP=generated/pointer-sweep/translated.lpp
grep -Fq 'total_functions=2' "$POINTER_SWEEP_REPORT"
grep -Fq 'translated_functions=2' "$POINTER_SWEEP_REPORT"
grep -Fq 'rejected_functions=0' "$POINTER_SWEEP_REPORT"
POINTER_CHECK=$("$LPP" "$POINTER_SWEEP_LPP" --check 2>&1)
printf '%s\n' "$POINTER_CHECK" | grep -Fq 'L++ check: OK'
cat > generated/pointer-sweep/smoke.lpp <<'EOF'
import translated

def main():
    memory := c_memory_new(4)
    pointer := c_malloc(memory, 4)
    print(pointer_nonnull(pointer))
    print(c_ptr_is_null(pointer_null()))
    c_free(pointer)
    c_memory_destroy(memory)
EOF
(cd generated/pointer-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
POINTER_RESULT=$(generated/pointer-sweep/smoke)
[ "$POINTER_RESULT" = '1
1' ]
if command -v cc >/dev/null 2>&1; then
    cc tests/pointer_signatures_reference.c -o generated/pointer-sweep/c-reference
    POINTER_C_RESULT=$(generated/pointer-sweep/c-reference)
    [ "$POINTER_C_RESULT" = "$POINTER_RESULT" ]
fi
echo 'PASS aggregate-pointer signatures/null semantics -> pure L++'

rm -rf generated/cast-sweep
write_config sweep fixtures/casts.c '' cast_sweep cast_sweep generated/cast-sweep true false
"$EXE" >/dev/null
CAST_REPORT=generated/cast-sweep/c2lpp.function-sweep-report.txt
CAST_LPP=generated/cast-sweep/translated.lpp
grep -Fq 'total_functions=3' "$CAST_REPORT"
grep -Fq 'translated_functions=3' "$CAST_REPORT"
grep -Fq 'rejected_functions=0' "$CAST_REPORT"
CAST_CHECK=$("$LPP" "$CAST_LPP" --check 2>&1)
printf '%s\n' "$CAST_CHECK" | grep -Fq 'L++ check: OK'
cat > generated/cast-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(cast_down(7.75))
    print(cast_up(9))
    print(cast_truth(-3))
EOF
(cd generated/cast-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
CAST_RESULT=$(generated/cast-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/casts_reference.c -o generated/cast-sweep/c-reference
    CAST_C_RESULT=$(generated/cast-sweep/c-reference)
    [ "$CAST_RESULT" = "$CAST_C_RESULT" ]
fi
echo 'PASS primitive C casts -> typed pure L++ native equivalence'

rm -rf generated/expression-sweep
write_config sweep fixtures/expression_statements.c '' expression_sweep expression_sweep generated/expression-sweep true false
"$EXE" >/dev/null
EXPR_REPORT=generated/expression-sweep/c2lpp.function-sweep-report.txt
EXPR_LPP=generated/expression-sweep/translated.lpp
grep -Fq 'total_functions=2' "$EXPR_REPORT"
grep -Fq 'translated_functions=2' "$EXPR_REPORT"
grep -Fq 'rejected_functions=0' "$EXPR_REPORT"
EXPR_CHECK=$("$LPP" "$EXPR_LPP" --check 2>&1)
printf '%s\n' "$EXPR_CHECK" | grep -Fq 'L++ check: OK'
cat > generated/expression-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(call_wrapper(3))
EOF
(cd generated/expression-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
EXPR_RESULT=$(generated/expression-sweep/smoke)
[ "$EXPR_RESULT" = '5' ]
if command -v cc >/dev/null 2>&1; then
    cc tests/expression_statements_reference.c -o generated/expression-sweep/c-reference
    EXPR_C_RESULT=$(generated/expression-sweep/c-reference)
    [ "$EXPR_RESULT" = "$EXPR_C_RESULT" ]
fi
echo 'PASS forward calls/expression statements/void casts -> pure L++'

rm -rf generated/if-block-sweep
write_config sweep fixtures/if_blocks.c '' if_block_sweep if_block_sweep generated/if-block-sweep true false
"$EXE" >/dev/null
IF_BLOCK_REPORT=generated/if-block-sweep/c2lpp.function-sweep-report.txt
IF_BLOCK_LPP=generated/if-block-sweep/translated.lpp
grep -Fq 'total_functions=1' "$IF_BLOCK_REPORT"
grep -Fq 'translated_functions=1' "$IF_BLOCK_REPORT"
grep -Fq 'rejected_functions=0' "$IF_BLOCK_REPORT"
IF_BLOCK_CHECK=$("$LPP" "$IF_BLOCK_LPP" --check 2>&1)
printf '%s\n' "$IF_BLOCK_CHECK" | grep -Fq 'L++ check: OK'
cat > generated/if-block-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(adjust_value(-5))
    print(adjust_value(3))
EOF
(cd generated/if-block-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
IF_BLOCK_RESULT=$(generated/if-block-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/if_blocks_reference.c -o generated/if-block-sweep/c-reference
    IF_BLOCK_C_RESULT=$(generated/if-block-sweep/c-reference)
    [ "$IF_BLOCK_RESULT" = "$IF_BLOCK_C_RESULT" ]
fi
echo 'PASS non-return if/else blocks -> pure L++ native equivalence'

rm -rf generated/while-sweep
write_config sweep fixtures/while_blocks.c '' while_sweep while_sweep generated/while-sweep true false
"$EXE" >/dev/null
WHILE_REPORT=generated/while-sweep/c2lpp.function-sweep-report.txt
WHILE_LPP=generated/while-sweep/translated.lpp
grep -Fq 'total_functions=2' "$WHILE_REPORT"
grep -Fq 'translated_functions=2' "$WHILE_REPORT"
grep -Fq 'rejected_functions=0' "$WHILE_REPORT"
WHILE_CHECK=$("$LPP" "$WHILE_LPP" --check 2>&1)
printf '%s\n' "$WHILE_CHECK" | grep -Fq 'L++ check: OK'
cat > generated/while-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(sum_while(10))
    print(normalize_nonzero(7))
EOF
(cd generated/while-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
WHILE_RESULT=$(generated/while-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/while_blocks_reference.c -o generated/while-sweep/c-reference
    WHILE_C_RESULT=$(generated/while-sweep/c-reference)
    [ "$WHILE_RESULT" = "$WHILE_C_RESULT" ]
fi
echo 'PASS recursive while blocks/C truthiness -> pure L++ native equivalence'

# Braced do/while loops lower to an explicit first-iteration loop and bottom
# condition. Bodies containing continue remain fail-closed until a condition
# trampoline is available.
rm -rf generated/do-while-sweep
write_config translate-ir fixtures/do_while.c '' do_while_sweep do_while_sweep generated/do-while-sweep true false
"$EXE" >/dev/null
DO_WHILE_REPORT=generated/do-while-sweep/c2lpp.translation-report.txt
DO_WHILE_IR=generated/do-while-sweep/c2lpp.normalized-ir.txt
grep -Fq 'functions_translated=3' "$DO_WHILE_REPORT"
grep -Fq 'functions_rejected=0' "$DO_WHILE_REPORT"
grep -Fq 'stmt|do-while|' "$DO_WHILE_IR"
cat > generated/do-while-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(sum_do(5))
    print(sum_do(0))
    print(break_do(4))
    print(break_do(20))
    print(once_do(27))
EOF
(cd generated/do-while-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
DO_WHILE_RESULT=$(generated/do-while-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/do_while_reference.c -o generated/do-while-sweep/c-reference
    [ "$DO_WHILE_RESULT" = "$(generated/do-while-sweep/c-reference)" ]
fi
echo 'PASS braced do/while and break -> native equivalence'

# Unbraced if/else call statements are parsed as typed expression statements.
rm -rf generated/unbraced-call-sweep
write_config sweep fixtures/unbraced_calls.c '' unbraced_call_sweep unbraced_call_sweep generated/unbraced-call-sweep true false
"$EXE" >/dev/null
UNBRACED_REPORT=generated/unbraced-call-sweep/c2lpp.function-sweep-report.txt
UNBRACED_IR=generated/unbraced-call-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=4' "$UNBRACED_REPORT"
grep -Fq 'rejected_functions=0' "$UNBRACED_REPORT"
grep -Fq 'stmt|if-single|' "$UNBRACED_IR"
grep -Fq 'stmt|else-single' "$UNBRACED_IR"
cat > generated/unbraced-call-sweep/smoke.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(2)
    value := c_calloc(memory, 1, 4)
    c_store_u32(value, 5)
    maybe_set(0, value, 11)
    print(c_load_i32(value))
    maybe_set(1, value, 13)
    print(c_load_i32(value))
    either_set(1, value, 17, 19)
    print(c_load_i32(value))
    either_set(0, value, 17, 19)
    print(c_load_i32(value))
    print(read_after_set(1, value))
    c_free(value)
    c_memory_destroy(memory)
EOF
(cd generated/unbraced-call-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
UNBRACED_RESULT=$(generated/unbraced-call-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/unbraced_calls_reference.c -o generated/unbraced-call-sweep/c-reference
    [ "$UNBRACED_RESULT" = "$(generated/unbraced-call-sweep/c-reference)" ]
fi
echo 'PASS unbraced if/else call statements -> call closure/native equivalence'

# The call-closure fixpoint emits a function only once every callee it references
# is also emitted. A chain of small translated helpers (c1->c2->c3) all come
# out together; a wrapper around a large non-translated body is rejected with
# C2-SWEEP-CALL-CLOSURE instead of emitting a dangling call.
rm -rf generated/call-closure-chain
write_config sweep fixtures/call_closure_chain.c '' call_closure_chain call_closure_chain generated/call-closure-chain true false
"$EXE" >/dev/null
CHAIN_REPORT=generated/call-closure-chain/c2lpp.function-sweep-report.txt
grep -Fq 'translated_functions=4' "$CHAIN_REPORT"
grep -Fq 'rejected_functions=0' "$CHAIN_REPORT"
cat > generated/call-closure-chain/smoke.lpp <<'EOF'
import translated
def main():
    print(entry(5))
EOF
(cd generated/call-closure-chain && "$LPP" smoke.lpp --linker direct >/dev/null)
CHAIN_RESULT=$(generated/call-closure-chain/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/call_closure_chain_reference.c -o generated/call-closure-chain/c-reference
    [ "$CHAIN_RESULT" = "$(generated/call-closure-chain/c-reference)" ]
fi
echo 'PASS call-closure fixpoint (transitive helper chains) -> native equivalence'

# Unbraced if/else assignment, compound-assignment, ternary-assignment and
# dereference-assignment statements are parsed as single statements.
rm -rf generated/unbraced-assign-sweep
write_config sweep fixtures/unbraced_assignments.c '' unbraced_assign_sweep unbraced_assign_sweep generated/unbraced-assign-sweep true false
"$EXE" >/dev/null
UNBRACED_ASSIGN_REPORT=generated/unbraced-assign-sweep/c2lpp.function-sweep-report.txt
UNBRACED_ASSIGN_IR=generated/unbraced-assign-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=4' "$UNBRACED_ASSIGN_REPORT"
grep -Fq 'rejected_functions=0' "$UNBRACED_ASSIGN_REPORT"
grep -Fq 'stmt|if-single|' "$UNBRACED_ASSIGN_IR"
grep -Fq 'stmt|else-single' "$UNBRACED_ASSIGN_IR"
grep -Fq 'stmt|assign|n|=' "$UNBRACED_ASSIGN_IR"
grep -Fq 'place-assign|+=' "$UNBRACED_ASSIGN_IR"
grep -Fq 'stmt|assign|n|=|conditional|' "$UNBRACED_ASSIGN_IR"
grep -Fq 'place-assign|+=|place-load|dereference' "$UNBRACED_ASSIGN_IR"
cat > generated/unbraced-assign-sweep/smoke.lpp <<'EOF'
import translated
import c_memory
def main():
    memory := c_memory_new(2)
    v := c_calloc(memory, 1, 4)
    c_store_u32(v, 25)
    print(clamp_value(3,10))
    print(clamp_value(20,10))
    print(adjust(-5))
    print(adjust(4))
    print(pick(0))
    print(pick(1))
    print(bump(v))
    c_free(v)
    c_memory_destroy(memory)
EOF
(cd generated/unbraced-assign-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
UNBRACED_ASSIGN_RESULT=$(generated/unbraced-assign-sweep/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/unbraced_assignments_reference.c -o generated/unbraced-assign-sweep/c-reference
    [ "$UNBRACED_ASSIGN_RESULT" = "$(generated/unbraced-assign-sweep/c-reference)" ]
fi
echo 'PASS unbraced if/else assignment statements -> native equivalence'

rm -rf generated/for-sweep
write_config sweep fixtures/for_blocks.c '' for_sweep for_sweep generated/for-sweep true false
"$EXE" >/dev/null
FOR_REPORT=generated/for-sweep/c2lpp.function-sweep-report.txt
FOR_LPP=generated/for-sweep/translated.lpp
grep -Fq 'total_functions=3' "$FOR_REPORT"
grep -Fq 'translated_functions=3' "$FOR_REPORT"
grep -Fq 'rejected_functions=0' "$FOR_REPORT"
FOR_CHECK=$("$LPP" "$FOR_LPP" --check 2>&1)
printf '%s\n' "$FOR_CHECK" | grep -Fq 'L++ check: OK'
cat > generated/for-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(sum_odd_steps(9))
    print(count_down(5))
    print(sum_adjusted(6))
EOF
(cd generated/for-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
FOR_RESULT=$(generated/for-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/for_blocks_reference.c -o generated/for-sweep/c-reference
    FOR_C_RESULT=$(generated/for-sweep/c-reference)
    [ "$FOR_RESULT" = "$FOR_C_RESULT" ]
fi
echo 'PASS canonical for loops/scoped indices/steps -> pure L++ native equivalence'

# `for` increment forms: `i = i + 1` (plain reassignment), `i--` and `i += n`.
rm -rf generated/for-increment-sweep
write_config sweep fixtures/for_increment_forms.c '' for_increment_sweep for_increment_sweep generated/for-increment-sweep true false
"$EXE" >/dev/null
FORINC_REPORT=generated/for-increment-sweep/c2lpp.function-sweep-report.txt
grep -Fq 'translated_functions=3' "$FORINC_REPORT"
grep -Fq 'rejected_functions=0' "$FORINC_REPORT"
cat > generated/for-increment-sweep/smoke.lpp <<'EOF'
import translated
def main():
    print(f(5))
    print(g(5))
    print(h(5))
EOF
(cd generated/for-increment-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
FORINC_RESULT=$(generated/for-increment-sweep/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/for_increment_forms_reference.c -o generated/for-increment-sweep/c-reference
    [ "$FORINC_RESULT" = "$(generated/for-increment-sweep/c-reference)" ]
fi
echo 'PASS for-loop increment forms (i=i+1 / i-- / i+=n) -> native equivalence'

# Bitwise compound assignments on local scalars: `x <<= 2`, `x >>= 1`,
# `x &= 3`, `x |= 1`, `x ^= 5`.
rm -rf generated/bitwise-compound
write_config sweep fixtures/bitwise_compound.c '' bitwise_compound_sweep bitwise_compound_sweep generated/bitwise-compound true false
"$EXE" >/dev/null
BITWISE_COMPOUND_REPORT=generated/bitwise-compound/c2lpp.function-sweep-report.txt
grep -Fq 'translated_functions=2' "$BITWISE_COMPOUND_REPORT"
grep -Fq 'rejected_functions=0' "$BITWISE_COMPOUND_REPORT"
cat > generated/bitwise-compound/smoke.lpp <<'EOF'
import translated
def main():
    print(f1(3))
    print(f1(5))
    print(f6(29))
    print(f6(8))
EOF
(cd generated/bitwise-compound && "$LPP" smoke.lpp --linker direct >/dev/null)
BITWISE_COMPOUND_RESULT=$(generated/bitwise-compound/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/bitwise_compound_reference.c -o generated/bitwise-compound/c-reference
    [ "$BITWISE_COMPOUND_RESULT" = "$(generated/bitwise-compound/c-reference)" ]
fi
echo 'PASS bitwise compound assignments (x <<= / >>= / &= / |= / ^=) -> native equivalence'

rm -rf generated/loop-control-sweep
write_config sweep fixtures/loop_control.c '' loop_control_sweep loop_control_sweep generated/loop-control-sweep true false
"$EXE" >/dev/null
LOOP_CONTROL_REPORT=generated/loop-control-sweep/c2lpp.function-sweep-report.txt
LOOP_CONTROL_LPP=generated/loop-control-sweep/translated.lpp
grep -Fq 'total_functions=2' "$LOOP_CONTROL_REPORT"
grep -Fq 'translated_functions=2' "$LOOP_CONTROL_REPORT"
grep -Fq 'rejected_functions=0' "$LOOP_CONTROL_REPORT"
LOOP_CONTROL_CHECK=$("$LPP" "$LOOP_CONTROL_LPP" --check 2>&1)
printf '%s\n' "$LOOP_CONTROL_CHECK" | grep -Fq 'L++ check: OK'
cat > generated/loop-control-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(for_control(10))
    print(while_control(12))
EOF
(cd generated/loop-control-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
LOOP_CONTROL_RESULT=$(generated/loop-control-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/loop_control_reference.c -o generated/loop-control-sweep/c-reference
    LOOP_CONTROL_C_RESULT=$(generated/loop-control-sweep/c-reference)
    [ "$LOOP_CONTROL_RESULT" = "$LOOP_CONTROL_C_RESULT" ]
fi
echo 'PASS loop break/continue targets -> pure L++ native equivalence'

# Loop idioms: empty for-increment (`for(i=0;i<n;)`), `while(1)` with unbraced
# `if (...) break;`, and `i = i - 1` for-increment.
rm -rf generated/loop-idioms
write_config sweep fixtures/loop_idioms.c '' loop_idioms_sweep loop_idioms_sweep generated/loop-idioms true false
"$EXE" >/dev/null
LOOP_IDIOMS_REPORT=generated/loop-idioms/c2lpp.function-sweep-report.txt
grep -Fq 'translated_functions=3' "$LOOP_IDIOMS_REPORT"
grep -Fq 'rejected_functions=0' "$LOOP_IDIOMS_REPORT"
cat > generated/loop-idioms/smoke.lpp <<'EOF'
import translated
def main():
    print(f1(5))
    print(f2(5))
    print(f3(5))
EOF
(cd generated/loop-idioms && "$LPP" smoke.lpp --linker direct >/dev/null)
LOOP_IDIOMS_RESULT=$(generated/loop-idioms/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/loop_idioms_reference.c -o generated/loop-idioms/c-reference
    [ "$LOOP_IDIOMS_RESULT" = "$(generated/loop-idioms/c-reference)" ]
fi
echo 'PASS loop idioms (empty for-increment / while(1)+break / i=i-1) -> native equivalence'

# Return-position conditional expressions lower to real branches, preserving C's
# lazy arm evaluation rather than eagerly evaluating both operands.
rm -rf generated/ternary-return-sweep
write_config sweep fixtures/ternary_returns.c '' ternary_return_sweep ternary_return_sweep generated/ternary-return-sweep true false
"$EXE" >/dev/null
TERNARY_REPORT=generated/ternary-return-sweep/c2lpp.function-sweep-report.txt
TERNARY_IR=generated/ternary-return-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=7' "$TERNARY_REPORT"
grep -Fq 'rejected_functions=0' "$TERNARY_REPORT"
grep -Fq 'stmt|ternary-return|condition|' "$TERNARY_IR"
cat > generated/ternary-return-sweep/smoke.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(2)
    value := c_calloc(memory, 1, 4)
    c_store_u32(value, 37)
    print(choose_int(1, 5, 9))
    print(choose_int(0, 5, 9))
    print(choose_comparison(3, 7))
    print(choose_comparison(8, 9))
    print(choose_float(1, 2.5, 8.75))
    print(choose_float(0, 2.5, 8.75))
    print(c2lpp_bool_to_int(c_ptr_equal(choose_pointer(1, value), value)))
    print(c_ptr_is_null(choose_pointer(0, value)))
    print(load_or(value, 11))
    print(load_or(c_ptr_null(), 11))
    print(both_nonnull(value, value))
    print(both_nonnull(value, c_ptr_null()))
    print(guarded_value(value))
    print(guarded_value(c_ptr_null()))
    c_free(value)
    c_memory_destroy(memory)
EOF
(cd generated/ternary-return-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
TERNARY_RESULT=$(generated/ternary-return-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/ternary_returns_reference.c -o generated/ternary-return-sweep/c-reference
    [ "$TERNARY_RESULT" = "$(generated/ternary-return-sweep/c-reference)" ]
fi
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/ternary-return-sweep/sanitize-probe - >/dev/null 2>&1; then
    (cd generated/ternary-return-sweep && rm -f smoke.o && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" smoke.lpp >/dev/null)
    cc -fsanitize=address,undefined -fno-omit-frame-pointer \
        generated/ternary-return-sweep/smoke.o "$REPO/lpp_runtime.c" \
        -o generated/ternary-return-sweep/smoke.asan -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        generated/ternary-return-sweep/smoke.asan >/dev/null
fi
echo 'PASS lazy ternary returns and pointer logical short-circuit -> native equivalence + sanitizers'

# Statement-context conditionals preserve lazy evaluation for local initialization,
# ordinary/compound assignment and pointer/null selection. Character constants
# are normalized to their C integer values.
rm -rf generated/conditional-context-sweep
write_config sweep fixtures/conditional_contexts.c '' conditional_context_sweep conditional_context_sweep generated/conditional-context-sweep true false
"$EXE" >/dev/null
CONDITIONAL_CONTEXT_REPORT=generated/conditional-context-sweep/c2lpp.function-sweep-report.txt
CONDITIONAL_CONTEXT_IR=generated/conditional-context-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=6' "$CONDITIONAL_CONTEXT_REPORT"
grep -Fq 'rejected_functions=0' "$CONDITIONAL_CONTEXT_REPORT"
grep -Fq 'stmt|local|selected|int|conditional|' "$CONDITIONAL_CONTEXT_IR"
grep -Fq 'stmt|assign|selected|=|conditional|' "$CONDITIONAL_CONTEXT_IR"
grep -Fq 'character|95' "$CONDITIONAL_CONTEXT_IR"
cat > generated/conditional-context-sweep/smoke.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(2)
    value := c_calloc(memory, 1, 4)
    c_store_u32(value, 41)
    print(conditional_local(1, value))
    print(conditional_local(0, c_ptr_null()))
    print(conditional_assignment(1, 7, 13))
    print(conditional_assignment(0, 7, 13))
    print(conditional_compound(1))
    print(conditional_compound(0))
    print(c2lpp_bool_to_int(c_ptr_equal(conditional_pointer_local(1, value), value)))
    print(c_ptr_is_null(conditional_pointer_local(0, value)))
    print(character_class(95))
    print(character_class(120))
    print(escape_total())
    c_free(value)
    c_memory_destroy(memory)
EOF
(cd generated/conditional-context-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
CONDITIONAL_CONTEXT_RESULT=$(generated/conditional-context-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/conditional_contexts_reference.c -o generated/conditional-context-sweep/c-reference
    [ "$CONDITIONAL_CONTEXT_RESULT" = "$(generated/conditional-context-sweep/c-reference)" ]
fi
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/conditional-context-sweep/sanitize-probe - >/dev/null 2>&1; then
    (cd generated/conditional-context-sweep && rm -f smoke.o && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" smoke.lpp >/dev/null)
    cc -fsanitize=address,undefined -fno-omit-frame-pointer \
        generated/conditional-context-sweep/smoke.o "$REPO/lpp_runtime.c" \
        -o generated/conditional-context-sweep/smoke.asan -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        generated/conditional-context-sweep/smoke.asan >/dev/null
fi
echo 'PASS conditional locals/assignments and character literals -> native equivalence + sanitizers'

# sizeof is compile-time only: type and dereference operands are never evaluated.
# Primitive, pointer, aggregate and null-dereference forms use explicit ABI sizes.
rm -rf generated/sizeof-sweep
write_config sweep fixtures/sizeof_values.c '' sizeof_sweep sizeof_sweep generated/sizeof-sweep true false
"$EXE" >/dev/null
SIZEOF_REPORT=generated/sizeof-sweep/c2lpp.function-sweep-report.txt
SIZEOF_IR=generated/sizeof-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=5' "$SIZEOF_REPORT"
grep -Fq 'rejected_functions=0' "$SIZEOF_REPORT"
grep -Fq 'sizeof-type|Pair|16' "$SIZEOF_IR"
grep -Fq 'sizeof-dereference|pair|16' "$SIZEOF_IR"
cat > generated/sizeof-sweep/smoke.lpp <<'EOF'
import translated
import c_memory

def main():
    print(size_of_char())
    print(size_of_int())
    print(size_of_pointer())
    print(size_of_pair())
    print(size_of_dereference(c_ptr_null()))
EOF
(cd generated/sizeof-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
SIZEOF_RESULT=$(generated/sizeof-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/sizeof_values_reference.c -o generated/sizeof-sweep/c-reference
    [ "$SIZEOF_RESULT" = "$(generated/sizeof-sweep/c-reference)" ]
fi
echo 'PASS compile-time sizeof values -> explicit ABI/native equivalence/null-safe dereference'

# Comma-separated expression statements preserve left-to-right sequencing while
# retaining typed call checking and void-discard casts.
rm -rf generated/comma-statement-sweep
write_config sweep fixtures/comma_statements.c '' comma_statement_sweep comma_statement_sweep generated/comma-statement-sweep true false
"$EXE" >/dev/null
COMMA_REPORT=generated/comma-statement-sweep/c2lpp.function-sweep-report.txt
COMMA_IR=generated/comma-statement-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=4' "$COMMA_REPORT"
grep -Fq 'rejected_functions=0' "$COMMA_REPORT"
[ "$(grep -c 'stmt|expression|' "$COMMA_IR")" -ge 5 ]
cat > generated/comma-statement-sweep/smoke.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(2)
    left := c_calloc(memory, 1, 4)
    right := c_calloc(memory, 1, 4)
    print(comma_calls(7))
    print(discard_two(left, right))
    discard_three(3, 4, 5)
    print(3)
    c_free(left)
    c_free(right)
    c_memory_destroy(memory)
EOF
(cd generated/comma-statement-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
COMMA_RESULT=$(generated/comma-statement-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/comma_statements_reference.c -o generated/comma-statement-sweep/c-reference
    [ "$COMMA_RESULT" = "$(generated/comma-statement-sweep/c-reference)" ]
fi
echo 'PASS comma expression statements -> typed sequencing/native equivalence'

# Immutable file-scope integer arrays become checked pure-L++ accessors.
rm -rf generated/const-array-sweep
write_config sweep fixtures/const_arrays.c '' const_array_sweep const_array_sweep generated/const-array-sweep true false
"$EXE" >/dev/null
CONST_ARRAY_REPORT=generated/const-array-sweep/c2lpp.function-sweep-report.txt
CONST_ARRAY_IR=generated/const-array-sweep/c2lpp.normalized-ir.txt
grep -Fq 'const_arrays_seen=2' "$CONST_ARRAY_REPORT"
grep -Fq 'const_arrays_emitted=2' "$CONST_ARRAY_REPORT"
grep -Fq 'const_arrays_rejected=0' "$CONST_ARRAY_REPORT"
grep -Fq 'translated_functions=3' "$CONST_ARRAY_REPORT"
grep -Fq 'global-const-array|weights|count=8|explicit=8' "$CONST_ARRAY_IR"
grep -Fq 'global-const-array|inferred|count=4|explicit=4' "$CONST_ARRAY_IR"
grep -Fq 'const-array-load|weights' "$CONST_ARRAY_IR"
cat > generated/const-array-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(weight_at(0))
    print(weight_at(3))
    print(weight_at(4))
    print(inferred_at(2))
    print(weighted_sum(5, 3))
EOF
(cd generated/const-array-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
CONST_ARRAY_RESULT=$(generated/const-array-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/const_arrays_reference.c -o generated/const-array-sweep/c-reference
    [ "$CONST_ARRAY_RESULT" = "$(generated/const-array-sweep/c-reference)" ]
fi
cat > generated/const-array-sweep/oob.lpp <<'EOF'
import translated

def main():
    print(weight_at(8))
EOF
(cd generated/const-array-sweep && "$LPP" oob.lpp --linker direct >/dev/null)
if generated/const-array-sweep/oob > generated/const-array-sweep/oob.out 2>&1; then
    echo 'const-array out-of-bounds case unexpectedly succeeded' >&2
    exit 1
fi
grep -Fq 'C2-GLOBAL-CONST-ARRAY-INDEX:weights' generated/const-array-sweep/oob.out
echo 'PASS immutable global integer arrays -> checked pure-L++ accessors/native equivalence'

# General parser integration for scalar pointer places and pointer arithmetic.
rm -rf generated/pointer-place-sweep
write_config sweep fixtures/pointer_places.c '' pointer_place_sweep pointer_place_sweep generated/pointer-place-sweep true false
"$EXE" >/dev/null
POINTER_SWEEP_REPORT=generated/pointer-place-sweep/c2lpp.function-sweep-report.txt
POINTER_SWEEP_IR=generated/pointer-place-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=10' "$POINTER_SWEEP_REPORT"
grep -Fq 'rejected_functions=0' "$POINTER_SWEEP_REPORT"
grep -Fq 'place-load|dereference' "$POINTER_SWEEP_IR"
grep -Fq 'place-load|index' "$POINTER_SWEEP_IR"
grep -Fq 'place-assign|+=' "$POINTER_SWEEP_IR"
grep -Fq 'address-of|place-load|index' "$POINTER_SWEEP_IR"
cat > generated/pointer-place-sweep/smoke.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(4)
    values := c_calloc(memory, 4, 4)
    c_store_u32(c_ptr_add(values, 0), 7)
    c_store_u32(c_ptr_add(values, 1), 11)
    c_store_u32(c_ptr_add(values, 2), 13)
    c_store_u32(c_ptr_add(values, 3), 17)
    print(read_first(values))
    print(read_at(values, 2))
    print(sum_edges(values, 3))
    write_at(values, 1, 23)
    add_at(values, 2, 5)
    increment_first(values)
    print(c_load_i32(c_ptr_add(values, 0)))
    print(c_load_i32(c_ptr_add(values, 1)))
    print(c_load_i32(c_ptr_add(values, 2)))
    print(c_ptr_difference(address_at(values, 3), values))
    print(local_shift(values))
    print(pointer_distance(values, 1, 3))
    print(pointer_same(values, 2))
    c_free(values)
    c_memory_destroy(memory)
EOF
(cd generated/pointer-place-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
POINTER_SWEEP_RESULT=$(generated/pointer-place-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/pointer_places_reference.c -o generated/pointer-place-sweep/c-reference
    [ "$POINTER_SWEEP_RESULT" = "$(generated/pointer-place-sweep/c-reference)" ]
fi
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/pointer-place-sweep/sanitize-probe - >/dev/null 2>&1; then
    (cd generated/pointer-place-sweep && rm -f smoke.o && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" smoke.lpp >/dev/null)
    cc -fsanitize=address,undefined -fno-omit-frame-pointer \
        generated/pointer-place-sweep/smoke.o "$REPO/lpp_runtime.c" \
        -o generated/pointer-place-sweep/smoke.asan -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        generated/pointer-place-sweep/smoke.asan >/dev/null
fi
echo 'PASS parser-integrated scalar pointer places -> pure L++ native equivalence + sanitizers'

# Pointer-to-pointer loads/stores use ABI pointer slots. Array parameter syntax
# decays to a typed CPtr without guessing multidimensional array layout.
rm -rf generated/pointer-indirection-sweep
write_config sweep fixtures/pointer_indirection.c '' pointer_indirection_sweep pointer_indirection_sweep generated/pointer-indirection-sweep true false
"$EXE" >/dev/null
INDIRECTION_REPORT=generated/pointer-indirection-sweep/c2lpp.function-sweep-report.txt
INDIRECTION_IR=generated/pointer-indirection-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=6' "$INDIRECTION_REPORT"
grep -Fq 'rejected_functions=0' "$INDIRECTION_REPORT"
grep -Fq 'place-load|pointer-dereference|' "$INDIRECTION_IR"
grep -Fq 'place-load|pointer-index|' "$INDIRECTION_IR"
grep -Fq 'pointer-place-assign|' "$INDIRECTION_IR"
cat > generated/pointer-indirection-sweep/smoke.lpp <<'EOF'
import translated
import c_memory
import c_place

def make_int(memory: CMemory, value: Int) -> CPtr:
    pointer := c_calloc(memory, 1, 4)
    c_store_u32(pointer, value)
    return pointer

def main():
    memory := c_memory_new(8)
    first := make_int(memory, 11)
    second := make_int(memory, 23)
    third := make_int(memory, 37)
    values := c_calloc(memory, 2, 8)
    c_abi_pointer_place_store(c_abi_pointer_place_at(values, 0), first)
    c_abi_pointer_place_store(c_abi_pointer_place_at(values, 8), second)
    plain := c_calloc(memory, 4, 4)
    c_store_u32(c_ptr_add(plain, 0), 3)
    c_store_u32(c_ptr_add(plain, 1), 5)
    c_store_u32(c_ptr_add(plain, 2), 7)
    c_store_u32(c_ptr_add(plain, 3), 9)
    print(read_indirect(values))
    print(read_pointer_array(values, 1))
    write_pointer_array(values, 0, third)
    print(read_indirect(values))
    second_slot := c_ptr_add(c_ptr_cast(values, 8, 1), 1)
    write_indirect(second_slot, first)
    print(read_pointer_array(values, 1))
    print(read_array_parameter(plain, 2))
    print(read_fixed_parameter(plain, 3))
    c_free(plain)
    c_free(values)
    c_free(third)
    c_free(second)
    c_free(first)
    c_memory_destroy(memory)
EOF
(cd generated/pointer-indirection-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
INDIRECTION_RESULT=$(generated/pointer-indirection-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/pointer_indirection_reference.c -o generated/pointer-indirection-sweep/c-reference
    [ "$INDIRECTION_RESULT" = "$(generated/pointer-indirection-sweep/c-reference)" ]
fi
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/pointer-indirection-sweep/sanitize-probe - >/dev/null 2>&1; then
    (cd generated/pointer-indirection-sweep && rm -f smoke.o && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" smoke.lpp >/dev/null)
    cc -fsanitize=address,undefined -fno-omit-frame-pointer \
        generated/pointer-indirection-sweep/smoke.o "$REPO/lpp_runtime.c" \
        -o generated/pointer-indirection-sweep/smoke.asan -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        generated/pointer-indirection-sweep/smoke.asan >/dev/null
fi
echo 'PASS pointer-to-pointer and array-parameter places -> native equivalence + sanitizers'

# Postfix ++/-- on assignable places: `arr[i]++` and `(*p)++` return the old
# value and mutate the pointee, using the place post-increment/decrement helpers.
rm -rf generated/postfix-places
write_config sweep fixtures/postfix_places.c '' postfix_places_sweep postfix_places_sweep generated/postfix-places true false
"$EXE" >/dev/null
POSTFIX_PLACES_REPORT=generated/postfix-places/c2lpp.function-sweep-report.txt
grep -Fq 'translated_functions=2' "$POSTFIX_PLACES_REPORT"
grep -Fq 'rejected_functions=0' "$POSTFIX_PLACES_REPORT"
cat > generated/postfix-places/smoke.lpp <<'EOF'
import translated
import c_memory
def main():
    memory := c_memory_new(2)
    a := c_calloc(memory, 3, 4)
    c_store_u32(a, 10)
    c_store_u32(c_ptr_add(a,1), 20)
    c_store_u32(c_ptr_add(a,2), 30)
    r1 := f(a, 1)
    n1 := c_load_u32(c_ptr_add(a,1))
    r2 := g(a)
    n2 := c_load_u32(a)
    print(r1)
    print(n1)
    print(r2)
    print(n2)
    c_free(a)
    c_memory_destroy(memory)
EOF
(cd generated/postfix-places && "$LPP" smoke.lpp --linker direct >/dev/null)
POSTFIX_PLACES_RESULT=$(generated/postfix-places/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/postfix_places_reference.c -o generated/postfix-places/c-reference
    [ "$POSTFIX_PLACES_RESULT" = "$(generated/postfix-places/c-reference)" ]
fi
echo 'PASS postfix ++/-- on assignable places (arr[i]++ / (*p)++) -> native equivalence'

# An uninitialized pointer local (`int *p;`) now defaults to null instead of
# being rejected, so bodies that declare a pointer and assign it later translate.
rm -rf generated/ptrfix-sweep
write_config sweep fixtures/ptrfix.c '' ptrfix_sweep ptrfix_sweep generated/ptrfix-sweep true false
"$EXE" >/dev/null
PTRFIX_REPORT=generated/ptrfix-sweep/c2lpp.function-sweep-report.txt
PTRFIX_IR=generated/ptrfix-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=2' "$PTRFIX_REPORT"
grep -Fq 'rejected_functions=0' "$PTRFIX_REPORT"
grep -Fq 'stmt|local|p|pointer|' "$PTRFIX_IR"
grep -Fq 'stmt|assign|p|=' "$PTRFIX_IR"
cat > generated/ptrfix-sweep/smoke.lpp <<'EOF'
import translated
import c_memory
def main():
    memory := c_memory_new(4)
    arr := c_calloc(memory, 4, 4)
    c_store_u32(c_ptr_add(arr, 0), 10)
    c_store_u32(c_ptr_add(arr, 1), 20)
    c_store_u32(c_ptr_add(arr, 2), 30)
    c_store_u32(c_ptr_add(arr, 3), 40)
    print(f(0, arr))
    print(f(2, arr))
    print(g(1, arr))
    print(g(0, arr))
    c_free(arr)
    c_memory_destroy(memory)
EOF
(cd generated/ptrfix-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
PTRFIX_RESULT=$(generated/ptrfix-sweep/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/ptrfix_reference.c -o generated/ptrfix-sweep/c-reference
    [ "$PTRFIX_RESULT" = "$(generated/ptrfix-sweep/c-reference)" ]
fi
echo 'PASS uninitialized pointer locals -> null-default/native equivalence'

# A local declaration may contain several declarators: `int x = 1, y = 2;`.
rm -rf generated/multi-declarators
write_config sweep fixtures/multi_declarators.c '' multi_declarators_sweep multi_declarators_sweep generated/multi-declarators true false
"$EXE" >/dev/null
MULTI_DECL_REPORT=generated/multi-declarators/c2lpp.function-sweep-report.txt
grep -Fq 'translated_functions=5' "$MULTI_DECL_REPORT"
grep -Fq 'rejected_functions=0' "$MULTI_DECL_REPORT"
cat > generated/multi-declarators/smoke.lpp <<'EOF'
import translated
import c_memory
def main():
    memory := c_memory_new(2)
    a := c_calloc(memory, 4, 4)
    c_store_u32(a, 10)
    c_store_u32(c_ptr_add(a,1), 20)
    c_store_u32(c_ptr_add(a,2), 30)
    c_store_u32(c_ptr_add(a,3), 40)
    print(f1(0))
    print(f2(5))
    print(f3(5))
    print(f4(0))
    print(f5(a, 4))
    c_free(a)
    c_memory_destroy(memory)
EOF
(cd generated/multi-declarators && "$LPP" smoke.lpp --linker direct >/dev/null)
MULTI_DECL_RESULT=$(generated/multi-declarators/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/multi_declarators_reference.c -o generated/multi-declarators/c-reference
    [ "$MULTI_DECL_RESULT" = "$(generated/multi-declarators/c-reference)" ]
fi
echo 'PASS multi-declarator locals (int x = 1, y = 2;) -> native equivalence'

# Demand-bounded SysV aggregate layouts drive scalar arrow/member chains,
# nested by-value fields, bitfields and fixed-array places. Function pointers
# remain quarantined until typed target sets are available.
rm -rf generated/aggregate-place-sweep
write_config sweep fixtures/aggregate_places.c '' aggregate_place_sweep aggregate_place_sweep generated/aggregate-place-sweep true false
"$EXE" >/dev/null
AGGREGATE_SWEEP_REPORT=generated/aggregate-place-sweep/c2lpp.function-sweep-report.txt
AGGREGATE_SWEEP_IR=generated/aggregate-place-sweep/c2lpp.normalized-ir.txt
grep -Fq 'aggregate_types_selected=2' "$AGGREGATE_SWEEP_REPORT"
grep -Fq 'aggregates_complete=2' "$AGGREGATE_SWEEP_REPORT"
grep -Fq 'aggregate_fields_emitted=7' "$AGGREGATE_SWEEP_REPORT"
grep -Fq 'total_functions=12' "$AGGREGATE_SWEEP_REPORT"
grep -Fq 'translated_functions=12' "$AGGREGATE_SWEEP_REPORT"
grep -Fq 'rejected_functions=0' "$AGGREGATE_SWEEP_REPORT"
grep -Fq 'aggregate-layout|Inner|struct|8|4|1|' "$AGGREGATE_SWEEP_IR"
grep -Fq 'aggregate-layout|Node|struct|48|8|1|' "$AGGREGATE_SWEEP_IR"
grep -Fq 'place-load|aggregate-member|Node|id' "$AGGREGATE_SWEEP_IR"
grep -Fq 'place-load|aggregate-array|Node|values' "$AGGREGATE_SWEEP_IR"
cat > generated/aggregate-place-sweep/smoke.lpp <<'EOF'
import translated
import c_memory
import c_place

def main():
    memory := c_memory_new(4)
    node := c_calloc(memory, 1, 48)
    c_place_assign(c_place_from_parts(node, 0, 4, 1, -1, -1), 7)
    c_place_assign(c_place_from_parts(node, 16, 8, 1, -1, -1), 100)
    inner := c_ptr_subobject(node, 24, 8, 1)
    c_place_assign(c_place_from_parts(inner, 0, 4, 1, -1, -1), -4)
    c_place_assign(c_place_from_parts(inner, 4, 4, 0, 0, 3), 5)
    values := c_ptr_subobject(node, 32, 12, 4)
    c_place_assign(c_place_index(values, 0, 3, 4, 1), 11)
    c_place_assign(c_place_index(values, 1, 3, 4, 1), 13)
    c_place_assign(c_place_index(values, 2, 3, 4, 1), 17)
    print(8)
    print(48)
    print(inner_flags(inner))
    print(node_pointer_present(node))
    print(node_id(node))
    print(node_preincrement(node))
    print(node_total(node))
    print(node_nested(node))
    print(node_value(node, 2))
    node_set_id(node, 9)
    node_add_total(node, 23)
    node_set_delta(node, -8)
    node_set_flags(node, 3)
    node_set_value(node, 1, 29)
    print(c_place_load_int(c_place_from_parts(node, 0, 4, 1, -1, -1)))
    print(c_place_load_int(c_place_from_parts(node, 16, 8, 1, -1, -1)))
    print(c_place_load_int(c_place_from_parts(inner, 0, 4, 1, -1, -1)))
    print(c_place_load_int(c_place_from_parts(inner, 4, 4, 0, 0, 3)))
    print(c_place_load_int(c_place_index(values, 1, 3, 4, 1)))
    c_free(node)
    c_memory_destroy(memory)
EOF
(cd generated/aggregate-place-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
AGGREGATE_SWEEP_RESULT=$(generated/aggregate-place-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/aggregate_places_reference.c -o generated/aggregate-place-sweep/c-reference
    [ "$AGGREGATE_SWEEP_RESULT" = "$(generated/aggregate-place-sweep/c-reference)" ]
fi
cat > generated/aggregate-place-sweep/oob.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(2)
    node := c_calloc(memory, 1, 48)
    print(node_value(node, 3))
EOF
(cd generated/aggregate-place-sweep && "$LPP" oob.lpp --linker direct >/dev/null)
if generated/aggregate-place-sweep/oob > generated/aggregate-place-sweep/oob.out 2>&1; then
    echo 'aggregate array out-of-bounds case unexpectedly succeeded' >&2
    exit 1
fi
grep -Fq 'C2-PLACE-INDEX-OUT-OF-BOUNDS' generated/aggregate-place-sweep/oob.out
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/aggregate-place-sweep/sanitize-probe - >/dev/null 2>&1; then
    (cd generated/aggregate-place-sweep && rm -f smoke.o && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" smoke.lpp >/dev/null)
    cc -fsanitize=address,undefined -fno-omit-frame-pointer \
        generated/aggregate-place-sweep/smoke.o "$REPO/lpp_runtime.c" \
        -o generated/aggregate-place-sweep/smoke.asan -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        generated/aggregate-place-sweep/smoke.asan >/dev/null
fi
echo 'PASS parser-integrated aggregate scalar places -> native equivalence + bounds + sanitizers'

# Anonymous-struct typedefs (`typedef struct {...} Name;`) resolve to the typedef
# name, and nested aggregate pointer fields (`Box.p` of type `Pair *`) are
# transitively catalogued, so `if (b->p) return b->p->a;` translates.
rm -rf generated/nested-agg-sweep
write_config sweep fixtures/nested_aggregate_fields.c '' nested_agg_sweep nested_agg_sweep generated/nested-agg-sweep true false
"$EXE" >/dev/null
NESTED_AGG_REPORT=generated/nested-agg-sweep/c2lpp.function-sweep-report.txt
NESTED_AGG_IR=generated/nested-agg-sweep/c2lpp.normalized-ir.txt
grep -Fq 'translated_functions=2' "$NESTED_AGG_REPORT"
grep -Fq 'rejected_functions=0' "$NESTED_AGG_REPORT"
grep -Fq 'aggregate_types_selected=2' "$NESTED_AGG_REPORT"
grep -Fq 'aggregate-layout|Box|' "$NESTED_AGG_IR"
grep -Fq 'aggregate-layout|Pair|' "$NESTED_AGG_IR"
grep -Fq 'place-load|aggregate-pointer-member|Box|p' "$NESTED_AGG_IR"
grep -Fq 'place-load|aggregate-member|Box|n' "$NESTED_AGG_IR"
cat > generated/nested-agg-sweep/smoke.lpp <<'EOF'
import translated
import c_memory
def main():
    memory := c_memory_new(8)
    b := c_calloc(memory, 16, 1)
    pr := c_calloc(memory, 8, 1)
    c_store_u32(pr, 42)
    c_store_u32(c_ptr_add(pr, 4), 7)
    c_abi_pointer_place_store(c_abi_pointer_place_at(b, 0), pr)
    c_store_u32(c_ptr_add(b, 8), 5)
    print(f(b))
    print(f1(b))
    c_free(pr)
    c_free(b)
    c_memory_destroy(memory)
EOF
(cd generated/nested-agg-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
NESTED_AGG_RESULT=$(generated/nested-agg-sweep/smoke | paste -sd' ')
if command -v cc >/dev/null 2>&1; then
    cc tests/nested_aggregate_fields_reference.c -o generated/nested-agg-sweep/c-reference
    [ "$NESTED_AGG_RESULT" = "$(generated/nested-agg-sweep/c-reference)" ]
fi
echo 'PASS nested aggregate pointer-field chains (b->p->a) -> native equivalence'

# ABI-width pointer fields retain full CPtr provenance in a raw per-context side
# table. Direct load/store, null, chaining and pointer-field copies are executable;
# stale, untracked and invalidated representations trap deterministically.
rm -rf generated/aggregate-pointer-field-sweep
write_config sweep fixtures/aggregate_pointer_fields.c '' aggregate_pointer_field_sweep aggregate_pointer_field_sweep generated/aggregate-pointer-field-sweep true false
"$EXE" >/dev/null
ABI_POINTER_REPORT=generated/aggregate-pointer-field-sweep/c2lpp.function-sweep-report.txt
ABI_POINTER_IR=generated/aggregate-pointer-field-sweep/c2lpp.normalized-ir.txt
grep -Fq 'aggregates_complete=1' "$ABI_POINTER_REPORT"
grep -Fq 'aggregate_fields_emitted=2' "$ABI_POINTER_REPORT"
grep -Fq 'translated_functions=6' "$ABI_POINTER_REPORT"
grep -Fq 'rejected_functions=0' "$ABI_POINTER_REPORT"
grep -Fq 'place-load|aggregate-pointer-member|Link|next' "$ABI_POINTER_IR"
grep -Fq 'pointer-place-assign|place-load|aggregate-pointer-member|Link|next' "$ABI_POINTER_IR"
cat > generated/aggregate-pointer-field-sweep/smoke.lpp <<'EOF'
import translated
import c_memory
import c_place

def make_link(memory: CMemory, value: Int) -> CPtr:
    link := c_calloc(memory, 1, 16)
    c_place_assign(c_place_from_parts(link, 0, 4, 1, -1, -1), value)
    return link

def main():
    memory := c_memory_new(8)
    third := make_link(memory, 29)
    second := make_link(memory, 13)
    first := make_link(memory, 7)
    copy := make_link(memory, 5)
    link_set_next(second, third)
    link_set_next(first, second)
    print(link_has_next(first))
    print(link_next_value(first))
    print(c2lpp_bool_to_int(c_ptr_equal(link_get_next(first), second)))
    link_set_next(first, third)
    print(link_next_value(first))
    link_copy_next(copy, second)
    print(link_next_value(copy))
    link_clear_next(first)
    print(link_has_next(first))
    c_free(copy)
    c_free(first)
    c_free(second)
    c_free(third)
    c_memory_destroy(memory)
EOF
(cd generated/aggregate-pointer-field-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
ABI_POINTER_RESULT=$(generated/aggregate-pointer-field-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/aggregate_pointer_fields_reference.c -o generated/aggregate-pointer-field-sweep/c-reference
    [ "$ABI_POINTER_RESULT" = "$(generated/aggregate-pointer-field-sweep/c-reference)" ]
fi
cat > generated/aggregate-pointer-field-sweep/stale.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(4)
    owner := c_calloc(memory, 1, 16)
    target := c_calloc(memory, 1, 16)
    link_set_next(owner, target)
    c_free(target)
    print(c_ptr_is_null(link_get_next(owner)))
EOF
cat > generated/aggregate-pointer-field-sweep/untracked.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(4)
    owner := c_malloc(memory, 16)
    print(c_ptr_is_null(link_get_next(owner)))
EOF
cat > generated/aggregate-pointer-field-sweep/invalidated.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(4)
    owner := c_calloc(memory, 1, 16)
    target := c_calloc(memory, 1, 16)
    link_set_next(owner, target)
    slot := c_ptr_add(c_ptr_cast(owner, 8, 1), 1)
    c_store_i64(slot, 1)
    print(c_ptr_is_null(link_get_next(owner)))
EOF
cat > generated/aggregate-pointer-field-sweep/memmove.lpp <<'EOF'
import translated
import c_memory

def main():
    memory := c_memory_new(4)
    owners := c_calloc(memory, 2, 16)
    target := c_calloc(memory, 1, 16)
    link_set_next(owners, target)
    bytes := c_ptr_cast(owners, 1, 1)
    c_memmove(c_ptr_add(bytes, 16), bytes, 16)
EOF
for abi_pointer_case in stale untracked invalidated memmove; do
    (cd generated/aggregate-pointer-field-sweep && "$LPP" "$abi_pointer_case.lpp" --linker direct >/dev/null)
    if generated/aggregate-pointer-field-sweep/$abi_pointer_case > generated/aggregate-pointer-field-sweep/$abi_pointer_case.out 2>&1; then
        echo "ABI pointer-field safety case unexpectedly succeeded: $abi_pointer_case" >&2
        exit 1
    fi
done
grep -Fq 'C2-MEM-USE-AFTER-FREE' generated/aggregate-pointer-field-sweep/stale.out
grep -Fq 'C2-MEM-POINTER-FIELD-UNTRACKED' generated/aggregate-pointer-field-sweep/untracked.out
grep -Fq 'C2-MEM-POINTER-FIELD-INVALIDATED' generated/aggregate-pointer-field-sweep/invalidated.out
grep -Fq 'C2-MEM-POINTER-FIELD-MEMMOVE-UNSUPPORTED' generated/aggregate-pointer-field-sweep/memmove.out
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/aggregate-pointer-field-sweep/sanitize-probe - >/dev/null 2>&1; then
    (cd generated/aggregate-pointer-field-sweep && rm -f smoke.o && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" smoke.lpp >/dev/null)
    cc -fsanitize=address,undefined -fno-omit-frame-pointer \
        generated/aggregate-pointer-field-sweep/smoke.o "$REPO/lpp_runtime.c" \
        -o generated/aggregate-pointer-field-sweep/smoke.asan -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        generated/aggregate-pointer-field-sweep/smoke.asan >/dev/null
fi
echo 'PASS ABI-width aggregate pointer fields -> provenance side table/native equivalence/safety/sanitizers'

# Rejection is function-atomic: a pointer function is omitted, while valid
# functions before and after it remain typed and executable.
rm -rf generated/typed-unsupported
write_config translate-ir fixtures/typed_unsupported.c '' typed_unsupported typed_unsupported generated/typed-unsupported false false
"$EXE" >/dev/null
TYPED_UNSUPPORTED=generated/typed-unsupported/src/translated.lpp
TYPED_UNSUPPORTED_REPORT=generated/typed-unsupported/c2lpp.translation-report.txt
grep -Fq 'functions_total=4' "$TYPED_UNSUPPORTED_REPORT"
grep -Fq 'functions_translated=2' "$TYPED_UNSUPPORTED_REPORT"
grep -Fq 'functions_rejected=2' "$TYPED_UNSUPPORTED_REPORT"
grep -Fq 'C2-PLACE-DEREFERENCE-TYPE' "$TYPED_UNSUPPORTED_REPORT"
grep -Fq 'C2-TYPE-CALL-ARG' "$TYPED_UNSUPPORTED_REPORT"
grep -Fq 'def accepted_before(value: Int) -> Int:' "$TYPED_UNSUPPORTED"
grep -Fq 'def accepted_after(value: Int) -> Int:' "$TYPED_UNSUPPORTED"
! grep -Fq 'def rejected_pointer' "$TYPED_UNSUPPORTED"
! grep -Fq 'def rejected_bad_call' "$TYPED_UNSUPPORTED"
"$LPP" "$TYPED_UNSUPPORTED" --check >/dev/null
cat > generated/typed-unsupported/smoke.lpp <<'EOF'
import translated

def main():
    print(accepted_before(4))
    print(accepted_after(4))
EOF
(cd generated/typed-unsupported && "$LPP" smoke.lpp --linker direct >/dev/null)
[ "$(generated/typed-unsupported/smoke)" = '5
10' ]
echo 'PASS typed function-atomic unsupported quarantine/recovery'

# Integrated no-binding native profile: declarations/typedef graph, aggregate
# and bitfield layout, globals, checked allocation/pointer places, canonical
# loop, switch/fallthrough/goto CFG, ownership pairing and runtime emission.
rm -rf generated/native-profile
write_config native fixtures/native_profile.c '' native_profile native_profile generated/native-profile true false
"$EXE" >/dev/null
NATIVE=generated/native-profile/src/translated.lpp
NATIVE_IR=generated/native-profile/c2lpp.normalized-ir.txt
NATIVE_REPORT=generated/native-profile/c2lpp.translation-report.txt
grep -Fq 'accepted=1' "$NATIVE_REPORT"
grep -Fq 'pure_lpp=1' "$NATIVE_REPORT"
grep -Fq 'extern_blocks=0' "$NATIVE_REPORT"
grep -Fq 'native_links=0' "$NATIVE_REPORT"
grep -Fq 'unsupported=0' "$NATIVE_REPORT"
grep -Fq 'ownership_balanced=1' "$NATIVE_REPORT"
grep -Fq 'declaration_graph_complete_for_profile=1' "$NATIVE_REPORT"
grep -Fq 'global_init_graph_complete_for_profile=1' "$NATIVE_REPORT"
grep -Fq 'cfg_complete_for_profile=1' "$NATIVE_REPORT"
grep -Fq 'typedef|struct|Item|sysv-x86_64' "$NATIVE_IR"
grep -Fq 'place|index-member' "$NATIVE_IR"
grep -Fq 'place|address-index' "$NATIVE_IR"
grep -Fq 'place|dereference-member' "$NATIVE_IR"
grep -Fq 'place|arrow-member' "$NATIVE_IR"
grep -Fq 'cfg|for|switch|fallthrough|goto|cleanup' "$NATIVE_IR"
grep -Fq 'ownership|calloc|free|balanced|no-escape' "$NATIVE_IR"
grep -Fq 'callgraph|process->calloc' "$NATIVE_IR"
grep -Fq 'callgraph|process->free' "$NATIVE_IR"
grep -Fq 'global-dependency|base->seed_ptr' "$NATIVE_IR"
! grep -Fq 'extern "C"' "$NATIVE"
! grep -Fq 'link "' "$NATIVE"
! grep -Fq 'c2lpp typed unsupported' "$NATIVE"
! grep -Fq '\[native\]' generated/native-profile/lpp.toml
if find generated/native-profile -type f \( -name '*.c' -o -name '*.h' \) | grep -q .; then
    echo 'native generated package unexpectedly contains C source/header' >&2
    exit 1
fi
for runtime_module in c_memory c_layout c_globals c_cfg c_place; do
    [ -f "generated/native-profile/src/$runtime_module.lpp" ]
done
"$LPP" "$NATIVE" --check >/dev/null
cat > generated/native-profile/smoke.lpp <<'EOF'
import translated

def main():
    program := c_native_program_new()
    print(process(program, 4))
    print(process(program, 3))
    print(process(program, 1))
    c_native_program_destroy(program)
EOF
(cd generated/native-profile && "$LPP" smoke.lpp --linker direct >/dev/null)
NATIVE_RESULT=$(generated/native-profile/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/native_profile_reference.c -o generated/native-profile/c-reference
    NATIVE_C_RESULT=$(generated/native-profile/c-reference)
    [ "$NATIVE_RESULT" = "$NATIVE_C_RESULT" ]
fi
[ "$NATIVE_RESULT" = '76
50
52' ]
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/native-profile/sanitize-probe - >/dev/null 2>&1; then
    (cd generated/native-profile && rm -f smoke.o && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" smoke.lpp >/dev/null)
    cc -O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined \
        generated/native-profile/smoke.o "$REPO/lpp_runtime.c" \
        -o generated/native-profile/smoke.asan -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 \
    UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1 \
        generated/native-profile/smoke.asan >/dev/null
fi
echo 'PASS integrated no-binding native C profile -> 100% pure-L++ equivalence + sanitizers'

rm -rf generated/native-rejected
write_config native fixtures/native_profile_rejected.c '' native_rejected native_rejected generated/native-rejected true false
if "$EXE" > generated/native-rejected.out 2>&1; then
    echo 'unsupported native profile unexpectedly succeeded' >&2
    exit 1
fi
grep -Fq 'accepted=0' generated/native-rejected/c2lpp.translation-report.txt
grep -Fq 'C2-NATIVE-EXPECTED-typedef' generated/native-rejected/c2lpp.translation-report.txt
[ ! -f generated/native-rejected/translated.lpp ]
[ ! -f generated/native-rejected/bindings.lpp ]
rm -f generated/native-rejected.out
echo 'PASS native profile fail-closed with no binding fallback'

# General arbitrary-order translation-unit partition and declaration graph.
rm -rf generated/tu-graph
write_config tu-graph fixtures/frontend_profile.c '' frontend_tu frontend_tu generated/tu-graph true false
"$EXE" >/dev/null
TU_REPORT=generated/tu-graph/c2lpp.translation-unit-report.txt
TU_GRAPH=generated/tu-graph/c2lpp.translation-unit.txt
grep -Fq 'valid=1' "$TU_REPORT"
grep -Fq 'external_declarations=9' "$TU_REPORT"
grep -Fq 'typedef_declarations=3' "$TU_REPORT"
grep -Fq 'aggregate_declarations=1' "$TU_REPORT"
grep -Fq 'global_declarations=2' "$TU_REPORT"
grep -Fq 'prototype_declarations=2' "$TU_REPORT"
grep -Fq 'function_definitions=1' "$TU_REPORT"
grep -Fq 'function_pointer_declarations=1' "$TU_REPORT"
grep -Fq 'variadic_declarations=1' "$TU_REPORT"
grep -Fq 'unknown_declarations=0' "$TU_REPORT"
grep -Fq 'typedef|VisitFn|' "$TU_GRAPH"
grep -Fq 'prototype|visit|' "$TU_GRAPH"
grep -Fq 'prototype|log_values|' "$TU_GRAPH"
grep -Fq 'function|run_graph|' "$TU_GRAPH"
echo 'PASS general translation-unit partition/declaration graph'

rm -rf generated/decl-graph
write_config decl-graph fixtures/frontend_profile.c '' frontend_decl frontend_decl generated/decl-graph true false
"$EXE" >/dev/null
DECL_REPORT=generated/decl-graph/c2lpp.declaration-report.txt
DECL_GRAPH=generated/decl-graph/c2lpp.declaration-graph.txt
grep -Fq 'valid=1' "$DECL_REPORT"
grep -Fq 'declarations=9' "$DECL_REPORT"
grep -Fq 'base_types_resolved=9' "$DECL_REPORT"
grep -Fq 'base_types_unresolved=0' "$DECL_REPORT"
grep -Fq 'function_records=3' "$DECL_REPORT"
grep -Fq 'function_pointer_records=1' "$DECL_REPORT"
grep -Fq 'variadic_records=1' "$DECL_REPORT"
grep -Fq 'shape|typedef|VisitFn|base=int|class=primitive|' "$DECL_GRAPH"
grep -Fq 'shape|prototype|visit|base=int|class=primitive|' "$DECL_GRAPH"
grep -Fq 'shape|function|run_graph|base=int|class=primitive|' "$DECL_GRAPH"
echo 'PASS declaration base-type/declarator-shape graph'

rm -rf generated/body-graph
write_config body-graph fixtures/frontend_profile.c '' frontend_body frontend_body generated/body-graph true false
"$EXE" >/dev/null
BODY_REPORT=generated/body-graph/c2lpp.function-body-report.txt
BODY_GRAPH=generated/body-graph/c2lpp.function-body-graph.txt
grep -Fq 'valid=1' "$BODY_REPORT"
grep -Fq 'functions=1' "$BODY_REPORT"
grep -Fq 'bodies_partitioned=1' "$BODY_REPORT"
grep -Fq 'bodies_unbalanced=0' "$BODY_REPORT"
grep -Fq 'labels=1' "$BODY_REPORT"
grep -Fq 'case_labels=4' "$BODY_REPORT"
grep -Fq 'gotos=1' "$BODY_REPORT"
grep -Fq 'switches=1' "$BODY_REPORT"
grep -Fq 'for_loops=1' "$BODY_REPORT"
grep -Fq 'body|run_graph|' "$BODY_GRAPH"
echo 'PASS general function-body structural control graph'

rm -rf generated/call-graph
write_config call-graph fixtures/graph_analysis.c '' graph_calls graph_calls generated/call-graph true false
"$EXE" >/dev/null
CALL_REPORT=generated/call-graph/c2lpp.call-graph-report.txt
CALL_GRAPH=generated/call-graph/c2lpp.call-graph.txt
grep -Fq 'valid=1' "$CALL_REPORT"
grep -Fq 'functions=3' "$CALL_REPORT"
grep -Fq 'allocation_sites=1' "$CALL_REPORT"
grep -Fq 'reallocation_sites=1' "$CALL_REPORT"
grep -Fq 'deallocation_sites=2' "$CALL_REPORT"
grep -Fq 'call|graph_entry|graph_analyze|defined|' "$CALL_GRAPH"
grep -Fq 'call|graph_analyze|transform|indirect-or-unresolved|' "$CALL_GRAPH"
echo 'PASS direct/indirect call and allocation-site graph'

rm -rf generated/control-graph
write_config control-graph fixtures/graph_analysis.c '' graph_control graph_control generated/control-graph true false
"$EXE" >/dev/null
CONTROL_REPORT=generated/control-graph/c2lpp.control-graph-report.txt
CONTROL_GRAPH=generated/control-graph/c2lpp.control-graph.txt
grep -Fq 'valid=1' "$CONTROL_REPORT"
grep -Fq 'functions=3' "$CONTROL_REPORT"
grep -Fq 'functions_invalid=0' "$CONTROL_REPORT"
grep -Fq 'duplicate_labels=0' "$CONTROL_REPORT"
grep -Fq 'gotos=1' "$CONTROL_REPORT"
grep -Fq 'resolved_gotos=1' "$CONTROL_REPORT"
grep -Fq 'unresolved_gotos=0' "$CONTROL_REPORT"
grep -Fq 'edge|graph_analyze|goto|failed|' "$CONTROL_GRAPH"
echo 'PASS function-scoped labels/goto/control-target graph'

rm -rf generated/ownership-graph
write_config ownership-graph fixtures/graph_analysis.c '' graph_owner graph_owner generated/ownership-graph true false
"$EXE" >/dev/null
OWNER_REPORT=generated/ownership-graph/c2lpp.ownership-graph-report.txt
OWNER_GRAPH=generated/ownership-graph/c2lpp.ownership-graph.txt
grep -Fq 'valid=1' "$OWNER_REPORT"
grep -Fq 'functions=3' "$OWNER_REPORT"
grep -Fq 'functions_site_balanced=1' "$OWNER_REPORT"
grep -Fq 'allocation_sites=1' "$OWNER_REPORT"
grep -Fq 'reallocation_sites=1' "$OWNER_REPORT"
grep -Fq 'deallocation_sites=2' "$OWNER_REPORT"
grep -Fq 'ownership|graph_analyze|alloc=1|realloc=1|free=2|site-balanced-needs-path-proof' "$OWNER_GRAPH"
echo 'PASS allocation/free ownership-site graph'

# Path-sensitive ownership proof: walks each function body and proves balanced,
# escape, leak, double-free or (conservatively) unproven ownership.
rm -rf generated/ownership-proof
write_config ownership-proof fixtures/ownership_proof.c '' graph_ownerproof graph_ownerproof generated/ownership-proof true false
"$EXE" >/dev/null
OWNPROOF_REPORT=generated/ownership-proof/c2lpp.ownership-proof-report.txt
OWNPROOF_GRAPH=generated/ownership-proof/c2lpp.ownership-proof.txt
grep -Fq 'valid=1' "$OWNPROOF_REPORT"
grep -Fq 'functions=7' "$OWNPROOF_REPORT"
grep -Fq 'proved_balanced=2' "$OWNPROOF_REPORT"
grep -Fq 'proved_escape=1' "$OWNPROOF_REPORT"
grep -Fq 'proved_leak=1' "$OWNPROOF_REPORT"
grep -Fq 'proved_double_free=1' "$OWNPROOF_REPORT"
grep -Fq 'unproven=2' "$OWNPROOF_REPORT"
grep -Fq 'proof|balanced_path|proved-balanced|alloc=1|realloc=0|free=1' "$OWNPROOF_GRAPH"
grep -Fq 'proof|escape_path|proved-escape|alloc=1|realloc=0|free=0' "$OWNPROOF_GRAPH"
grep -Fq 'proof|leak_path|proved-leak|alloc=1|realloc=0|free=0' "$OWNPROOF_GRAPH"
grep -Fq 'proof|double_free_path|proved-double-free|alloc=1|realloc=0|free=2' "$OWNPROOF_GRAPH"
grep -Fq 'proof|realloc_balanced|proved-balanced|alloc=1|realloc=1|free=1' "$OWNPROOF_GRAPH"
grep -Fq 'proof|divergent_path|unproven' "$OWNPROOF_GRAPH"
grep -Fq 'proof|goto_path|unproven' "$OWNPROOF_GRAPH"
echo 'PASS path-sensitive ownership proof (balanced/escape/leak/double-free/unproven)'

rm -rf generated/graph-check
write_config graph-check fixtures/graph_analysis.c '' graph_check graph_check generated/graph-check true false
"$EXE" >/dev/null
GRAPH_CHECK_REPORT=generated/graph-check/c2lpp.graph-consistency-report.txt
grep -Fq 'valid=1' "$GRAPH_CHECK_REPORT"
grep -Fq 'error_count=0' "$GRAPH_CHECK_REPORT"
grep -Fq 'function_definitions=3' "$GRAPH_CHECK_REPORT"
grep -Fq 'allocation_sites=1' "$GRAPH_CHECK_REPORT"
grep -Fq 'reallocation_sites=1' "$GRAPH_CHECK_REPORT"
grep -Fq 'deallocation_sites=2' "$GRAPH_CHECK_REPORT"
grep -Fq 'semantic_translation_complete=0' "$GRAPH_CHECK_REPORT"
echo 'PASS cross-pass graph denominator/ownership consistency'

# Frontend profile v2 integrates forward typedefs, callback/variadic declaration
# graph, const string/int-array globals, nested aggregate arrays, macro
# provenance, pointer places and automatic loop/switch/goto lowering.
rm -rf generated/frontend-profile
write_config frontend fixtures/frontend_profile.c '' frontend_profile frontend_profile generated/frontend-profile true false
"$EXE" >/dev/null
FRONTEND=generated/frontend-profile/src/translated.lpp
FRONTEND_IR=generated/frontend-profile/c2lpp.normalized-ir.txt
FRONTEND_REPORT=generated/frontend-profile/c2lpp.translation-report.txt
grep -Fq 'engine=frontend-profile-v2' "$FRONTEND_REPORT"
grep -Fq 'accepted=1' "$FRONTEND_REPORT"
grep -Fq 'pure_lpp=1' "$FRONTEND_REPORT"
grep -Fq 'unsupported=0' "$FRONTEND_REPORT"
grep -Fq 'forward_typedefs=1' "$FRONTEND_REPORT"
grep -Fq 'callback_typedefs=1' "$FRONTEND_REPORT"
grep -Fq 'variadic_declarations=1' "$FRONTEND_REPORT"
grep -Fq 'const_string_globals=1' "$FRONTEND_REPORT"
grep -Fq 'const_array_globals=1' "$FRONTEND_REPORT"
grep -Fq 'nested_aggregates=1' "$FRONTEND_REPORT"
grep -Fq 'macro_provenance=1' "$FRONTEND_REPORT"
grep -Fq 'cfg_complete_for_profile=1' "$FRONTEND_REPORT"
grep -Fq 'typedef-forward|struct|Node' "$FRONTEND_IR"
grep -Fq 'typedef-callback|VisitFn|int|Node*,int' "$FRONTEND_IR"
grep -Fq 'prototype-variadic|log_values|const-char*,...|declaration-only' "$FRONTEND_IR"
grep -Fq 'global-const-string|banner|front' "$FRONTEND_IR"
grep -Fq 'global-const-array|weights|2,4,6' "$FRONTEND_IR"
grep -Fq 'provenance|macro-expand|SCALE->3' "$FRONTEND_IR"
grep -Fq 'cfg|auto-profile|for|switch|fallthrough|goto|cleanup' "$FRONTEND_IR"
! grep -Fq 'extern "C"' "$FRONTEND"
! grep -Fq 'link "' "$FRONTEND"
! grep -Fq '\[native\]' generated/frontend-profile/lpp.toml
"$LPP" "$FRONTEND" --check >/dev/null
cat > generated/frontend-profile/smoke.lpp <<'EOF'
import translated

def main():
    program := c_frontend_program_new()
    print(run_graph(program, 4))
    print(run_graph(program, 3))
    print(run_graph(program, 1))
    c_frontend_program_destroy(program)
EOF
(cd generated/frontend-profile && "$LPP" smoke.lpp --linker direct >/dev/null)
FRONTEND_RESULT=$(generated/frontend-profile/smoke)
[ "$FRONTEND_RESULT" = '197
163
143' ]
if command -v cc >/dev/null 2>&1; then
    cc tests/frontend_profile_reference.c -o generated/frontend-profile/c-reference
    FRONTEND_C_RESULT=$(generated/frontend-profile/c-reference)
    [ "$FRONTEND_RESULT" = "$FRONTEND_C_RESULT" ]
fi
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/frontend-profile/sanitize-probe - >/dev/null 2>&1; then
    (cd generated/frontend-profile && rm -f smoke.o && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" smoke.lpp >/dev/null)
    cc -O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined \
        generated/frontend-profile/smoke.o "$REPO/lpp_runtime.c" \
        -o generated/frontend-profile/smoke.asan -pthread -lm
    ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 \
    UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1 \
        generated/frontend-profile/smoke.asan >/dev/null
fi
echo 'PASS frontend profile v2 declarations/provenance/places/CFG -> pure L++ + sanitizers'

# Pure-L++ C pointer/allocation compatibility model. Compare successful memory
# operations with native C, then prove safety failures terminate explicitly.
rm -rf generated/c-memory
mkdir -p generated/c-memory
cp src/c_memory.lpp generated/c-memory/c_memory.lpp
cp tests/t_c_memory.lpp generated/c-memory/t_c_memory.lpp
(cd generated/c-memory && "$LPP" t_c_memory.lpp --linker direct >/dev/null)
MEMORY_RESULT=$(generated/c-memory/t_c_memory)
[ "$MEMORY_RESULT" = '24
48
6
6
0
38' ]
if command -v cc >/dev/null 2>&1; then
    cc tests/c_memory_reference.c -o generated/c-memory/c-reference
    MEMORY_C_RESULT=$(generated/c-memory/c-reference)
    [ "$MEMORY_RESULT" = "$MEMORY_C_RESULT" ]
fi

cat > generated/c-memory/t_oob.lpp <<'EOF'
import c_memory
def main():
    m := c_memory_new(4)
    p := c_ptr_cast(c_malloc(m, 4), 4, 1)
    print(c_load_i32(c_ptr_add(p, 1)))
EOF
cat > generated/c-memory/t_uaf.lpp <<'EOF'
import c_memory
def main():
    m := c_memory_new(4)
    p := c_malloc(m, 4)
    c_free(p)
    print(c_load_u8(p))
EOF
cat > generated/c-memory/t_double_free.lpp <<'EOF'
import c_memory
def main():
    m := c_memory_new(4)
    p := c_malloc(m, 4)
    c_free(p)
    c_free(p)
EOF
cat > generated/c-memory/t_interior_free.lpp <<'EOF'
import c_memory
def main():
    m := c_memory_new(4)
    p := c_malloc(m, 4)
    c_free(c_ptr_add(p, 1))
EOF
cat > generated/c-memory/t_readonly.lpp <<'EOF'
import c_memory
def main():
    m := c_memory_new(4)
    p := c_ptr_cast(c_malloc(m, 4), 1, 0)
    c_store_u8(p, 1)
EOF
for case_name in oob uaf double_free interior_free readonly; do
    (cd generated/c-memory && "$LPP" "t_$case_name.lpp" --linker direct >/dev/null)
    if generated/c-memory/t_$case_name > generated/c-memory/$case_name.out 2>&1; then
        echo "C memory safety case unexpectedly succeeded: $case_name" >&2
        exit 1
    fi
done
grep -Fq 'C2-MEM-OUT-OF-BOUNDS' generated/c-memory/oob.out
grep -Fq 'C2-MEM-USE-AFTER-FREE' generated/c-memory/uaf.out
grep -Fq 'C2-MEM-DOUBLE-FREE' generated/c-memory/double_free.out
grep -Fq 'C2-MEM-INTERIOR-FREE' generated/c-memory/interior_free.out
grep -Fq 'C2-MEM-WRITE-READONLY' generated/c-memory/readonly.out
echo 'PASS pure-L++ C pointer/allocation model + native equivalence/safety traps'

# Typed place/lvalue foundation over CPtr and target layout metadata.
cp src/c_layout.lpp generated/c-memory/c_layout.lpp
cp src/c_place.lpp generated/c-memory/c_place.lpp
cp tests/t_c_place.lpp generated/c-memory/t_c_place.lpp
(cd generated/c-memory && "$LPP" t_c_place.lpp --linker direct >/dev/null)
PLACE_RESULT=$(generated/c-memory/t_c_place)
if command -v cc >/dev/null 2>&1; then
    cc tests/c_place_reference.c -o generated/c-memory/place-reference
    PLACE_C_RESULT=$(generated/c-memory/place-reference)
    [ "$PLACE_RESULT" = "$PLACE_C_RESULT" ]
fi
PLACE_LINES=$(printf '%s\n' "$PLACE_RESULT" | wc -l | tr -d ' ')
[ "$PLACE_LINES" -eq 24 ]

cat > generated/c-memory/t_place_oob.lpp <<'EOF'
import c_memory
import c_layout
import c_place
def main():
    memory := c_memory_new(4)
    pointer := c_calloc(memory, 2, 4)
    place := c_place_index(pointer, 2, 2, 4, 1)
    print(c_place_load_int(place))
EOF
cat > generated/c-memory/t_place_readonly.lpp <<'EOF'
import c_memory
import c_layout
import c_place
def main():
    memory := c_memory_new(4)
    pointer := c_ptr_cast(c_malloc(memory, 4), 4, 0)
    place := c_place_dereference(pointer, 4, 1)
    c_place_assign(place, 7)
EOF
cat > generated/c-memory/t_place_divzero.lpp <<'EOF'
import c_memory
import c_layout
import c_place
def main():
    memory := c_memory_new(4)
    pointer := c_calloc(memory, 1, 4)
    place := c_place_dereference(pointer, 4, 1)
    c_place_div_assign(place, 0)
EOF
cat > generated/c-memory/t_place_copy_size.lpp <<'EOF'
import c_memory
import c_layout
import c_place
def main():
    memory := c_memory_new(4)
    pointer := c_calloc(memory, 1, 8)
    small := c_place_at(pointer, 0, 4, 1)
    large := c_place_at(pointer, 0, 8, 1)
    c_place_copy(small, large)
EOF
for place_case in oob readonly divzero copy_size; do
    (cd generated/c-memory && "$LPP" "t_place_$place_case.lpp" --linker direct >/dev/null)
    if generated/c-memory/t_place_$place_case > generated/c-memory/place_$place_case.out 2>&1; then
        echo "C place safety case unexpectedly succeeded: $place_case" >&2
        exit 1
    fi
done
grep -Fq 'C2-PLACE-INDEX-OUT-OF-BOUNDS' generated/c-memory/place_oob.out
grep -Fq 'C2-MEM-WRITE-READONLY' generated/c-memory/place_readonly.out
grep -Fq 'C2-PLACE-DIVIDE-BY-ZERO' generated/c-memory/place_divzero.out
grep -Fq 'C2-PLACE-COPY-SIZE' generated/c-memory/place_copy_size.out
echo 'PASS typed C place foundation (24 differential checks + 4 safety traps)'

cp tests/t_c_pointer_place.lpp generated/c-memory/t_c_pointer_place.lpp
(cd generated/c-memory && "$LPP" t_c_pointer_place.lpp --linker direct >/dev/null)
POINTER_PLACE_RESULT=$(generated/c-memory/t_c_pointer_place)
if command -v cc >/dev/null 2>&1; then
    cc tests/c_pointer_place_reference.c -o generated/c-memory/pointer-place-reference
    POINTER_PLACE_C_RESULT=$(generated/c-memory/pointer-place-reference)
    [ "$POINTER_PLACE_RESULT" = "$POINTER_PLACE_C_RESULT" ]
fi
[ "$(printf '%s\n' "$POINTER_PLACE_RESULT" | wc -l | tr -d ' ')" -eq 13 ]

cat > generated/c-memory/t_pointer_slot_oob.lpp <<'EOF'
import c_memory
import c_layout
import c_place
def main():
    memory := c_memory_new(4)
    slots := c_calloc(memory, 2, 64)
    place := c_pointer_place_index(slots, 2, 2)
    print(c_pointer_place_load(place).offset)
EOF
cat > generated/c-memory/t_pointer_slot_readonly.lpp <<'EOF'
import c_memory
import c_layout
import c_place
def main():
    memory := c_memory_new(4)
    raw := c_calloc(memory, 1, 64)
    readonly := c_ptr_cast(raw, 64, 0)
    place := c_pointer_place_index(readonly, 0, 1)
    c_pointer_place_assign_null(place)
EOF
cat > generated/c-memory/t_pointer_slot_stale.lpp <<'EOF'
import c_memory
import c_layout
import c_place
def main():
    memory := c_memory_new(4)
    slots := c_calloc(memory, 1, 64)
    target := c_malloc(memory, 4)
    place := c_pointer_place_index(slots, 0, 1)
    c_pointer_place_store(place, target)
    c_free(target)
    stale := c_pointer_place_load(place)
    print(stale.offset)
EOF
for pointer_slot_case in oob readonly stale; do
    (cd generated/c-memory && "$LPP" "t_pointer_slot_$pointer_slot_case.lpp" --linker direct >/dev/null)
    if generated/c-memory/t_pointer_slot_$pointer_slot_case > generated/c-memory/pointer_slot_$pointer_slot_case.out 2>&1; then
        echo "C pointer-place safety case unexpectedly succeeded: $pointer_slot_case" >&2
        exit 1
    fi
done
grep -Fq 'C2-POINTER-PLACE-INDEX-OUT-OF-BOUNDS' generated/c-memory/pointer_slot_oob.out
grep -Fq 'C2-MEM-WRITE-READONLY' generated/c-memory/pointer_slot_readonly.out
grep -Fq 'C2-MEM-USE-AFTER-FREE' generated/c-memory/pointer_slot_stale.out
echo 'PASS pointer-valued C places (13 differential checks + 3 safety traps)'

# Target-explicit struct/union and bitfield layout over checked C memory.
cp src/c_layout.lpp generated/c-memory/c_layout.lpp
cp tests/t_c_layout.lpp generated/c-memory/t_c_layout.lpp
(cd generated/c-memory && "$LPP" t_c_layout.lpp --linker direct >/dev/null)
LAYOUT_RESULT=$(generated/c-memory/t_c_layout)
if command -v cc >/dev/null 2>&1; then
    cc -std=c11 tests/c_layout_reference.c -o generated/c-memory/layout-reference
    LAYOUT_C_RESULT=$(generated/c-memory/layout-reference)
    [ "$LAYOUT_RESULT" = "$LAYOUT_C_RESULT" ]
fi
[ "$LAYOUT_RESULT" = '12
4
5
17
-9
1
77
8
8' ]
echo 'PASS pure-L++ SysV struct/union/bitfield layout + native equivalence'

# Explicit zero-initialized global/static region and ordered initialization.
cp src/c_globals.lpp generated/c-memory/c_globals.lpp
cp tests/t_c_globals.lpp generated/c-memory/t_c_globals.lpp
(cd generated/c-memory && "$LPP" t_c_globals.lpp --linker direct >/dev/null)
GLOBALS_RESULT=$(generated/c-memory/t_c_globals)
[ "$GLOBALS_RESULT" = '0
7
21
26
24
2' ]
if command -v cc >/dev/null 2>&1; then
    cc tests/c_globals_reference.c -o generated/c-memory/globals-reference
    GLOBALS_C_RESULT=$(generated/c-memory/globals-reference)
    [ "$GLOBALS_RESULT" = "$GLOBALS_C_RESULT" ]
fi
echo 'PASS pure-L++ C global/static storage/init model + native equivalence'

# Explicit CFG block machine for switch, fallthrough, goto, branch and return.
cp src/c_cfg.lpp generated/c-memory/c_cfg.lpp
cp tests/t_c_cfg.lpp generated/c-memory/t_c_cfg.lpp
(cd generated/c-memory && "$LPP" t_c_cfg.lpp --linker direct >/dev/null)
CFG_RESULT=$(generated/c-memory/t_c_cfg)
[ "$CFG_RESULT" = '10
123
3
-1' ]
if command -v cc >/dev/null 2>&1; then
    cc tests/c_cfg_reference.c -o generated/c-memory/cfg-reference
    CFG_C_RESULT=$(generated/c-memory/cfg-reference)
    [ "$CFG_RESULT" = "$CFG_C_RESULT" ]
fi
echo 'PASS pure-L++ C CFG state-machine model + goto/switch native equivalence'

# Sanitizer gate for all successful compatibility foundations. Do not suppress
# LeakSanitizer: the layout representation was specifically redesigned after
# this gate exposed managed metadata leaks.
if command -v cc >/dev/null 2>&1 && printf 'int main(void){return 0;}\n' | cc -x c -fsanitize=address,undefined -o generated/c-memory/sanitize-probe - >/dev/null 2>&1; then
    for sanitizer_case in t_c_memory t_c_place t_c_pointer_place t_c_layout t_c_globals t_c_cfg; do
        (cd generated/c-memory && rm -f "$sanitizer_case.o" && LPP_AOT=1 LPP_AOT_ONLY=1 "$LPP" "$sanitizer_case.lpp" >/dev/null)
        cc -O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined \
            "generated/c-memory/$sanitizer_case.o" "$REPO/lpp_runtime.c" \
            -o "generated/c-memory/$sanitizer_case.asan" -pthread -lm
        ASAN_OPTIONS=detect_leaks=1:halt_on_error=1 \
        UBSAN_OPTIONS=halt_on_error=1:print_stacktrace=1 \
            "generated/c-memory/$sanitizer_case.asan" >/dev/null
    done
    echo 'PASS C memory/layout/globals/CFG ASan+UBSan leak/error gate'
else
    echo 'NOTE ASan/UBSan compiler gate unavailable on this host' >&2
fi

rm -rf generated/unsupported
write_config translate fixtures/unsupported_pointer.c '' unsupported_sample unsupported_sample generated/unsupported true false
if "$EXE" >/dev/null; then
    echo 'strict legacy translation unexpectedly succeeded' >&2
    exit 1
fi
UNSUPPORTED=$(sed -n 's/^unsupported_constructs=//p' generated/unsupported/c2lpp.translation-report.txt)
[ "$UNSUPPORTED" -ge 2 ]
grep -Fq 'c2lpp phase2 unsupported' generated/unsupported/src/translated.lpp
"$LPP" generated/unsupported/src/translated.lpp --check >/dev/null
echo "PASS Phase 2 unsupported-code quarantine/report ($UNSUPPORTED markers)"

# Provenance-preserving multi-file audit.  This is a byte-conservation and
# dependency-graph gate, not a claim that the blocked constructs are translated.
rm -rf generated/audit-multifile
write_config audit '' fixtures/multifile/project.c2lpp audit_multifile audit_multifile generated/audit-multifile true false
"$EXE" >/dev/null
AUDIT=generated/audit-multifile/c2lpp.audit-report.txt
AUDIT_DEPS=generated/audit-multifile/c2lpp.project-dependencies.txt
grep -Fq 'byte_partition_ok=1' "$AUDIT"
grep -Fq 'lexical_zero_unclassified=1' "$AUDIT"
grep -Fq 'project.source_files=2' "$AUDIT"
grep -Fq 'project.header_files=1' "$AUDIT"
grep -Fq 'dependencies.local=2' "$AUDIT"
grep -Fq 'dependencies.external=1' "$AUDIT"
grep -Fq 'dependencies.unresolved=0' "$AUDIT"
grep -Fq 'reason.C2-CFG-GOTO=1' "$AUDIT"
grep -Fq 'reason.C2-CFG-SWITCH=1' "$AUDIT"
grep -Fq 'reason.C2-TYPE-UNION=1' "$AUDIT"
grep -Fq 'whole_translation_complete=1' "$AUDIT"
grep -Fq 'math.c|common.h|quote|local|' "$AUDIT_DEPS"
grep -Fq 'math.c|stdint.h|angle|external|' "$AUDIT_DEPS"
echo 'PASS multi-file C audit/provenance/dependency closure'

# Opt-in pinned whole-amalgamation scale gate.  The generated/downloaded files
# stay under work/ and never enter a source patch.
if [ "$SQLITE_AUDIT" = 1 ]; then
    SQLITE_SOURCE=$(sh scripts/fetch-sqlite-3460100.sh)
    rm -rf work/sqlite-3460100/audit-out
    write_config audit "$SQLITE_SOURCE" '' sqlite3460100_audit sqlite3460100_audit work/sqlite-3460100/audit-out true false 3.46.1 6c35bc5f7f85eac9c49928bacbb02bb694b547aabf69197e058cca245ad80e83
    "$EXE" >/dev/null
    SQLITE_AUDIT=work/sqlite-3460100/audit-out/c2lpp.audit-report.txt
    SQLITE_BYTES=$(wc -c < "$SQLITE_SOURCE" | tr -d ' ')
    grep -Fq "bytes=$SQLITE_BYTES" "$SQLITE_AUDIT"
    grep -Fq 'source_version=3.46.1' "$SQLITE_AUDIT"
    grep -Fq 'source_sha256=6c35bc5f7f85eac9c49928bacbb02bb694b547aabf69197e058cca245ad80e83' "$SQLITE_AUDIT"
    grep -Fq 'byte_partition_ok=1' "$SQLITE_AUDIT"
    grep -Fq 'lexical_zero_unclassified=1' "$SQLITE_AUDIT"
    grep -Fq 'reason.C2-CFG-GOTO=' "$SQLITE_AUDIT"
    grep -Fq 'reason.C2-PP-MACRO-INCLUDE=1' "$SQLITE_AUDIT"
    grep -Fq 'whole_translation_complete=1' "$SQLITE_AUDIT"
    echo "PASS pinned SQLite 3.46.1 whole-amalgamation audit ($SQLITE_BYTES bytes)"

    if command -v cc >/dev/null 2>&1 && command -v awk >/dev/null 2>&1; then
        SQLITE_ALL=work/sqlite-3460100/sqlite3.preprocessed-all.i
        SQLITE_FILTERED=work/sqlite-3460100/sqlite3.preprocessed-filtered.i
        cc -E "$SQLITE_SOURCE" > "$SQLITE_ALL"
        awk -v src="$SQLITE_SOURCE" '/^# [0-9]+ "/ { active=index($0,"\"" src "\"")>0; next } active { print }' "$SQLITE_ALL" > "$SQLITE_FILTERED"
        rm -rf work/sqlite-3460100/tu-out
        write_config tu-graph "$SQLITE_FILTERED" '' sqlite3460100_tu sqlite3460100_tu work/sqlite-3460100/tu-out true false 3.46.1-preprocessed ''
        "$EXE" >/dev/null
        TU_SQLITE_REPORT=work/sqlite-3460100/tu-out/c2lpp.translation-unit-report.txt
        TU_SQLITE_GRAPH=work/sqlite-3460100/tu-out/c2lpp.translation-unit.txt
        grep -Fq 'valid=1' "$TU_SQLITE_REPORT"
        grep -Fq 'unknown_declarations=0' "$TU_SQLITE_REPORT"
        TU_FUNCTIONS=$(sed -n 's/^function_definitions=//p' "$TU_SQLITE_REPORT")
        TU_DECLS=$(sed -n 's/^external_declarations=//p' "$TU_SQLITE_REPORT")
        [ "$TU_FUNCTIONS" -ge 2000 ]
        [ "$TU_DECLS" -ge 4000 ]
        grep -Fq 'global|sqlite3_version|' "$TU_SQLITE_GRAPH"
        grep -Fq 'prototype|sqlite3_libversion|' "$TU_SQLITE_GRAPH"
        grep -Fq 'function|sqlite3_sourceid|' "$TU_SQLITE_GRAPH"
        echo "PASS SQLite active translation-unit graph ($TU_DECLS declarations, $TU_FUNCTIONS function bodies, zero unknown)"

        rm -rf work/sqlite-3460100/decl-out
        write_config decl-graph "$SQLITE_FILTERED" '' sqlite3460100_decl sqlite3460100_decl work/sqlite-3460100/decl-out true false 3.46.1-preprocessed ''
        "$EXE" >/dev/null
        SQLITE_DECL_REPORT=work/sqlite-3460100/decl-out/c2lpp.declaration-report.txt
        grep -Fq "declarations=$TU_DECLS" "$SQLITE_DECL_REPORT"
        grep -Fq "base_types_resolved=$TU_DECLS" "$SQLITE_DECL_REPORT"
        grep -Fq 'base_types_unresolved=0' "$SQLITE_DECL_REPORT"
        grep -Fq 'function_pointer_records=' "$SQLITE_DECL_REPORT"
        grep -Fq 'variadic_records=' "$SQLITE_DECL_REPORT"
        echo "PASS SQLite declaration base-type shapes ($TU_DECLS/$TU_DECLS resolved)"

        rm -rf work/sqlite-3460100/body-out
        write_config body-graph "$SQLITE_FILTERED" '' sqlite3460100_body sqlite3460100_body work/sqlite-3460100/body-out true false 3.46.1-preprocessed ''
        "$EXE" >/dev/null
        SQLITE_BODY_REPORT=work/sqlite-3460100/body-out/c2lpp.function-body-report.txt
        grep -Fq "functions=$TU_FUNCTIONS" "$SQLITE_BODY_REPORT"
        grep -Fq "bodies_partitioned=$TU_FUNCTIONS" "$SQLITE_BODY_REPORT"
        grep -Fq 'bodies_unbalanced=0' "$SQLITE_BODY_REPORT"
        SQLITE_STATEMENTS=$(sed -n 's/^statements=//p' "$SQLITE_BODY_REPORT")
        SQLITE_GOTOS=$(sed -n 's/^gotos=//p' "$SQLITE_BODY_REPORT")
        [ "$SQLITE_STATEMENTS" -ge 40000 ]
        [ "$SQLITE_GOTOS" -ge 700 ]
        echo "PASS SQLite structural body graphs ($TU_FUNCTIONS/$TU_FUNCTIONS balanced, $SQLITE_STATEMENTS statements)"

        rm -rf work/sqlite-3460100/call-out
        write_config call-graph "$SQLITE_FILTERED" '' sqlite3460100_calls sqlite3460100_calls work/sqlite-3460100/call-out true false 3.46.1-preprocessed ''
        "$EXE" >/dev/null
        SQLITE_CALL_REPORT=work/sqlite-3460100/call-out/c2lpp.call-graph-report.txt
        grep -Fq "functions=$TU_FUNCTIONS" "$SQLITE_CALL_REPORT"
        SQLITE_CALLS=$(sed -n 's/^call_sites=//p' "$SQLITE_CALL_REPORT")
        [ "$SQLITE_CALLS" -ge 10000 ]
        grep -Fq 'allocation_sites=' "$SQLITE_CALL_REPORT"
        grep -Fq 'deallocation_sites=' "$SQLITE_CALL_REPORT"
        echo "PASS SQLite direct/indirect call graph ($SQLITE_CALLS call sites)"

        rm -rf work/sqlite-3460100/control-out
        write_config control-graph "$SQLITE_FILTERED" '' sqlite3460100_control sqlite3460100_control work/sqlite-3460100/control-out true false 3.46.1-preprocessed ''
        "$EXE" >/dev/null
        SQLITE_CONTROL_REPORT=work/sqlite-3460100/control-out/c2lpp.control-graph-report.txt
        grep -Fq 'valid=1' "$SQLITE_CONTROL_REPORT"
        grep -Fq "functions_valid=$TU_FUNCTIONS" "$SQLITE_CONTROL_REPORT"
        grep -Fq 'functions_invalid=0' "$SQLITE_CONTROL_REPORT"
        grep -Fq 'duplicate_labels=0' "$SQLITE_CONTROL_REPORT"
        grep -Fq 'unresolved_gotos=0' "$SQLITE_CONTROL_REPORT"
        SQLITE_RESOLVED_GOTOS=$(sed -n 's/^resolved_gotos=//p' "$SQLITE_CONTROL_REPORT")
        [ "$SQLITE_RESOLVED_GOTOS" -ge 700 ]
        echo "PASS SQLite function-scoped control targets ($SQLITE_RESOLVED_GOTOS gotos resolved)"

        rm -rf work/sqlite-3460100/owner-out
        write_config ownership-graph "$SQLITE_FILTERED" '' sqlite3460100_owner sqlite3460100_owner work/sqlite-3460100/owner-out true false 3.46.1-preprocessed ''
        "$EXE" >/dev/null
        SQLITE_OWNER_REPORT=work/sqlite-3460100/owner-out/c2lpp.ownership-graph-report.txt
        grep -Fq "functions=$TU_FUNCTIONS" "$SQLITE_OWNER_REPORT"
        grep -Fq 'allocation_sites=' "$SQLITE_OWNER_REPORT"
        grep -Fq 'reallocation_sites=' "$SQLITE_OWNER_REPORT"
        grep -Fq 'deallocation_sites=' "$SQLITE_OWNER_REPORT"
        SQLITE_OWNER_PATHS=$(sed -n 's/^functions_requiring_path_analysis=//p' "$SQLITE_OWNER_REPORT")
        [ "$SQLITE_OWNER_PATHS" -ge 500 ]
        echo "PASS SQLite ownership-site graph ($SQLITE_OWNER_PATHS functions require path proof)"

        rm -rf work/sqlite-3460100/graph-check-out
        write_config graph-check "$SQLITE_FILTERED" '' sqlite3460100_graphcheck sqlite3460100_graphcheck work/sqlite-3460100/graph-check-out true false 3.46.1-preprocessed ''
        "$EXE" >/dev/null
        SQLITE_GRAPH_CHECK=work/sqlite-3460100/graph-check-out/c2lpp.graph-consistency-report.txt
        grep -Fq 'valid=1' "$SQLITE_GRAPH_CHECK"
        grep -Fq 'error_count=0' "$SQLITE_GRAPH_CHECK"
        grep -Fq 'semantic_translation_complete=0' "$SQLITE_GRAPH_CHECK"
        echo 'PASS SQLite cross-pass graph consistency'

        rm -rf work/sqlite-3460100/sweep-out
        write_config sweep "$SQLITE_FILTERED" '' sqlite3460100_sweep sqlite3460100_sweep work/sqlite-3460100/sweep-out true false 3.46.1-preprocessed ''
        "$EXE" >/dev/null
        SQLITE_SWEEP_REPORT=work/sqlite-3460100/sweep-out/c2lpp.function-sweep-report.txt
        SQLITE_SWEEP_LPP=work/sqlite-3460100/sweep-out/translated.lpp
        grep -Fq "total_functions=$TU_FUNCTIONS" "$SQLITE_SWEEP_REPORT"
        SQLITE_TRANSLATED=$(sed -n 's/^translated_functions=//p' "$SQLITE_SWEEP_REPORT")
        [ "$SQLITE_TRANSLATED" -ge 66 ]
        grep -Fq 'scalar_typedef_aliases=232' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'const_arrays_emitted=3' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregate_type_budget=10' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregate_types_selected=10' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregates_complete=6' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregates_partial=3' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregate_fields_emitted=100' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'function|storeLastErrno|void' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|sqlite3WalFile|pointer' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|sqlite3WalLimit|void' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'stmt|if-single|' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|sqlite3_backup_remaining|int' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|sqlite3_str_errcode|int' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|sqlite3HeaderSizePcache|int' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|sqlite3WalHeapMemory|int' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|jsonIs4HexB|int' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'sizeof-type|PgHdr|80' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'character|117' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|sqlite3ExprWalkNoop|int' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'stmt|ternary-return|condition|' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'place-load|aggregate-member|PgHdr|nRef' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'place-load|aggregate-pointer-member|Wal|pWalFd' work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        ! grep -Fq 'extern "C"' "$SQLITE_SWEEP_LPP"
        ! grep -Fq 'link "' "$SQLITE_SWEEP_LPP"
        SQLITE_SWEEP_CHECK=$("$LPP" "$SQLITE_SWEEP_LPP" --check 2>&1)
        printf '%s\n' "$SQLITE_SWEEP_CHECK" | grep -Fq 'L++ check: OK'
        echo "PASS SQLite automatic semantic sweep ($SQLITE_TRANSLATED/$TU_FUNCTIONS pure-L++ functions type-check)"

        rm -rf work/sqlite-3460100/frontend-attempt
        write_config frontend "$SQLITE_FILTERED" '' sqlite3460100_frontend sqlite3460100_frontend work/sqlite-3460100/frontend-attempt true false 3.46.1-preprocessed ''
        if "$EXE" >/dev/null 2>&1; then
            echo 'whole SQLite frontend unexpectedly reported success' >&2
            exit 1
        fi
        grep -Fq 'accepted=0' work/sqlite-3460100/frontend-attempt/c2lpp.translation-report.txt
        [ ! -f work/sqlite-3460100/frontend-attempt/translated.lpp ]
        [ ! -f work/sqlite-3460100/frontend-attempt/bindings.lpp ]
        echo 'PASS whole SQLite frontend remains fail-closed (no fake L++ or binding fallback)'
    fi
fi

# Optional real-system end-to-end gates. They prove that generated packages
# compile, link, and call the installed native libraries rather than only
# matching fixture text.
if [ -f /usr/include/sqlite3.h ]; then
    rm -rf generated/sqlite3-system
    write_config bindings /usr/include/sqlite3.h '' sqlite3_system sqlite3 generated/sqlite3-system false true
    "$EXE" >/dev/null
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
    write_config bindings /usr/include/zlib.h '' zlib_system z generated/zlib-system false true
    "$EXE" >/dev/null
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

rm -f c2lpp.json
echo 'c2lpp tests: PASS'
