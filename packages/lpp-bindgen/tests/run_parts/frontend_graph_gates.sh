# native profile, declaration graphs, ownership, and frontend gates. Sourced by tests/run.sh in the same POSIX shell.
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
grep -Fq 'functions=8' "$OWNPROOF_REPORT"
grep -Fq 'proved_balanced=3' "$OWNPROOF_REPORT"
grep -Fq 'proved_escape=1' "$OWNPROOF_REPORT"
grep -Fq 'proved_leak=1' "$OWNPROOF_REPORT"
grep -Fq 'proved_double_free=1' "$OWNPROOF_REPORT"
grep -Fq 'unproven=2' "$OWNPROOF_REPORT"
grep -Fq 'proof|balanced_path|proved-balanced|alloc=1|realloc=0|free=1' "$OWNPROOF_GRAPH"
grep -Fq 'proof|escape_path|proved-escape|alloc=1|realloc=0|free=0' "$OWNPROOF_GRAPH"
grep -Fq 'proof|leak_path|proved-leak|alloc=1|realloc=0|free=0' "$OWNPROOF_GRAPH"
grep -Fq 'proof|double_free_path|proved-double-free|alloc=1|realloc=0|free=2' "$OWNPROOF_GRAPH"
grep -Fq 'proof|realloc_balanced|proved-balanced|alloc=1|realloc=1|free=1' "$OWNPROOF_GRAPH"
grep -Fq 'proof|do_balanced|proved-balanced|alloc=1|realloc=0|free=1' "$OWNPROOF_GRAPH"
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
cp tests/t_context_destroyed.lpp generated/c-memory/t_context_destroyed.lpp
for case_name in oob uaf double_free interior_free readonly context_destroyed; do
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
grep -Fq 'C2-MEM-CONTEXT-DESTROYED' generated/c-memory/context_destroyed.out
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

