#!/usr/bin/env sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-net-adapter.XXXXXX")
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
: "${CC:=cc}"
CFLAGS=${CFLAGS:-}
RUNNER=${RUNNER:-}
cargo build --manifest-path "$ROOT/runtime/lpp-net/Cargo.toml" --release --locked
cat > "$TMP/client.c" <<'C'
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
int64_t lpp_net_connect(const char *, int64_t); int64_t lpp_net_send_all(int64_t, const char *);
int64_t lpp_net_set_timeout(int64_t, int64_t); char *lpp_net_recv(int64_t, int64_t); void lpp_net_close(int64_t);
int main(int argc, char **argv) { if (argc != 2) return 2; int64_t s=lpp_net_connect("127.0.0.1",atoll(argv[1])); if (!s||!lpp_net_set_timeout(s,2000)||lpp_net_send_all(s,"ping")!=4) return 10; char *r=lpp_net_recv(s,32); int ok=r&&strcmp(r,"pong")==0; free(r); lpp_net_close(s); return ok?0:11; }
C
# CFLAGS is intentionally configurable for ASan/UBSan safety missions.
# shellcheck disable=SC2086
"$CC" $CFLAGS -std=c11 -Wall -Wextra -Werror -Wno-unused-function -Wno-unused-variable -Wno-unused-parameter -DLPP_NO_NETWORK -I"$ROOT/runtime" -I"$ROOT/runtime/lpp-net/include" "$TMP/client.c" "$ROOT/lpp_runtime.c" "$ROOT/runtime/lpp_net_rust_adapter.c" "$ROOT/runtime/lpp-net/target/release/liblpp_net_runtime.a" -ldl -lm -lpthread -o "$TMP/client"
python3 - "$TMP/port" <<'PY' &
import socket,sys
s=socket.socket();s.bind(('127.0.0.1',0));s.listen(1);open(sys.argv[1],'w').write(str(s.getsockname()[1]));c,_=s.accept();assert c.recv(4)==b'ping';c.sendall(b'pong');c.close();s.close()
PY
p=$!; for _ in $(seq 1 100); do [ -s "$TMP/port" ] && break;sleep .01;done
# RUNNER permits Valgrind/Memcheck in the S0 safety gate without duplicating
# the native ABI test. It is intentionally word-split for command + arguments.
# shellcheck disable=SC2086
$RUNNER "$TMP/client" "$(cat "$TMP/port")";wait "$p";echo 'PASS Rust network compatibility adapter'
