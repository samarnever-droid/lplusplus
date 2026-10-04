//! The ownership plan as a pass over the validated MIR pass manager.

use lpp_mir::{MirInvariant, MirProgram};
use lpp_passes::{MirPass, PassContext, PassFailure};

use crate::plan::compute_ownership_plan;
use crate::verify::verify_ownership_plan;

/// The invariants the ownership-plan pass preserves: the core MIR
/// invariants plus the ownership invariants the plan proves.
const OWNERSHIP_PRESERVED: &[MirInvariant] = &[
    MirInvariant::References,
    MirInvariant::ControlFlow,
    MirInvariant::Types,
    MirInvariant::DefiniteInitialization,
    MirInvariant::Ownership,
    MirInvariant::NoOwningCycles,
];

/// Computes and proves the ownership plan, establishing the `Ownership`
/// and `NoOwningCycles` invariants in the manager's bookkeeping. The
/// pass never mutates the program.
pub struct OwnershipPlanPass;

impl MirPass for OwnershipPlanPass {
    fn name(&self) -> &'static str {
        "ownership-plan"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        OWNERSHIP_PRESERVED
    }

    fn established(&self) -> &'static [MirInvariant] {
        &[MirInvariant::Ownership, MirInvariant::NoOwningCycles]
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let plan = compute_ownership_plan(program, context.types)
            .map_err(|_| PassFailure {
                message: "ownership plan construction failed",
            })?;
        if verify_ownership_plan(program, context.types, &plan).is_empty() {
            Ok(())
        } else {
            Err(PassFailure {
                message: "ownership plan proof failed",
            })
        }
    }
}
