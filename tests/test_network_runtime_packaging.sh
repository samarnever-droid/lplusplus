#!/usr/bin/env sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
RELEASE="$ROOT/.github/workflows/release.yml"
grep -Fq 'liblpp_net_runtime.a' "$RELEASE"
grep -Fq 'lpp_net_runtime.h' "$RELEASE"
grep -Fq 'cargo build --manifest-path runtime/lpp-net/Cargo.toml --release --locked' "$RELEASE"
grep -Fq 'lpp_net_rs_connect' "$ROOT/runtime/lpp-net/include/lpp_net_runtime.h"
grep -Fq 'lpp_net_rs_udp_recv' "$ROOT/runtime/lpp-net/include/lpp_net_runtime.h"
echo 'PASS Rust network runtime release packaging contract'
