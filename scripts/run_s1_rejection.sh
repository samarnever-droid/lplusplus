#!/usr/bin/env sh
# S1 safety gate: S0 tooling plus deterministic rejection/no-artifact tests.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
sh scripts/run_s0_safety.sh
# AOT parity includes successful ownership cases and explicit no-object rejection
# checks for unsafe list representations and ownership cycles.
sh tests/run_aot_parity.sh
cargo build --release --locked --bin lpp-link
sh tests/test_lpp_link_negative.sh
# The ARM policy test documents/guards rejection rather than silently emitting a
# static executable macOS will kill. It is a format-level test and runs on Unix.
sh tests/test_direct_macho_arm64.sh
echo 'PASS S1 deterministic rejection contract'
