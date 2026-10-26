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


. "$ROOT/tests/run_parts/config_and_typed_smoke.sh"
. "$ROOT/tests/run_parts/typed_control_and_places.sh"
. "$ROOT/tests/run_parts/frontend_graph_gates.sh"
. "$ROOT/tests/run_parts/sqlite_scale_and_system.sh"

rm -f c2lpp.json
echo 'c2lpp tests: PASS'
