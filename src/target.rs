//! Compatibility re-export of the shared Phase 1 target model.
//!
//! New compiler stages import this contract from `lpp-common` directly. The
//! legacy pipeline keeps its existing `crate::target` path during migration.

pub use lpp_common::target::{TargetSpec, is_wasm_triple_str};
