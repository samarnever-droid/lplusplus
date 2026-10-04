//! The Phase 5F cross-backend safety-exit crate.
//!
//! This crate has no production code. Its single integration test
//! (`tests/phase5f_gate.rs`) is the backend safety exit: it drives all three
//! backends (Cranelift, WASM, LLVM) on the shared scalar corpus against the
//! reference oracle, establishes a deterministic object-size baseline, and
//! runs the native backends under the sanitizers.
