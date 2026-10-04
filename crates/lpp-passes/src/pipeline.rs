//! Level-driven entry points for the 4F optimizer.

use lpp_common::OptimizationLevel;
use lpp_mir::{MirProgram, mir_snapshot};
use lpp_types::TypeInterner;

use crate::passes::{BranchFoldPass, ConstFoldPass, ConstPropPass};
use crate::{PassManager, PassManagerError, PassManagerOutcome};

/// One canonical optimization sweep: `const-fold`, then `const-prop`,
/// then `branch-fold`, each revalidated by the manager afterwards.
#[must_use]
pub fn optimization_passes() -> PassManager {
    let mut manager = PassManager::new();
    manager.push(ConstFoldPass);
    manager.push(ConstPropPass);
    manager.push(BranchFoldPass);
    manager
}

/// Run the canonical sweep up to the level's
/// `fixed_point_iterations` times (O0: 1, O1: 2, Oz: 2, Os: 3, O2: 4,
/// O3: 6 — the budget table is the source of truth), stopping early
/// as soon as a sweep changes nothing. Returns the manager outcome of
/// the last sweep.
pub fn run_optimization(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
) -> Result<PassManagerOutcome, PassManagerError> {
    let sweeps = level.budget().fixed_point_iterations;
    let mut outcome: Option<PassManagerOutcome> = None;
    for _ in 0..sweeps {
        let before = mir_snapshot(program);
        outcome = Some(optimization_passes().run(program, types)?);
        if mir_snapshot(program) == before {
            break;
        }
    }
    Ok(outcome.expect("every optimization budget allows at least one sweep"))
}
