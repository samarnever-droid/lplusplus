use crate::mir::ir::*;

/// Compatibility hook for the old standalone closure pass.
///
/// Closures are lowered while building MIR in `mir::lower::MirLowerCtx` so
/// capture scopes, lifted functions, and environment records are created in a
/// single ownership-aware operation.  Running a second transformation here
/// would duplicate lifted functions.  Keep this hook as an explicit no-op for
/// downstream users that still call the historical pass entry point.
pub fn run_closure_lowering_pass(_program: &mut MirProgram) {}
