//! Level-driven entry points for the 4F optimizer.

use std::collections::BTreeSet;

use lpp_common::OptimizationLevel;
use lpp_mir::{MirFunctionId, MirProgram, mir_snapshot};
use lpp_types::TypeInterner;

use crate::passes::{BranchFoldPass, ConstFoldPass, ConstPropPass, InlinePass};
use crate::{PassManager, PassManagerError, PassManagerOutcome};

/// One canonical local optimization sweep: `const-fold`, `const-prop`, then
/// `branch-fold`, each revalidated by the manager afterwards.
#[must_use]
pub fn optimization_passes() -> PassManager {
    let mut manager = PassManager::new();
    manager.push(ConstFoldPass);
    manager.push(ConstPropPass);
    manager.push(BranchFoldPass);
    manager
}

/// Run the canonical sweep up to the level's fixed-point budget (O0: 1, O1:
/// 2, O2: 4, O3: 6, Os: 3, Oz: 2), stopping when a sweep changes nothing.
pub fn run_optimization(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
) -> Result<PassManagerOutcome, PassManagerError> {
    run_optimization_internal(program, types, level, None)
}

/// Run local optimization plus a safe, whole-program MIR inlining pass.
///
/// All L++ modules are already merged into one `MirProgram`, so this lets
/// direct calls across source modules inline before object emission. The
/// current inliner only substitutes tiny pure scalar callees; this is not
/// linker/object LTO and does not optimize the runtime or foreign libraries.
pub fn run_optimization_with_lto(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
    enable_lto: bool,
) -> Result<PassManagerOutcome, PassManagerError> {
    let inline = enable_lto.then(InlinePass::whole_program);
    run_optimization_internal(program, types, level, inline)
}

/// Run local optimization with profile-guided inlining restricted to the
/// provided hot-callee set. Explicit LTO takes precedence and inlines every
/// eligible tiny scalar callee.
pub fn run_optimization_with_profile(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
    enable_lto: bool,
    hot_callees: BTreeSet<MirFunctionId>,
) -> Result<PassManagerOutcome, PassManagerError> {
    let inline = if enable_lto {
        Some(InlinePass::whole_program())
    } else if hot_callees.is_empty() {
        None
    } else {
        Some(InlinePass::profile_guided(hot_callees))
    };
    run_optimization_internal(program, types, level, inline)
}

fn run_optimization_internal(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
    inline: Option<InlinePass>,
) -> Result<PassManagerOutcome, PassManagerError> {
    let sweeps = level.budget().fixed_point_iterations;
    let mut outcome: Option<PassManagerOutcome> = None;
    for sweep in 0..sweeps {
        let before = if sweeps > 1 && sweep + 1 < sweeps {
            Some(mir_snapshot(program))
        } else {
            None
        };
        let mut manager = optimization_passes();
        if let Some(inline) = inline.clone() {
            manager.push(inline);
        }
        outcome = Some(manager.run(program, types)?);
        if let Some(before) = before
            && mir_snapshot(program) == before
        {
            break;
        }
    }
    Ok(outcome.expect("every optimization budget allows at least one sweep"))
}
