#!/usr/bin/env bash
set -euo pipefail

echo "========================================"
echo "  L++ FAST LOCAL CI VERIFICATION (UNIX)"
echo "========================================"

echo -e "\n[1/5] Running Rust unit tests & symbol parity gate..."
cargo test --locked

echo -e "\n[2/5] Building release binaries (lpp, lpp-link)..."
cargo build --release --bin lpp --bin lpp-link

echo -e "\n[3/5] Testing direct ELF linking..."
sh tests/test_lpp_link_elf.sh

echo -e "\n[4/5] Testing AOT parity..."
sh tests/run_aot_parity.sh

echo -e "\n[5/5] Running core syntax tests..."
target/release/lpp tests/test_augmented_assign.lpp
target/release/lpp tests/test_index.lpp
target/release/lpp tests/test_string_ops.lpp
target/release/lpp tests/test_struct_constructor.lpp

echo -e "\n========================================"
echo "  ALL LOCAL CI CHECKS PASSED (100% GREEN)"
echo "========================================"

