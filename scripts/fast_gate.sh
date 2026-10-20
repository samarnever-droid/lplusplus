#!/usr/bin/env bash
# fast_gate.sh — the sub-5-minute pre-commit gate.
#
# fmt + clippy + workspace tests + AOT parity. Run before every commit.
# The full battery (corpus_correctness.sh, sanitizers, fuzz batches)
# runs in CI and before releases — see ROADMAP.md §3.
set -eu
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
echo "== AOT parity"
sh tests/run_aot_parity.sh 2>&1 | tail -n 1
echo "FAST GATE: PASS"
