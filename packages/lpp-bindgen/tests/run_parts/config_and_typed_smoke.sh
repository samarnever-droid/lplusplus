# configuration, backends, translation smoke, and early typed-parser gates. Sourced by tests/run.sh in the same POSIX shell.
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
grep -Fq 'vendored_modules=30' "$BACKEND_REPORT"
! grep -R -Fq 'extern "C"' generated/sqlite-backend --include='*.lpp'
! grep -R -Fq 'link "' generated/sqlite-backend --include='*.lpp'
if find generated/sqlite-backend -type f \( -name '*.c' -o -name '*.h' \) | grep -q .; then
    echo 'curated backend unexpectedly contains C source/header' >&2
    exit 1
fi
BACKEND_CHECK=$("$LPP" "$BACKEND_LPP" --check 2>&1)
printf '%s\n' "$BACKEND_CHECK" | grep -Fq '[rewrite] check OK'
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
printf '%s\n' "$POINTER_CHECK" | grep -Fq '[rewrite] check OK'
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
printf '%s\n' "$CAST_CHECK" | grep -Fq '[rewrite] check OK'
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
printf '%s\n' "$EXPR_CHECK" | grep -Fq '[rewrite] check OK'
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
printf '%s\n' "$IF_BLOCK_CHECK" | grep -Fq '[rewrite] check OK'
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
printf '%s\n' "$WHILE_CHECK" | grep -Fq '[rewrite] check OK'
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

# C parameters are by-value locals. Writes nested under a loop/branch therefore
# need a mutable L++ local copy even though nested statement records are not
# flattened into the top-level normalized summary.
rm -rf generated/mutated-parameter-sweep
write_config sweep tests/mutated_parameter.c '' mutated_parameter_sweep mutated_parameter_sweep generated/mutated-parameter-sweep true false
"$EXE" >/dev/null
MUTATED_PARAM_REPORT=generated/mutated-parameter-sweep/c2lpp.function-sweep-report.txt
MUTATED_PARAM_LPP=generated/mutated-parameter-sweep/translated.lpp
grep -Fq 'total_functions=2' "$MUTATED_PARAM_REPORT"
grep -Fq 'translated_functions=2' "$MUTATED_PARAM_REPORT"
grep -Fq 'rejected_functions=0' "$MUTATED_PARAM_REPORT"
grep -Fq 'def count_down(c2lpp_arg_n: Int)' "$MUTATED_PARAM_LPP"
grep -Fq 'def clamp_to_zero(c2lpp_arg_n: Int)' "$MUTATED_PARAM_LPP"
MUTATED_PARAM_CHECK=$("$LPP" "$MUTATED_PARAM_LPP" --check 2>&1)
printf '%s\n' "$MUTATED_PARAM_CHECK" | grep -Fq '[rewrite] check OK'
cat > generated/mutated-parameter-sweep/smoke.lpp <<'EOF'
import translated

def main():
    print(count_down(5))
    print(count_down(-2))
    print(clamp_to_zero(-7))
    print(clamp_to_zero(9))
EOF
(cd generated/mutated-parameter-sweep && "$LPP" smoke.lpp --linker direct >/dev/null)
MUTATED_PARAM_RESULT=$(generated/mutated-parameter-sweep/smoke)
if command -v cc >/dev/null 2>&1; then
    cc tests/mutated_parameter.c tests/mutated_parameter_reference.c -o generated/mutated-parameter-sweep/c-reference
    [ "$MUTATED_PARAM_RESULT" = "$(generated/mutated-parameter-sweep/c-reference)" ]
fi
echo 'PASS nested C parameter mutation -> mutable L++ local/native equivalence'

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
printf '%s\n' "$FOR_CHECK" | grep -Fq '[rewrite] check OK'
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
printf '%s\n' "$LOOP_CONTROL_CHECK" | grep -Fq '[rewrite] check OK'
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

