#![forbid(unsafe_code)]
#![allow(
    clippy::collapsible_if,
    clippy::question_mark,
    clippy::needless_lifetimes,
    clippy::unnecessary_lazy_evaluations
)]

mod manager;
mod passes;
mod pipeline;

pub use manager::{
    MirPass, PassContext, PassFailure, PassManager, PassManagerError, PassManagerErrorKind,
    PassManagerOutcome,
};
pub use passes::{
    BranchFoldPass, ConstFoldPass, ConstPropPass, InlinePass, fold_binary, fold_unary,
    is_primitive_constant, single_def_constants,
};
pub use pipeline::{
    optimization_passes, run_optimization, run_optimization_with_lto, run_optimization_with_profile,
};
