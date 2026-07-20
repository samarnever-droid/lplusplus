# Safety corpus

This directory records safety contracts and negative cases for L++.

`tests/run_aot_parity.sh` already exercises supported ownership behavior and rejection cases. The safety mission gate ensures those rejection cases remain explicit and that user-facing documentation does not make a premature Rust-equivalence claim.

Add a named regression test here or under `tests/` whenever a bug could cause use-after-free, double release, ownership-cycle leakage, invalid backend output, allocator mismatch, or silent unsupported-feature compilation.
