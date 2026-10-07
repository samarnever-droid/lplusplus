#!/usr/bin/env bash
set -euo pipefail

echo "========================================"
echo "  L++ FAST LOCAL CI VERIFICATION (UNIX)"
echo "========================================"

echo -e "\n[1/10] Verifying frozen v1.2 compatibility fixtures..."
python3 scripts/check_compatibility_freeze.py
python3 -m json.tool tests/source_baseline.json >/dev/null
python3 -m py_compile scripts/check_source_baseline.py scripts/check_compatibility_freeze.py scripts/materialize_v1_oracle.py scripts/package_release.py
python3 tests/test_package_release.py

echo -e "\n[2/10] Checking Rust formatting..."
cargo fmt --all -- --check

echo -e "\n[3/10] Running blocking Clippy correctness and suspicious groups..."
cargo clippy --workspace --all-targets --locked -- -A warnings -D clippy::correctness -D clippy::suspicious

echo -e "\n[4/10] Requiring warning-clean rewrite crates..."
cargo clippy --locked -p lpp-common -p lpp-driver -p lpp-frontend -p lpp-hir -p lpp-types -p lpp-mir -p lpp-passes -p lpp-runtime-abi --all-targets -- -D warnings

echo -e "\n[5/10] Running Rust unit tests & symbol parity gate..."
cargo test --workspace --locked

echo -e "\n[6/10] Building release binaries (lpp, lpp-link)..."
cargo build --release --locked --bin lpp --bin lpp-link

echo -e "\n[7/10] Checking the classified L++ source baseline..."
python3 scripts/check_source_baseline.py target/release/lpp

echo -e "\n[8/10] Testing direct ELF linking..."
sh tests/test_lpp_link_elf.sh

echo -e "\n[9/10] Testing AOT parity..."
sh tests/run_aot_parity.sh

echo -e "\n[10/10] Running core syntax tests..."
target/release/lpp tests/test_augmented_assign.lpp
target/release/lpp tests/test_index.lpp
target/release/lpp tests/test_string_ops.lpp
target/release/lpp tests/test_struct_constructor.lpp

echo -e "\n========================================"
echo "  ALL LOCAL CI CHECKS PASSED (100% GREEN)"
echo "========================================"
