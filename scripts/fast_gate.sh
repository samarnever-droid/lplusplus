#!/usr/bin/env bash
# fast_gate.sh — the sub-5-minute pre-commit gate.
#
# fmt + clippy + workspace tests + bindgen source/LSan gates + AOT parity. Run before every commit.
# The full battery (corpus_correctness.sh, sanitizers, fuzz batches)
# runs in CI and before releases — see ROADMAP.md §3.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

echo "== cargo fmt --check"
cargo fmt --check
echo "== cargo clippy (workspace, all targets)"
cargo clippy --locked --workspace -j2 --all-targets 2>&1 | tail -n 2
echo "== cargo test --workspace"
cargo test --locked --workspace -j2 2>&1 | awk '/test result/ {p+=$4; f+=$6} END {printf "workspace: %d passed, %d failed\n", p, f; if (f > 0) exit 1}'
echo "== build lpp driver"
cargo build --locked -j2 -p lpp 2>&1 | tail -n 1
echo "== lpp-bindgen source compilation"
LPP="$ROOT/target/debug/lpp" python3 scripts/check_bindgen_sources.py
echo "== lpp-bindgen stale-context ASan/LSan/UBSan gate"
LPP="$ROOT/target/debug/lpp" bash scripts/check_bindgen_lsan.sh
echo "== AOT parity"
sh tests/run_aot_parity.sh 2>&1 | tail -n 1
echo "FAST GATE: PASS"
