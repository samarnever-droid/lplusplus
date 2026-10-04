//! `const-prop` — scalar pass.
//!
//! Replaces every inline `Operand::Copy(l)` of a single-def constant
//! local `l` with `Operand::Constant(c)`, in place. Operand lists
//! (call and builtin arguments, aggregate fields, closure captures)
//! live in the read-only list arena and are not touched. The single
//! def instruction is kept.

use std::collections::BTreeMap;

use lpp_mir::{
    BasicBlockId, Constant, InstructionKind, MirFunctionId, MirInvariant, MirLocalId, MirProgram,
    Operand, Rvalue, Terminator,
};

use crate::passes::analysis::single_def_constants;
use crate::passes::constfold::IN_PLACE_PRESERVED;
use crate::{MirPass, PassContext, PassFailure};

/// `const-prop`: propagate single-def primitive constants into their
/// inline read sites.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConstPropPass;

impl MirPass for ConstPropPass {
    fn name(&self) -> &'static str {
        "const-prop"
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
            if map.is_empty() {
                continue;
            }
            for block_id in block_ids {
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                let instruction_ids: Vec<_> = program.block_instructions(block).to_vec();
                for instruction_id in instruction_ids {
                    let Some(kind) = program.instruction(instruction_id).map(|i| i.kind) else {
                        continue;
                    };
                    if let Some(replacement) = rewrite_kind(&map, kind) {
                        if let Some(instruction) = program.instruction_mut(instruction_id) {
                            instruction.kind = replacement;
                        }
                    }
                }
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                if let Some(replacement) = rewrite_terminator(&map, block.terminator) {
                    if let Some(block) = program.block_mut(block_id) {
                        block.terminator = replacement;
                    }
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
) -> Option<(BTreeMap<MirLocalId, Constant>, Vec<BasicBlockId>)> {
    let function = program.function(function_id)?;
    Some((
        single_def_constants(program, function),
        program.function_blocks(function).to_vec(),
    ))
}

fn rewrite_operand(map: &BTreeMap<MirLocalId, Constant>, operand: Operand) -> Operand {
    if let Operand::Copy(local) = operand {
        if let Some(value) = map.get(&local) {
            return Operand::Constant(*value);
        }
    }
    operand
}

fn rewrite_rvalue(map: &BTreeMap<MirLocalId, Constant>, value: Rvalue) -> Rvalue {
    match value {
        Rvalue::Use(operand) => Rvalue::Use(rewrite_operand(map, operand)),
        Rvalue::Unary { operator, operand } => Rvalue::Unary {
            operator,
            operand: rewrite_operand(map, operand),
        },
        Rvalue::Binary {
            left,
            operator,
            right,
        } => Rvalue::Binary {
            left: rewrite_operand(map, left),
            operator,
            right: rewrite_operand(map, right),
        },
        Rvalue::ListLen(operand) => Rvalue::ListLen(rewrite_operand(map, operand)),
        Rvalue::Await(operand) => Rvalue::Await(rewrite_operand(map, operand)),
        Rvalue::Spawn(operand) => Rvalue::Spawn(rewrite_operand(map, operand)),
        // `Tuple`, `List`, `ConstructStruct`, `ConstructVariant`,
        // `Call`, `Builtin`, and `MakeClosure` hold their operands in
        // the read-only list arena: untouched.
        other => other,
    }
}

fn rewrite_kind(
    map: &BTreeMap<MirLocalId, Constant>,
    kind: InstructionKind,
) -> Option<InstructionKind> {
    match kind {
        InstructionKind::Assign { target, value } => {
            let replaced = rewrite_rvalue(map, value);
            (replaced != value).then_some(InstructionKind::Assign {
                target,
                value: replaced,
            })
        }
        InstructionKind::Store { place, value } => {
            let replaced = rewrite_operand(map, value);
            (replaced != value).then_some(InstructionKind::Store {
                place,
                value: replaced,
            })
        }
    }
}

fn rewrite_terminator(
    map: &BTreeMap<MirLocalId, Constant>,
    terminator: Terminator,
) -> Option<Terminator> {
    match terminator {
        Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            let replaced = rewrite_operand(map, condition);
            (replaced != condition).then_some(Terminator::Branch {
                condition: replaced,
                then_block,
                else_block,
            })
        }
        Terminator::SwitchEnum {
            subject,
            aggregate,
            targets,
        } => {
            let replaced = rewrite_operand(map, subject);
            (replaced != subject).then_some(Terminator::SwitchEnum {
                subject: replaced,
                aggregate,
                targets,
            })
        }
        Terminator::Return(value) => {
            let replaced = value.map(|operand| rewrite_operand(map, operand));
            (replaced != value).then_some(Terminator::Return(replaced))
        }
        _ => None,
    }
}
