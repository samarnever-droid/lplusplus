#!/bin/sh
set -e
echo "CLI Test 2: Emit AST"
LPP_EMULATOR=1 LPP_TEST_MODE=1 target/release/lpp emit ast tests/arith.lpp
