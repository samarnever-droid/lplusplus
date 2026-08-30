#!/bin/bash
set -eu

LPP_EMULATOR=1 sh tests/test_source_commands.sh
LPP_EMULATOR=1 sh tests/cli_tests/test_cli_commands.sh
