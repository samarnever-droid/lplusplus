# typed C control flow, places, pointer, and aggregate gates. Sourced by tests/run.sh in the same POSIX shell.
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

