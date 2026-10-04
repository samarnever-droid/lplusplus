//! Legacy shim — the direct-linker engine moved to the `lpp-linker` crate
//! (Phase 6A, `docs/rewrite/PHASE_6.md`). The public surface is identical,
//! so `lpp::linker::*` call sites (pm, legacy driver, `lpp-link` binary)
//! keep working unchanged.
pub use lpp_linker::*;
