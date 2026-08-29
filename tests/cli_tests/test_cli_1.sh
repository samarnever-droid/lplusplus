#!/bin/sh
set -e
echo "CLI Test 1: Compile file"
LPP_EMULATOR=1 LPP_TEST_MODE=1 target/release/lpp tests/arith.lpp
