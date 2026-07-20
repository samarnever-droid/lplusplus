# Roadmap

## Near-term correctness

- Finish migration packaging from C network compatibility runtime to selected Rust static runtime.
- Add a typed result/error model and byte-buffer ABI.
- Add native DNS/error diagnostics and leak/cancellation tests.
- Preserve direct-link negative tests and platform rejection behavior.

## Networking progression

1. Blocking native TCP and connected UDP runtime foundation — in progress.
2. Compiler-selected Rust runtime and unified allocation ownership.
3. Byte buffers, typed sockets/listeners, `Result` errors.
4. Reactor abstraction with epoll/kqueue/IOCP via a reviewed Rust component.
5. L++ async tasks, cancellation, deadlines, and backpressure.
6. TLS through rustls and HTTP/1.1.
7. HTTP/2, WebSockets, and only then evaluate QUIC/HTTP/3.

## Non-goals until supported

- Claiming complete Go-level networking before async/runtime/protocol work exists.
- Claiming full language-wide safety guarantee outside the verified subset.
- Emitting macOS ARM64 static native executables that the OS will kill.
- Using cURL as a substitute for a native L++ networking runtime.

## v0.1.3 documentation status

For the current supported subset and explicit feature boundaries, see
[`documentation/CURRENT_CAPABILITIES.md`](../../documentation/CURRENT_CAPABILITIES.md).

Do not use historical benchmark numbers or roadmap text as current guarantees.
