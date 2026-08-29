#!/bin/sh
set -e
echo "CLI Test 5: Emit LLVM IR"
LPP_EMULATOR=1 LPP_TEST_MODE=1 target/release/lpp emit llvm-ir tests/arith.lpp
