use crate::mir::ir::*;

/// MIR Transformation Pass: Closure Lowering
///
/// This pass finds `Rvalue::Closure` or `Operand::Closure` (to be added)
/// and extracts them into flat, top-level `MirFunction`s.
/// It also generates the "Environment" struct to capture external variables
/// and modifies the closure invocation to pass this environment pointer.
pub fn run_closure_lowering_pass(_program: &mut MirProgram) {
    // TODO: Extract closures, build env structs, update calls
}
