//! Shared, dependency-light contracts for the L++ rewrite.
//!
//! This crate owns stable source identities, byte spans, structured
//! diagnostics, and target selection. Compiler stages may depend on these
//! contracts, but this crate never depends on a compiler stage.

pub mod diagnostic;
pub mod optimization;
pub mod source;
pub mod target;

pub use diagnostic::{Diagnostic, DiagnosticCode, Label, Severity};
pub use optimization::{
    InvalidOptimizationLevel, OptimizationBudget, OptimizationLevel, OptimizationOptions,
};
pub use source::{FileId, SourceFile, SourceMap, SourceMapError, Span};
pub use target::{TargetFamily, TargetSpec, is_wasm_triple_str};
