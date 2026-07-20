# L++ networking: native-socket foundation

## Status — first production-oriented milestone

L++ networking is built on operating-system socket APIs directly. It does **not** shell out to, link to, or wrap cURL. The host-link runtime supports native Winsock on Windows and BSD/POSIX sockets on Linux and macOS.

Current supported transport is blocking TCP. It is deliberately small, auditable, and useful for clients, services, and protocol implementations:

```lpp
def main():
    socket := net_connect("127.0.0.1", 8080)
    if socket == 0:
        print("connect failed")
        return

    net_set_timeout(socket, 5000)
    sent := net_send_all(socket, "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
    if sent < 0:
        print("write failed")
    else:
        print(net_recv(socket, 4096))
    net_close(socket)
```

## API

| Function | Result | Contract |
|---|---:|---|
| `net_connect(host, port)` | socket handle / `0` | DNS resolution plus TCP connection. IPv4 and IPv6 candidates are tried. |
| `net_listen(port)` | listener handle / `0` | Binds a TCP IPv4 listener. |
| `net_accept(listener)` | socket handle / `0` | Accepts one TCP connection. |
| `net_send(socket, data)` | bytes / `-1` | Compatibility name; now completes the full string write. |
| `net_send_all(socket, data)` | bytes / `-1` | Retries partial OS writes until all UTF-8 bytes are sent or an error occurs. |
| `net_recv(socket, max_bytes)` | string | Reads at most `max_bytes`; empty string means EOF/error/timeout. |
| `net_set_timeout(socket, milliseconds)` | `1` / `0` | Sets both read and write deadlines; positive timeouts only. |
| `net_close(handle)` | void | Releases the OS socket handle. Calling twice is safe. |

Socket handles are owned resources today, but must be closed explicitly. A future `Socket` standard-library type will make lifetime ownership automatic in L++ source.

## Engineering guarantees and boundaries

- Native OS sockets only: no cURL dependency and no subprocess networking.
- `net_send_all` prevents a common correctness bug: `send` can return a successful **partial** write.
- Timeout configuration is applied to both reads and writes.
- SIGPIPE is suppressed where the operating system provides `MSG_NOSIGNAL`, so writes to a closed peer return failure rather than killing a Linux/macOS process.
- The return-value convention is intentionally explicit while L++ gains a `Result[T, NetError]` type: handle `0` and byte count `-1` signal failure.

Not yet delivered: TLS, HTTP parsing/client helpers, UDP, proxies, non-blocking poller/event loop, HTTP/2/3, WebSockets, cancellation, structured errors, and automatic socket lifetime. These require language-level `Result`, byte buffers, and async/runtime design; they must not be advertised as finished.

## Road to Go-class networking

1. **Correct native blocking TCP** — socket lifecycle, partial-write safety, deadlines, and loopback tests. *(current milestone)*
2. **Typed standard API** — `Socket`, `Listener`, `Addr`, `Result`, byte buffers, DNS and UDP.
3. **Poller runtime** — epoll/kqueue/IOCP behind one reactor API, cancellation, backpressure, and owned tasks.
4. **Protocols** — HTTP/1.1 parser/client/server; TLS through a reviewed native TLS backend (never cURL); WebSocket.
5. **Go-level developer experience** — `go`/tasks, channels, context/deadlines, race and leak tests, benchmarks, HTTP/2 and HTTP/3 where the runtime can uphold their guarantees.

## Current v0.1.3 status note — 2026-07-20

This document is historical/design context. For current public capability claims,
platform boundaries, filesystem APIs, package cache layout, and known missing
features, see [Current Capabilities](CURRENT_CAPABILITIES.md).

Current rules:

```text
- Do not claim fixed compile-time, binary-size, or C/Rust parity numbers.
- Do not claim language-wide Rust-equivalent safety.
- Host-linked AOT is the compatibility path for filesystem and networking work.
- Linux direct ELF remains a verified subset; filesystem/networking are not direct-link features yet.
- macOS ARM64 static direct output is rejected; dynamic libSystem imports are required.
- L++ package outputs/cache are LppData/build/release and LppData/cache.
```
