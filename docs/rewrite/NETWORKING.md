# HTTP / Networking in the L++ rewrite — status and the path to support it

_Answering: "how would we do HTTP — from scratch, or lean on something underneath?"_

## TL;DR

HTTP is **not** a compiler feature and **not** a new set of builtins. The
frozen v1 ABI already reserves the low-level socket primitives (40 `net_*`
ids). The right design is:

1. **Underneath: reuse the host TCP stack** via Rust's `std::net`
   (`TcpListener` / `TcpStream` / `UdpSocket`) inside `lpp-runtime`. We do
   *not* write a TCP/IP stack from scratch — that would be reimplementing the
   kernel for no benefit.
2. **In between: implement the already-reserved `net_*` ABI symbols** as thin
   runtime functions over `std::net`, plus a small integer→socket handle
   registry (sockets are passed around L++ as `Int` fds, matching the ABI's
   `result = "i64"`).
3. **On top: write HTTP itself in L++** (a library module) that formats a
   request / parses a response as text over `net_send` / `net_recv`. HTTP is
   just framed bytes over a socket, so it belongs in userland, not the runtime.

This keeps the frozen ABI frozen (no ids added — see the schema lock in
`crates/lpp-runtime-abi/tests/schema.rs`), and matches how every other
capability (files, strings, lists, maps) is layered: a host-backed runtime
primitive + an L++ surface.

## Current implementation status

Native rewrite support has started:

* `crates/lpp-runtime/src/net.rs` implements the frozen `net_*` and `http_*`
  runtime symbols over `std::net`, using an opaque generational `i64` socket
  handle table.
* `crates/lpp-codegen-cranelift/src/lower.rs` now treats the network/HTTP
  builtins as a native lowering family and declares their `lpp_*` import
  signatures.
* `tests/network_loopback.lpp` is the first self-contained proof target: it
  listens, connects to itself, accepts, sends, receives, echoes, and closes
  without external peers or threads.

Still open: reason-aware corpus verification, WASM/LLVM parity, stronger async
blocking/capture rejection, and spawned/concurrent server support.

## Why it failed before this slice

* The 40 `feature = "network"` builtins (`net_dial`, `net_listen`,
  `net_accept`, `net_send`, `net_recv`, UDP variants, …) all carried
  `targets = ["legacy"]` in `abi/builtins.toml`.
* The rewrite had **no Rust implementation** of any of them
  (`crates/lpp-runtime/src/` had io/string/list/map/task/… but no `net`).
* The Cranelift backend treated a `legacy`-only builtin as
  `E5003: UnrepresentableBuiltin` — hence `network_echo_client.lpp` (uses
  `net_dial`, id 131) and friends failed to compile, and `network_echo_server`
  additionally tripped MIR's `InvalidSpawnTarget` (its accept-loop spawns a
  per-connection task).

So the original gap was three gaps stacked: no runtime code, a target gate that
forbade the symbols, and (for the server) the spawn model.

## The concrete plan (bounded, frozen-ABI-safe)

### 1. Runtime: `crates/lpp-runtime/src/net.rs`
* A process-global `SocketTable`: `Mutex<Slab<Socket>>` where
  `enum Socket { Listener(TcpListener), Stream(TcpStream), Udp(UdpSocket) }`.
  Hand out `i64` handles (index + generation) — never leak a raw pointer to
  L++, exactly like the task/arc handles already do.
* Implement each reserved symbol as `extern "C"`, matching the ABI signature:
  * `lpp_net_dial(host_ptr, host_len, port) -> i64` → `TcpStream::connect`,
    insert, return handle (or a negative errno-style code).
  * `lpp_net_listen(host, port) -> i64` → `TcpListener::bind`.
  * `lpp_net_accept(listener) -> i64` → `accept()`, insert the stream.
  * `lpp_net_send(sock, buf_ptr, buf_len) -> i64` → `write`.
  * `lpp_net_recv(sock, buf_ptr, cap) -> i64` → `read`, return byte count.
  * `lpp_net_close(sock) -> i64`, plus the UDP `*_udp` variants.
* Return values are `i64` throughout — already what the ABI declares, so no
  schema change.

### 2. Codegen / target model: un-gate the symbols on the native target
* These are Family-D-rejected only because `legacy` isn't in the native
  target's allow-set. Add a `net`-family arm to
  `crates/lpp-codegen-cranelift/src/lower.rs` that imports the runtime symbol
  directly (same mechanism used for the str-slice overloads: declare the
  import in `pre_scan`, emit a direct `import_ref` call — **no new BuiltinId**).
* Equivalent to how file I/O works today: the id exists in the frozen table,
  the backend routes it to a real runtime function instead of erroring.

### 3. Server spawn model
* `network_echo_server` spawns a task per accepted connection. The rewrite's
  spawn currently rejects a parameterized/looping target (`InvalidSpawnTarget`).
  Two options: (a) support a nullary task whose captured env holds the socket
  handle (the accept loop hands each connection to a spawned handler), or
  (b) ship a single-threaded blocking echo first and defer concurrency. Start
  with a blocking client (`network_echo_client`) to prove the stack, then do
  the server.

### 4. HTTP as an L++ library (no compiler work at all)
```
# std/http.lpp  (illustrative)
def http_get(host, path) -> Str:
    s := net_dial(host, 80)
    req := "GET " + path + " HTTP/1.0\r\nHost: " + host + "\r\n\r\n"
    net_send(s, req)
    mut body := ""
    loop:
        chunk := net_recv(s, 4096)
        if str_len(chunk) == 0: break
        body = body + chunk
    net_close(s)
    return body
```
Response parsing (status line, headers, chunked bodies) is ordinary string
work on top of the primitives — the same layering as the rest of the stdlib.

## What we explicitly do NOT do
* **No new builtins / no ABI edits.** The schema test hard-locks 518
  builtins / 354 symbols; the `net_*` ids already exist and are enough.
* **No hand-rolled TCP/IP or TLS.** `std::net` for TCP; if HTTPS is ever
  needed, link a vetted TLS crate (rustls) behind the same handle table
  rather than implementing crypto.
* **No async runtime requirement for v1.** Blocking sockets are sufficient to
  make the echo client/server correct; concurrency reuses the existing task
  model incrementally.

## Cost estimate
* Runtime `net.rs` + handle table: ~1 file, medium.
* Codegen net-family arm + pre_scan imports: small, mirrors existing arms.
* `net_dial`-only path unblocks `network_echo_client`; the server needs the
  spawn work above. `network_stress_test` is a separate load/timing test.
