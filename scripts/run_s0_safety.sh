#!/usr/bin/env sh
# One reproducible local entry point for the mandatory S0 safety tool baseline.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
command -v cargo >/dev/null || { echo 'cargo is required for S0'; exit 2; }
cargo test --manifest-path runtime/lpp-net/Cargo.toml --locked
CFLAGS='-O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined' \
ASAN_OPTIONS='detect_leaks=1:halt_on_error=1' \
UBSAN_OPTIONS='halt_on_error=1:print_stacktrace=1' \
  sh tests/test_rust_network_adapter.sh
if command -v valgrind >/dev/null 2>&1; then
  RUNNER='valgrind --leak-check=full --show-leak-kinds=all --errors-for-leak-kinds=definite --error-exitcode=97' \
    sh tests/test_rust_network_adapter.sh
else
  echo 'S0 NOTE: Valgrind unavailable locally; CI runs Memcheck.' >&2
fi
