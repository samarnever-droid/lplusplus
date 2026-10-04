//! `branch-fold` — CFG pass.
//!
//! Replaces a `Branch` whose condition is a known bool (a constant, or
//! a copy of a single-def Bool constant) with a `Goto` to the taken
//! arm. The abandoned arm becomes unreachable, which is legal
//! validated MIR (unreachable blocks never poison reachable state —
//! the 4B rule) and the interpreter executes such programs; the block
//! remains in the program.

use lpp_mir::{Constant, MirFunctionId, MirInvariant, MirProgram, Operand, Terminator};

use crate::passes::constfold::IN_PLACE_PRESERVED;
use crate::{MirPass, PassContext, PassFailure};

/// `branch-fold`: fold branches on known bool conditions.
#[derive(Debug, Clone, Copy, Default)]
pub struct BranchFoldPass;

impl MirPass for BranchFoldPass {
    fn name(&self) -> &'static str {
        "branch-fold"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        IN_PLACE_PRESERVED
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let function_ids: Vec<MirFunctionId> = program.functions().map(|(id, _)| id).collect();
        for function_id in function_ids {
            let Some((map, block_ids)) = function_shapes(program, function_id) else {
                continue;
            };
            for block_id in block_ids {
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                let Terminator::Branch {
                    condition,
                    then_block,
                    else_block,
                } = block.terminator
                else {
                    continue;
                };
                let taken = match condition {
                    Operand::Constant(Constant::Bool(value)) => Some(value),
                    Operand::Copy(local) => map.get(&local).and_then(|value| match value {
                        Constant::Bool(value) => Some(*value),
                        _ => None,
                    }),
                    _ => None,
                };
                let Some(taken) = taken else {
                    continue;
                };
                let target = if taken { then_block } else { else_block };
                if let Some(block) = program.block_mut(block_id) {
                    block.terminator = Terminator::Goto(target);
                }
            }
        }
        Ok(())
    }
}

/// Borrow the function's single-def constants and its block ids with
/// a single immutable borrow so the caller can mutate afterwards.
fn function_shapes<'p>(
    program: &'p MirProgram,
    function_id: MirFunctionId,
) -> Option<(
    std::collections::BTreeMap<lpp_mir::MirLocalId, Constant>,
    Vec<lpp_mir::BasicBlockId>,
)> {
    let function = program.function(function_id)?;
    Some((
        crate::passes::analysis::single_def_constants(program, function),
        program.function_blocks(function).to_vec(),
    ))
}
