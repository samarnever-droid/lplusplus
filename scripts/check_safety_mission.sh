#!/usr/bin/env sh
# Keep public safety language tied to checked-in evidence, not marketing claims.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
MISSION="$ROOT/documentation/Safety_Mission.md"
test -f "$MISSION"
grep -Fq 'S4 — Rust-equivalent claim' "$MISSION"
grep -Fq 'Not yet claimed' "$MISSION"
grep -Fiq 'strong cycles are rejected' "$MISSION"
# A premature absolute claim is forbidden outside the mission's discussion of
# the future threshold. Keep the search deliberately narrow and case-insensitive.
if grep -RIni --exclude='Safety_Mission.md' --exclude='check_safety_mission.sh' \
  -E 'safe as rust|as safe as rust|rust-equivalent safety' \
  "$ROOT/README.md" "$ROOT/Doc.md" "$ROOT/wiki" "$ROOT/documentation" 2>/dev/null | grep -viE "do not claim|not a blanket|not claim" ; then
  echo 'Unsafe documentation claim detected: use the verified-subset wording.' >&2
  exit 1
fi
# Existing negative AOT contracts are required safety regressions.
grep -Fq 'aot_reject_arc_cycle' "$ROOT/tests/run_aot_parity.sh"
grep -Fq 'aot_reject_list_arc_cycle' "$ROOT/tests/run_aot_parity.sh"
grep -Fq 'not supported safely yet' "$ROOT/tests/run_aot_parity.sh"
echo 'PASS L++ safety mission claim and regression gate'
