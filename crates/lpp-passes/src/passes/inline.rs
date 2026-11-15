//! A bounded whole-program MIR inliner for tiny pure scalar functions.
//!
//! This is the safe first LTO slice: all L++ modules are already present in a
//! single `MirProgram`, so direct calls between them can be simplified before
//! code generation. Only a one-block function with at most one pure scalar
//! assignment is eligible. It creates no MIR entities and cannot duplicate
//! ownership effects, calls, or stores. It is not object-file LTO and does not
//! optimize the runtime or foreign libraries.

use std::collections::BTreeSet;

use lpp_mir::{
    Constant, InstructionId, InstructionKind, MirFunctionId, MirInvariant, MirLocalId, MirProgram,
    Operand, Rvalue, Terminator,
};
use lpp_types::{PrimitiveType, TypeInterner, TypeKind};

use crate::{MirPass, PassContext, PassFailure};

const IN_PLACE_PRESERVED: &[MirInvariant] = &[
    MirInvariant::References,
    MirInvariant::ControlFlow,
    MirInvariant::Types,
    MirInvariant::DefiniteInitialization,
    MirInvariant::Ownership,
    MirInvariant::NoOwningCycles,
    MirInvariant::OwnershipBalance,
];

/// Inline tiny direct callees. `hot_callees = None` enables every eligible
/// callee; `Some(ids)` restricts the pass to a profile-selected hot set.
#[derive(Debug, Clone, Default)]
pub struct InlinePass {
    hot_callees: Option<BTreeSet<MirFunctionId>>,
}

impl InlinePass {
    #[must_use]
    pub fn whole_program() -> Self {
        Self { hot_callees: None }
    }

    #[must_use]
    pub fn profile_guided(hot_callees: BTreeSet<MirFunctionId>) -> Self {
        Self {
            hot_callees: Some(hot_callees),
        }
    }
}

impl MirPass for InlinePass {
    fn name(&self) -> &'static str {
        "whole-program-inline"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        IN_PLACE_PRESERVED
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let callers: Vec<_> = program.functions().map(|(id, _)| id).collect();
        for caller_id in callers {
            let Some(caller) = program.function(caller_id) else {
                continue;
            };
            let blocks = program.function_blocks(caller).to_vec();
            for block_id in blocks {
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                let instructions: Vec<InstructionId> = program.block_instructions(block).to_vec();
                for instruction_id in instructions {
                    let Some(instruction) = program.instruction(instruction_id) else {
                        continue;
                    };
                    let InstructionKind::Assign {
                        target,
                        value:
                            Rvalue::Call {
                                callee: Operand::Function(callee_id),
                                arguments,
                            },
                    } = instruction.kind
                    else {
                        continue;
                    };
                    if caller_id == callee_id
                        || self
                            .hot_callees
                            .as_ref()
                            .is_some_and(|hot| !hot.contains(&callee_id))
                    {
                        continue;
                    }
                    let arguments = program.operands(arguments).to_vec();
                    let Some(replacement) =
                        inline_expression(program, context.types, callee_id, &arguments)
                    else {
                        continue;
                    };
                    if let Some(instruction) = program.instruction_mut(instruction_id) {
                        instruction.kind = InstructionKind::Assign {
                            target,
                            value: replacement,
                        };
                    }
                }
            }
        }
        Ok(())
    }
}

fn inline_expression(
    program: &MirProgram,
    types: &TypeInterner,
    callee_id: MirFunctionId,
    arguments: &[Operand],
) -> Option<Rvalue> {
    let callee = program.function(callee_id)?;
    if callee.kind != lpp_mir::MirFunctionKind::Function
        || !program.function_captures(callee).is_empty()
        || !is_scalar(types, callee.return_type)
    {
        return None;
    }
    let params = program.function_parameters(callee);
    if params.len() != arguments.len()
        || params.iter().any(|param| {
            program
                .local(*param)
                .is_none_or(|local| !is_scalar(types, local.ty))
        })
    {
        return None;
    }
    let blocks = program.function_blocks(callee);
    if blocks.len() != 1 || blocks[0] != callee.entry {
        return None;
    }
    let block = program.block(callee.entry)?;
    let instructions = program.block_instructions(block);
    match (instructions, block.terminator) {
        ([], Terminator::Return(Some(returned))) => Some(Rvalue::Use(substitute_operand(
            returned, params, arguments,
        )?)),
        ([instruction_id], Terminator::Return(Some(Operand::Copy(return_local)))) => {
            let instruction = program.instruction(*instruction_id)?;
            let InstructionKind::Assign { target, value } = instruction.kind else {
                return None;
            };
            if target != return_local
                || program.local(target).is_none_or(|local| {
                    local.kind != lpp_mir::MirLocalKind::Temporary || !is_scalar(types, local.ty)
                })
            {
                return None;
            }
            substitute_rvalue(value, params, arguments)
        }
        _ => None,
    }
}

fn substitute_rvalue(
    value: Rvalue,
    params: &[MirLocalId],
    arguments: &[Operand],
) -> Option<Rvalue> {
    Some(match value {
        Rvalue::Use(operand) => Rvalue::Use(substitute_operand(operand, params, arguments)?),
        Rvalue::Unary { operator, operand } => Rvalue::Unary {
            operator,
            operand: substitute_operand(operand, params, arguments)?,
        },
        Rvalue::Binary {
            left,
            operator,
            right,
        } => Rvalue::Binary {
            left: substitute_operand(left, params, arguments)?,
            operator,
            right: substitute_operand(right, params, arguments)?,
        },
        Rvalue::Cast { value, target } => Rvalue::Cast {
            value: substitute_operand(value, params, arguments)?,
            target,
        },
        _ => return None,
    })
}

fn substitute_operand(
    operand: Operand,
    params: &[MirLocalId],
    arguments: &[Operand],
) -> Option<Operand> {
    match operand {
        Operand::Copy(local) => {
            let index = params.iter().position(|parameter| *parameter == local)?;
            arguments.get(index).copied()
        }
        Operand::Constant(constant) if is_scalar_constant(constant) => Some(operand),
        _ => None,
    }
}

fn is_scalar(types: &TypeInterner, ty: lpp_types::TypeId) -> bool {
    matches!(
        types.kind(ty),
        TypeKind::Primitive(
            PrimitiveType::Bool
                | PrimitiveType::Int
                | PrimitiveType::Float
                | PrimitiveType::Char
                | PrimitiveType::U8
                | PrimitiveType::U16
                | PrimitiveType::U32
                | PrimitiveType::U64
                | PrimitiveType::USize
                | PrimitiveType::I8
                | PrimitiveType::I16
                | PrimitiveType::I32
                | PrimitiveType::ISize
                | PrimitiveType::RawPtr
        ) | TypeKind::Ptr(_)
    )
}

fn is_scalar_constant(constant: Constant) -> bool {
    matches!(
        constant,
        Constant::Integer(_)
            | Constant::FloatBits(_)
            | Constant::Bool(_)
            | Constant::Character { .. }
    )
}
