#!/usr/bin/env sh
# Validates that the Rust staticlib is callable through the stable C ABI that
# generated L++ programs will use. No network client executable or cURL is used.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-net-ffi.XXXXXX")
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM
: "${CC:=cc}"
command -v cargo >/dev/null
command -v "$CC" >/dev/null
cargo build --manifest-path "$ROOT/runtime/lpp-net/Cargo.toml" --release --locked
cat > "$TMP/client.c" <<'C'
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int64_t lpp_net_rs_connect(const char *, int64_t, int64_t);
int64_t lpp_net_rs_send_all(int64_t, const char *);
int64_t lpp_net_rs_set_timeout(int64_t, int64_t);
char *lpp_net_rs_recv(int64_t, int64_t);
void lpp_net_rs_free_string(char *);
void lpp_net_rs_close(int64_t);
int main(int argc, char **argv) {
  if (argc != 2) return 2;
  int64_t s = lpp_net_rs_connect("127.0.0.1", atoll(argv[1]), 2000);
  if (!s || !lpp_net_rs_set_timeout(s, 2000)) return 10;
  if (lpp_net_rs_send_all(s, "ping") != 4) return 11;
  char *reply = lpp_net_rs_recv(s, 32);
  int ok = reply != NULL && strcmp(reply, "pong") == 0;
  lpp_net_rs_free_string(reply);
  lpp_net_rs_close(s);
  return ok ? 0 : 12;
}
C
"$CC" -std=c11 -Wall -Wextra -Werror "$TMP/client.c" \
  "$ROOT/runtime/lpp-net/target/release/liblpp_net_runtime.a" \
  -ldl -lm -lpthread -o "$TMP/client"
python3 - "$TMP/port" <<'PY' &
import socket, sys
listener = socket.socket()
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 0)); listener.listen(1)
open(sys.argv[1], "w").write(str(listener.getsockname()[1]))
client, _ = listener.accept()
assert client.recv(4) == b"ping"
client.sendall(b"pong")
client.close(); listener.close()
PY
server=$!
for _ in $(seq 1 100); do [ -s "$TMP/port" ] && break; sleep 0.01; done
[ -s "$TMP/port" ]
"$TMP/client" "$(cat "$TMP/port")"
wait "$server"
echo 'PASS Rust networking C ABI loopback'
