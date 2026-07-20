# S0 Native Safety Baseline

S0 is the entry level for every runtime, linker, ABI, and ownership change. It does not certify memory safety. It creates reproducible evidence and a known failing baseline if a famous safety tool finds a defect.

## Required commands

```sh
cargo test --manifest-path runtime/lpp-net/Cargo.toml --locked
CFLAGS='-O1 -g -fno-omit-frame-pointer -fsanitize=address,undefined' \
  ASAN_OPTIONS='detect_leaks=1:halt_on_error=1' \
  UBSAN_OPTIONS='halt_on_error=1:print_stacktrace=1' \
  sh tests/test_rust_network_adapter.sh
RUNNER='valgrind --leak-check=full --show-leak-kinds=all --errors-for-leak-kinds=definite --error-exitcode=97' \
  sh tests/test_rust_network_adapter.sh
```

## Required evidence record

Each safety-sensitive pull request records platform, compiler versions, exact commands, exit status, and any sanitizer suppression. Suppressions require a linked issue, expiration condition, and reviewer approval.

## Promotion rule

S0 only proves that the tool gate exists and was executed. S1 additionally requires deterministic rejection for unsupported ownership behavior. S2 requires ownership/MIR/runtime/negative/parity evidence as defined in `documentation/Safety_Mission.md`.
