# optional pinned SQLite scale audit and host-library gates. Sourced by tests/run.sh in the same POSIX shell.
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
        [ "$SQLITE_TRANSLATED" -ge 74 ]
        grep -Fq 'scalar_typedef_aliases=232' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'const_arrays_emitted=3' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregate_type_budget=20' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregate_types_selected=18' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregates_complete=15' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregates_partial=2' "$SQLITE_SWEEP_REPORT"
        grep -Fq 'aggregate_fields_emitted=159' "$SQLITE_SWEEP_REPORT"
        SQLITE_SWEEP_IR=work/sqlite-3460100/sweep-out/c2lpp.normalized-ir.txt
        grep -Fq 'function|storeLastErrno|void' "$SQLITE_SWEEP_IR"
        grep -Fq 'def countLookasideSlots(c2lpp_arg_p: CPtr)' "$SQLITE_SWEEP_LPP"
        grep -Fq 'mut p := c2lpp_arg_p' "$SQLITE_SWEEP_LPP"
        grep -Fq 'stmt|if-block|' "$SQLITE_SWEEP_IR"
        grep -Fq 'stmt|else-block' "$SQLITE_SWEEP_IR"
        grep -Fq 'function|sqlite3_str_errcode|int' "$SQLITE_SWEEP_IR"
        grep -Fq 'function|sqlite3HeaderSizePcache|int' "$SQLITE_SWEEP_IR"
        grep -Fq 'function|sqlite3HeaderSizePcache1|int' "$SQLITE_SWEEP_IR"
        grep -Fq 'function|jsonIs4HexB|int' "$SQLITE_SWEEP_IR"
        grep -Fq 'sizeof-type|PgHdr|80' "$SQLITE_SWEEP_IR"
        grep -Fq 'sizeof-type|PgHdr1|56' "$SQLITE_SWEEP_IR"
        grep -Fq 'character|117' "$SQLITE_SWEEP_IR"
        grep -Fq 'function|sqlite3ExprWalkNoop|int' "$SQLITE_SWEEP_IR"
        grep -Fq 'stmt|ternary-return|condition|' "$SQLITE_SWEEP_IR"
        grep -Fq 'place-load|aggregate-member|PgHdr|nRef' "$SQLITE_SWEEP_IR"
        grep -Fq 'place-load|aggregate-pointer-member|PgHdr|pData' "$SQLITE_SWEEP_IR"
        ! grep -Fq 'extern "C"' "$SQLITE_SWEEP_LPP"
        ! grep -Fq 'link "' "$SQLITE_SWEEP_LPP"
        SQLITE_SWEEP_CHECK=$("$LPP" "$SQLITE_SWEEP_LPP" --check 2>&1)
        printf '%s\n' "$SQLITE_SWEEP_CHECK" | grep -Fq '[rewrite] check OK'

        # The extra size-query helper is executed as pure L++ and compared with
        # the same function compiled from the pinned native amalgamation.
        cat > work/sqlite-3460100/sweep-out/header-size-smoke.lpp <<'EOF'
import translated

def main():
    print(sqlite3HeaderSizePcache1())
EOF
        (cd work/sqlite-3460100/sweep-out && "$LPP" header-size-smoke.lpp --linker direct >/dev/null)
        SQLITE_HEADER_SIZE_LPP=$(work/sqlite-3460100/sweep-out/header-size-smoke)
        cat > work/sqlite-3460100/header-size-reference.c <<EOF
#include <stdio.h>
#include "$SQLITE_SOURCE"
int main(void){
  printf("%d\n", sqlite3HeaderSizePcache1());
  return 0;
}
EOF
        cc -D_GNU_SOURCE -w work/sqlite-3460100/header-size-reference.c -o work/sqlite-3460100/header-size-reference
        SQLITE_HEADER_SIZE_C=$(work/sqlite-3460100/header-size-reference)
        [ "$SQLITE_HEADER_SIZE_LPP" = "$SQLITE_HEADER_SIZE_C" ]
        [ "$SQLITE_HEADER_SIZE_LPP" = '56' ]
        echo 'PASS SQLite PgHdr1 size helper pure-L++/native-C equivalence (56)'

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

