# lpp-net-runtime

`lpp-net-runtime` is the Rust native-networking foundation for L++. It builds a static library, `liblpp_net_runtime.a`, with a stable C ABI declared in:

```text
include/lpp_net_runtime.h
```

It uses native Rust/OS networking primitives. It does not use cURL, invoke a subprocess, or wrap an external network command.

## Build and test

```sh
cargo test --locked
cargo build --release --locked
sh ../../tests/test_rust_network_runtime.sh
```

## ABI ownership rule

`lpp_net_rs_recv` and `lpp_net_rs_udp_recv` return strings allocated by Rust. Callers must release them using `lpp_net_rs_free_string`; they must not pass them to `free` or the legacy `lpp_free_str` ABI.

## Migration status

The `lpp_net_rs_*` names intentionally remain separate from existing `lpp_net_*` compatibility symbols. The compiler/package layer must select one networking runtime per native executable before public L++ calls are switched. This prevents duplicate symbol definitions and cross-allocator bugs.
