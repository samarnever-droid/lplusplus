# Rust networking runtime design

## Decision

L++ will not fork a complete programming language or copy another language's networking implementation. That creates licence, maintenance, ABI, and security-update debt while still failing to integrate with L++ ownership semantics.

Instead, L++ owns a small, documented network ABI and implements it in a dedicated Rust runtime:

```text
L++ API → compiler ABI call → lpp-net-runtime (Rust) → operating-system sockets
```

There is no cURL, no command-line fallback, and no subprocess boundary.

## Current artifact

`runtime/lpp-net/` is a Rust `staticlib` with a deliberately dependency-free TCP core. It exports separate `lpp_net_rs_*` symbols during migration so it cannot collide with the existing C host runtime.

The core provides:

- DNS-aware TCP dial, trying all resolved addresses
- IPv4 TCP listener/accept lifecycle
- complete writes (`write_all`)
- read and write timeouts
- opaque monotonic handles that are never recycled
- an explicit Rust-owned string free function
- bounded reads (16 MiB) and rejection of embedded-NUL payloads under the current `Str` ABI

## Migration rule

The existing C socket runtime remains the compatibility implementation. It must not be linked together with an ABI-compatible Rust implementation using identical symbol names. Migration happens only after compiler/link packaging selects exactly one runtime:

1. Build `lpp-net-runtime` as `liblpp_net_runtime.a`.
2. Add a package/runtime selector to link that archive with host-linked AOT programs.
3. Switch public `lpp_net_*` calls to the Rust ABI after string ownership is unified.
4. Retain C networking only as an explicit compatibility build option.
5. Add native integration tests on Linux, Windows, and macOS before changing the default.

## Async and protocol roadmap

`std::net` is chosen for the initial core because it is small and auditable. It does not claim to be a Go-equivalent async runtime. The next layer must be a reviewed Rust reactor built on `mio` (epoll on Linux, kqueue on macOS, IOCP on Windows), then task/cancellation primitives in L++.

The current Rust ABI also has connected UDP primitives; L++ source exposure waits for byte buffers and typed errors.

TLS should use `rustls`; HTTP should be built on a reviewed Rust HTTP parser/client/server stack. Those components will be selected only after licence and security-maintenance review. No source from Go or another language runtime is copied into L++.
