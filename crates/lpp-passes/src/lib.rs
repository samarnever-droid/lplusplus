#![forbid(unsafe_code)]

mod manager;
mod passes;
mod pipeline;

pub use manager::{
    MirPass, PassContext, PassFailure, PassManager, PassManagerError, PassManagerErrorKind,
    PassManagerOutcome,
};
pub use passes::{
    BranchFoldPass, ConstFoldPass, ConstPropPass, fold_binary, fold_unary, is_primitive_constant,
    single_def_constants,
};
pub use pipeline::{optimization_passes, run_optimization};
