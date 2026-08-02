#!/usr/bin/env sh
# Runner for the Android/Termux target-triple tests.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP=${LPP:-$ROOT/target/release/lpp}
[ -x "$LPP" ] || LPP="$ROOT/target/debug/lpp"
[ -x "$LPP" ] || { echo "lpp compiler not found; set LPP" >&2; exit 2; }
export LPP
sh "$ROOT/tests/test_target_android.sh"
