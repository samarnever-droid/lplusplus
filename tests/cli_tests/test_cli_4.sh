#!/bin/sh
set -e
echo "CLI Test 4: Emit Cranelift"
LPP_EMULATOR=1 LPP_TEST_MODE=1 target/release/lpp emit cranelift tests/arith.lpp
