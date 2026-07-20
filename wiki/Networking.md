# Networking

L++ networking must be native, auditable, and independent of cURL. It must not spawn external commands to perform network operations.

## Current public TCP compatibility API

```text
net_connect(host, port) -> handle or 0
net_listen(port) -> handle or 0
net_accept(listener) -> handle or 0
net_send(socket, data) -> bytes or -1
net_send_all(socket, data) -> bytes or -1
net_set_timeout(socket, milliseconds) -> 1 or 0
net_recv(socket, max_bytes) -> String
net_close(handle)
```

`net_send` and `net_send_all` have complete-write semantics. A successful OS `send` may write only a prefix, which is unsafe for HTTP and framed protocols.

## Rust runtime direction

`runtime/lpp-net/` is the production-oriented static runtime foundation. It exposes an isolated migration ABI (`lpp_net_rs_*`) while the compiler/linker is prepared to choose exactly one networking runtime per executable.

Implemented in the Rust ABI:

- TCP dial with DNS candidate iteration
- TCP listen/accept
- complete TCP writes and deadlines
- connected UDP bind/connect/send/receive
- monotonic opaque handles
- per-socket locking
- Rust-owned returned-string release API

The Rust runtime currently uses `std::net`. It is not yet an async Go-style scheduler. The next approved direction is a reviewed reactor based on `mio`, then L++ task/cancellation semantics, then TLS through `rustls` and HTTP facilities.

## Security boundaries

- Current string ABI rejects binary payloads containing NUL; byte buffers are required before binary protocols are exposed safely.
- TLS, HTTP, WebSockets, HTTP/2, HTTP/3, proxies, and cancellation are not complete.
- Do not describe a raw socket handle API as Go-level networking.

## v0.1.3 current-status note

This page is maintained with the project, but current support claims are
centralized in [Current Capabilities](../documentation/CURRENT_CAPABILITIES.md).

```text
Use LppData/build/release and LppData/cache for package artifacts.
Use host-linked AOT for filesystem/networking work.
Do not assume direct ELF supports files, networking, JSON, or threads.
Do not claim language-wide Rust-equivalent safety outside the verified AOT subset.
```
