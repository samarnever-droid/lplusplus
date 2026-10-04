#!/usr/bin/env bash
# corpus_correctness.sh — the v1-INDEPENDENT correctness oracle for the rewrite.
#
# v1/LegacyEngine is retired, so "parity with v1" is no longer the measure.
# This runs every corpus program through the REWRITE engine and judges it on its
# own terms: does it compile, link, run, exit 0, and (when the program carries
# internal assertions) report success rather than failure?
#
# Usage: scripts/corpus_correctness.sh [run|check]   (default: run)
#   run   — compile + link + execute each program (full correctness)
#   check — type-check only (fast triage of compile-level gaps)
#
# Output: a summary line, a per-category breakdown, an error-code histogram for
# compile failures, and the full per-file log in /tmp/corpus_correctness.log.
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LPP="$ROOT/target/debug/lpp"
MODE="${1:-run}"
LOG=/tmp/corpus_correctness.log
PER_FILE_TIMEOUT=25

if [ ! -x "$LPP" ]; then
  echo "lpp binary not found at $LPP — build it: cargo build --bin lpp && cargo build -p lpp-runtime" >&2
  exit 2
fi

: > "$LOG"
total=0; pass=0; compfail=0; runfail=0; assertfail=0; timeout_n=0
xfail_ok=0; xfail_bad=0; skip_n=0
declare -A errcodes

# Corpus = every .lpp under tests/ and examples/ (sorted for determinism).
mapfile -t FILES < <(find "$ROOT/tests" "$ROOT/examples" -name '*.lpp' 2>/dev/null | sort)

# Classify a file's EXPECTED outcome. The corpus mixes three kinds of program
# and scoring each as "must exit 0" is wrong for two of them:
#   run    — a normal program: must compile, link, run, exit 0, no assert fail.
#   reject — a negative test: the compiler MUST reject it (emit an Exxxx code).
#            A clean rejection is the CORRECT result; compiling it is the bug.
#   skip   — not a standalone runnable program in this headless harness:
#            a library module (no `main`), or a GUI/display program that needs
#            a window server. Excluded from the denominator.
# A file may override the heuristic with a directive comment anywhere in it:
#   `# corpus-expect: run|reject|skip`
classify() {
  local f="$1" rel="$2" dir
  dir=$(grep -oiE 'corpus-expect:[[:space:]]*(run|reject|skip)' "$f" 2>/dev/null \
        | head -1 | sed -E 's/.*:[[:space:]]*//' | tr 'A-Z' 'a-z')
  if [ -n "$dir" ]; then echo "$dir"; return; fi
  # WASM-target rejection tests assert restrictions of the `wasm` target
  # (no file/simd/spawn/ffi/net/env/command). Running them under the native
  # target is meaningless — several of those features ARE allowed natively —
  # so they belong to the wasm gate, not this native corpus.
  case "$rel" in
    *wasm/reject/*) echo "skip"; return;;
  esac
  case "$rel" in
    *reject*|*_rejected*|*_bad_*) echo "reject"; return;;
  esac
  # A program with no `main` is a library module meant to be imported.
  if ! grep -qE '\bdef[[:space:]]+main\b' "$f" 2>/dev/null; then echo "skip"; return; fi
  # GUI/display programs (import the `gui` module, or use gui/sdl/webview
  # builtins) need a window server and cannot run headless.
  if grep -qE '^\s*import\s+gui\b|\b(gui_|webview_|sdl)' "$f" 2>/dev/null; then
    echo "skip"; return
  fi
  echo "run"
}

for f in "${FILES[@]}"; do
  total=$((total + 1))
  rel="${f#$ROOT/}"
  expect=$(classify "$f" "$rel")

  if [ "$expect" = "skip" ]; then
    skip_n=$((skip_n + 1)); echo "SKIP     $rel" >> "$LOG"; continue
  fi

  if [ "$MODE" = "check" ]; then
    out=$(LPP_ENGINE=rewrite timeout "$PER_FILE_TIMEOUT" "$LPP" "$f" --check 2>&1); rc=$?
  else
    # </dev/null guarantees stdin is at EOF: a program calling input() reads the
    # empty string and terminates instead of blocking until the per-file timeout.
    out=$(LPP_ENGINE=rewrite timeout "$PER_FILE_TIMEOUT" "$LPP" run "$f" </dev/null 2>&1); rc=$?
    # lpp run emits an executable named after the source into the cwd; remove it so the
    # harness never pollutes the repo root with one binary per corpus file.
    rm -f "./$(basename "$f" .lpp)"
  fi

  code=$(printf '%s' "$out" | grep -oE 'E[0-9]{4}' | head -1)

  # Negative tests: a rejection (nonzero exit with a diagnostic) is CORRECT.
  if [ "$expect" = "reject" ]; then
    if [ "$rc" -ne 0 ] && [ "$rc" -ne 124 ]; then
      xfail_ok=$((xfail_ok + 1)); echo "XFAIL-OK $rel  [${code:-rejected}]" >> "$LOG"
    else
      xfail_bad=$((xfail_bad + 1))
      echo "WRONGACCEPT $rel  (should have been rejected, rc=$rc)" >> "$LOG"
    fi
    continue
  fi

  # From here on the file is expected to RUN successfully.
  if [ "$rc" -eq 124 ]; then
    timeout_n=$((timeout_n + 1)); echo "TIMEOUT  $rel" >> "$LOG"; continue
  fi

  if [ "$rc" -ne 0 ]; then
    if [ -n "$code" ]; then
      compfail=$((compfail + 1)); errcodes[$code]=$(( ${errcodes[$code]:-0} + 1 ))
      echo "COMPFAIL $rel  [$code]  $(printf '%s' "$out" | grep -m1 -E "$code" | cut -c1-100)" >> "$LOG"
    else
      runfail=$((runfail + 1))
      echo "RUNFAIL  $rel  rc=$rc  $(printf '%s' "$out" | tail -1 | cut -c1-100)" >> "$LOG"
    fi
    continue
  fi

  # rc == 0: ran. If the program carries internal assertions, honour its verdict.
  if printf '%s' "$out" | grep -qiE '\bFAIL(ED|URE)?\b|ASSERTION FAILED|[0-9]+/[0-9]+ .*fail'; then
    assertfail=$((assertfail + 1)); echo "ASSERTFAIL $rel  $(printf '%s' "$out" | grep -iE 'fail' | head -1 | cut -c1-100)" >> "$LOG"
  else
    pass=$((pass + 1)); echo "PASS     $rel" >> "$LOG"
  fi
done

echo "==================== CORPUS CORRECTNESS ($MODE) ===================="
# "correct" = normal programs that pass + negative tests correctly rejected.
# The denominator excludes skipped (non-runnable) programs.
correct=$((pass + xfail_ok))
scored=$((total - skip_n))
echo "total=$total  scored=$scored  skipped=$skip_n"
echo "  run:    PASS=$pass  COMPFAIL=$compfail  RUNFAIL=$runfail  ASSERTFAIL=$assertfail  TIMEOUT=$timeout_n"
echo "  reject: XFAIL-OK=$xfail_ok  WRONGACCEPT=$xfail_bad"
if [ "$scored" -gt 0 ]; then
  pct=$(awk "BEGIN{printf \"%.1f\", ($correct/$scored)*100}")
  echo "correct: ${correct}/${scored} = ${pct}%   (pass ${pass} + correctly-rejected ${xfail_ok})"
fi
if [ "$compfail" -gt 0 ]; then
  echo "--- compile-error codes (count) ---"
  for c in "${!errcodes[@]}"; do echo "$c ${errcodes[$c]}"; done | sort -k2 -nr
fi
echo "full log: $LOG"

