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
grep -Fq 'Cannot mutate captured variable' "$ROOT/tests/run_aot_parity.sh"
# SAFETY-CONTRACT CHANGE: the two ARC-cycle rejection contracts were replaced
# by positive ones. Cycles are broken statically rather than refused, so the
# guard now checks that the breaker, its acyclicity proof and the programs that
# exercise it are all still present.
grep -Fq 'cycle_broken_node.lpp' "$ROOT/tests/aot_parity.tsv"
grep -Fq 'cycle_broken_list.lpp' "$ROOT/tests/aot_parity.tsv"
grep -Fq 'owning subgraph' "$ROOT/src/analysis/cyclebreak.rs"
grep -Fq 'owning_subgraph_is_acyclic_property' "$ROOT/src/analysis/cyclebreak.rs"
echo 'PASS L++ safety mission claim and regression gate'
