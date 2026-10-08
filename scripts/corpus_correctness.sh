#!/usr/bin/env bash
# corpus_correctness.sh — the v1-INDEPENDENT correctness oracle for the rewrite.
#
# This gate does not invoke v1/LegacyEngine, so textual parity with v1 is not
# the measure. It runs every corpus program through the REWRITE engine and
# judges it on its own terms: does it compile, link, run, exit 0, and (when the
# program carries internal assertions) report success rather than failure?
#
# Usage: scripts/corpus_correctness.sh [run|check]   (default: run)
#   run   — compile + link + execute each program (full correctness)
#   check — type-check only (fast triage of compile-level gaps)
#
# Output: a summary line, a per-category breakdown, an error-code histogram for
# compile failures, and the full per-file log in /tmp/corpus_correctness.log.
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2
LPP="./target/debug/lpp"
if [ -x "./target/debug/lpp.exe" ] || [ -f "./target/debug/lpp.exe" ]; then
  LPP="./target/debug/lpp.exe"
fi
MODE="${1:-run}"
LOG=/tmp/corpus_correctness.log
PER_FILE_TIMEOUT=25

case "$MODE" in
  run|check) ;;
  *) echo "usage: $0 [run|check]" >&2; exit 2;;
esac

if [ ! -x "$LPP" ]; then
  echo "lpp binary not found at $LPP — build it: cargo build --bin lpp && cargo build -p lpp-runtime" >&2
  exit 2
fi

: > "$LOG"
ERRCODES_LOG=/tmp/corpus_errcodes.log
: > "$ERRCODES_LOG"
total=0; pass=0; compfail=0; runfail=0; assertfail=0; timeout_n=0
xfail_ok=0; xfail_bad=0; skip_n=0

# Corpus = every .lpp under tests/ and examples/ (sorted for determinism).
FILES=()
if type mapfile >/dev/null 2>&1; then
  mapfile -t FILES < <(find "$ROOT/tests" "$ROOT/examples" -name '*.lpp' 2>/dev/null | sort)
else
  while IFS= read -r f; do
    [ -n "$f" ] && FILES+=("$f")
  done < <(find "$ROOT/tests" "$ROOT/examples" -name '*.lpp' 2>/dev/null | sort)
fi

# Classify a file's EXPECTED outcome. The corpus mixes three kinds of program
# and scoring each as "must exit 0" is wrong for two of them:
#   run    — a normal program: must compile, link, run, exit 0, no assert fail.
#   reject — a negative test: the compiler MUST reject it with a compile
#            diagnostic. A clean rejection is CORRECT; accepting it, timing
#            out, or failing later in linking/execution is a gate failure.
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
    out=$(LPP_ENGINE=rewrite timeout "$PER_FILE_TIMEOUT" "$LPP" "$rel" --check 2>&1); rc=$?
  else
    # </dev/null guarantees stdin is at EOF: a program calling input() reads the
    # empty string and terminates instead of blocking until the per-file timeout.
    out=$(LPP_ENGINE=rewrite timeout "$PER_FILE_TIMEOUT" "$LPP" run "$rel" </dev/null 2>&1); rc=$?
    # lpp run emits an executable named after the source into the cwd; remove it so the
    # harness never pollutes the repo root with one binary per corpus file.
    rm -f "./$(basename "$rel" .lpp)" "./$(basename "$rel" .lpp).exe"
  fi

  code=$(printf '%s' "$out" | grep -oE 'E[0-9]{4}' | head -1)

  # Negative tests: only a compiler rejection is CORRECT. A linker crash,
  # timeout, or unrelated nonzero process status must not masquerade as an
  # expected failure merely because the source belongs to the reject corpus.
  if [ "$expect" = "reject" ]; then
    if [ "$rc" -ne 0 ] && [ "$rc" -ne 124 ] \
       && { [ -n "$code" ] || printf '%s' "$out" | grep -qiE '\[rewrite\][[:space:]]+compile error|compile error:'; }; then
      xfail_ok=$((xfail_ok + 1)); echo "XFAIL-OK $rel  [${code:-compile-error}]" >> "$LOG"
    else
      xfail_bad=$((xfail_bad + 1))
      echo "WRONGREJECT $rel  (expected compiler rejection, rc=$rc, code=${code:-none})" >> "$LOG"
      printf '%s\n' "$out" | sed 's/^/           | /' >> "$LOG"
    fi
    continue
  fi

  # From here on the file is expected to RUN successfully.
  if [ "$rc" -eq 124 ]; then
    timeout_n=$((timeout_n + 1)); echo "TIMEOUT  $rel" >> "$LOG"; continue
  fi

  if [ "$rc" -ne 0 ]; then
    # In check mode every nonzero result is a compile/check failure, even when
    # an internal verifier message has not yet been assigned an Exxxx code.
    # In run mode the explicit rewrite prefix separates compilation failures
    # from linker/runtime failures.
    if [ "$MODE" = "check" ] \
       || [ -n "$code" ] \
       || printf '%s' "$out" | grep -qiE '\[rewrite\][[:space:]]+compile error|compile error:'; then
      compfail=$((compfail + 1))
      bucket="${code:-NO_CODE}"
      echo "$bucket" >> "$ERRCODES_LOG"
      echo "COMPFAIL $rel  [$bucket]" >> "$LOG"
    else
      runfail=$((runfail + 1))
      echo "RUNFAIL  $rel  rc=$rc" >> "$LOG"
    fi
    printf '%s\n' "$out" | sed 's/^/           | /' >> "$LOG"
    continue
  fi

  # rc == 0: ran. If the program carries internal assertions, honour its verdict.
  if printf '%s' "$out" | grep -qiE '\bFAIL(ED|URE)?\b|ASSERTION FAILED|[0-9]+/[0-9]+ .*fail'; then
    assertfail=$((assertfail + 1)); echo "ASSERTFAIL $rel" >> "$LOG"
    printf '%s\n' "$out" | sed 's/^/           | /' >> "$LOG"
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
echo "  reject: XFAIL-OK=$xfail_ok  REJECT-BAD=$xfail_bad"
if [ "$scored" -gt 0 ]; then
  pct=$(awk "BEGIN{printf \"%.1f\", ($correct/$scored)*100}")
  echo "correct: ${correct}/${scored} = ${pct}%   (pass ${pass} + correctly-rejected ${xfail_ok})"
fi
if [ "$compfail" -gt 0 ] && [ -s "$ERRCODES_LOG" ]; then
  echo "--- compile-error codes (count) ---"
  sort "$ERRCODES_LOG" | uniq -c | sort -nr | awk '{print $2, $1}'
fi
echo "full log: $LOG"

failures=$((compfail + runfail + assertfail + timeout_n + xfail_bad))
if [ "$failures" -ne 0 ]; then
  echo "gate: FAIL ($failures incorrect corpus outcomes)" >&2
  head -n 50 "$LOG" >&2
  exit 1
fi
echo "gate: PASS"

