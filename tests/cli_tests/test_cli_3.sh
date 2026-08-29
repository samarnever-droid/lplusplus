#!/bin/sh
set -e
echo "CLI Test 3: Emit MIR"
LPP_EMULATOR=1 LPP_TEST_MODE=1 target/release/lpp emit mir tests/arith.lpp
