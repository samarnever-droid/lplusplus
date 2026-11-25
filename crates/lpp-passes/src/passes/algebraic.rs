//! `algebraic-simplify` — oracle-exact scalar algebraic rewrite & strength
//! reduction pass.
//!
//! Inspired by Cat/MetaCat's algebraic rewrite rules and Pliant's expression
//! simplification, this pass simplifies primitive `Int` and `Bool` rvalues in
//! place (additive/multiplicative/bitwise identities, absorbing elements,
//! power-of-two multiplication strength reduction, `x ** 0/1/2`, self-operand
//! cancellations, and boolean tautologies) while preserving every MIR
//! invariant including the ownership trio.

use lpp_mir::{
    BinaryOperator, Constant, InstructionId, InstructionKind, MirInvariant, MirProgram, Operand,
    Rvalue, UnaryOperator,
};
use lpp_types::{PrimitiveType, TypeInterner, TypeKind};

use crate::passes::constfold::IN_PLACE_PRESERVED;
use crate::{MirPass, PassContext, PassFailure};

/// `algebraic-simplify`: apply oracle-exact algebraic identities and
/// strength reductions to scalar `Int` and `Bool` rvalues in place.
#[derive(Debug, Clone, Copy, Default)]
pub struct AlgebraicSimplifyPass;

impl MirPass for AlgebraicSimplifyPass {
    fn name(&self) -> &'static str {
        "algebraic-simplify"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        IN_PLACE_PRESERVED
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let function_ids: Vec<_> = program.functions().map(|(id, _)| id).collect();
        for function_id in function_ids {
            let Some(function) = program.function(function_id) else {
                continue;
            };
            let block_ids: Vec<_> = program.function_blocks(function).to_vec();
            for block_id in block_ids {
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                let instruction_ids: Vec<InstructionId> =
                    program.block_instructions(block).to_vec();
                for instruction_id in instruction_ids {
                    let Some(replacement) =
                        simplify_instruction(program, context.types, instruction_id)
                    else {
                        continue;
                    };
                    if let Some(instruction) = program.instruction_mut(instruction_id) {
                        instruction.kind = replacement;
                    }
                }
            }
        }
        Ok(())
    }
}

fn operand_primitive(
    program: &MirProgram,
    types: &TypeInterner,
    operand: Operand,
) -> Option<PrimitiveType> {
    match operand {
        Operand::Copy(local_id) => {
            let local = program.local(local_id)?;
            match types.kind(local.ty) {
                TypeKind::Primitive(p) => Some(p),
                _ => None,
            }
        }
        Operand::Constant(Constant::Integer(_)) => Some(PrimitiveType::Int),
        Operand::Constant(Constant::Bool(_)) => Some(PrimitiveType::Bool),
        Operand::Constant(Constant::FloatBits(_)) => Some(PrimitiveType::Float),
        _ => None,
    }
}

fn simplify_instruction(
    program: &MirProgram,
    types: &TypeInterner,
    instruction_id: InstructionId,
) -> Option<InstructionKind> {
    let instruction = program.instruction(instruction_id)?;
    let InstructionKind::Assign { target, value } = instruction.kind else {
        return None;
    };
    let simplified = match value {
        Rvalue::Unary {
            operator: UnaryOperator::Plus,
            operand,
        } => {
            let prim = operand_primitive(program, types, operand)?;
            if matches!(prim, PrimitiveType::Int | PrimitiveType::Float) {
                Some(Rvalue::Use(operand))
            } else {
                None
            }
        }
        Rvalue::Binary {
            left,
            operator,
            right,
        } => {
            let lp = operand_primitive(program, types, left)?;
            let rp = operand_primitive(program, types, right)?;
            if lp != rp {
                return None;
            }
            match lp {
                PrimitiveType::Int => simplify_int_binary(left, operator, right),
                PrimitiveType::Bool => simplify_bool_binary(left, operator, right),
                _ => None,
            }
        }
        _ => None,
    }?;
    Some(InstructionKind::Assign {
        target,
        value: simplified,
    })
}

fn simplify_int_binary(left: Operand, operator: BinaryOperator, right: Operand) -> Option<Rvalue> {
    // 1. Self-operand simplifications: x op x
    if let (Operand::Copy(a), Operand::Copy(b)) = (left, right)
        && a == b
    {
        return match operator {
            BinaryOperator::Subtract | BinaryOperator::BitXor => {
                Some(Rvalue::Use(Operand::Constant(Constant::Integer(0))))
            }
            BinaryOperator::BitAnd | BinaryOperator::BitOr => Some(Rvalue::Use(Operand::Copy(a))),
            BinaryOperator::Equal | BinaryOperator::LessEqual | BinaryOperator::GreaterEqual => {
                Some(Rvalue::Use(Operand::Constant(Constant::Bool(true))))
            }
            BinaryOperator::NotEqual | BinaryOperator::Less | BinaryOperator::Greater => {
                Some(Rvalue::Use(Operand::Constant(Constant::Bool(false))))
            }
            _ => None,
        };
    }

    // 2. Right-constant simplifications: x op C
    if let (Operand::Copy(_), Operand::Constant(Constant::Integer(c))) = (left, right) {
        match (operator, c) {
            (BinaryOperator::Add | BinaryOperator::Subtract, 0)
            | (BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Power, 1)
            | (
                BinaryOperator::BitOr
                | BinaryOperator::BitXor
                | BinaryOperator::ShiftLeft
                | BinaryOperator::ShiftRight,
                0,
            )
            | (BinaryOperator::BitAnd, -1) => return Some(Rvalue::Use(left)),

            (BinaryOperator::Multiply | BinaryOperator::BitAnd, 0)
            | (BinaryOperator::Modulo, 1) => {
                return Some(Rvalue::Use(Operand::Constant(Constant::Integer(0))));
            }
            (BinaryOperator::Power, 0) => {
                return Some(Rvalue::Use(Operand::Constant(Constant::Integer(1))));
            }
            (BinaryOperator::Power, 2) => {
                return Some(Rvalue::Binary {
                    left,
                    operator: BinaryOperator::Multiply,
                    right: left,
                });
            }
            (BinaryOperator::BitOr, -1) => {
                return Some(Rvalue::Use(Operand::Constant(Constant::Integer(-1))));
            }
            (BinaryOperator::Multiply, -1) => {
                return Some(Rvalue::Unary {
                    operator: UnaryOperator::Negate,
                    operand: left,
                });
            }
            (BinaryOperator::BitXor, -1) => {
                return Some(Rvalue::Unary {
                    operator: UnaryOperator::BitNot,
                    operand: left,
                });
            }
            (BinaryOperator::Multiply, k)
                if k > 1 && (k as u64).is_power_of_two() && k.trailing_zeros() <= 62 =>
            {
                return Some(Rvalue::Binary {
                    left,
                    operator: BinaryOperator::ShiftLeft,
                    right: Operand::Constant(Constant::Integer(i64::from(k.trailing_zeros()))),
                });
            }
            _ => {}
        }
    }

    // 3. Left-constant simplifications for commutative ops: C op x
    if let (Operand::Constant(Constant::Integer(c)), Operand::Copy(_)) = (left, right) {
        match (operator, c) {
            (BinaryOperator::Add | BinaryOperator::BitOr | BinaryOperator::BitXor, 0)
            | (BinaryOperator::Multiply, 1)
            | (BinaryOperator::BitAnd, -1) => return Some(Rvalue::Use(right)),

            (BinaryOperator::Multiply | BinaryOperator::BitAnd, 0) => {
                return Some(Rvalue::Use(Operand::Constant(Constant::Integer(0))));
            }
            (BinaryOperator::BitOr, -1) => {
                return Some(Rvalue::Use(Operand::Constant(Constant::Integer(-1))));
            }
            (BinaryOperator::Multiply, -1) => {
                return Some(Rvalue::Unary {
                    operator: UnaryOperator::Negate,
                    operand: right,
                });
            }
            (BinaryOperator::BitXor, -1) => {
                return Some(Rvalue::Unary {
                    operator: UnaryOperator::BitNot,
                    operand: right,
                });
            }
            (BinaryOperator::Multiply, k)
                if k > 1 && (k as u64).is_power_of_two() && k.trailing_zeros() <= 62 =>
            {
                return Some(Rvalue::Binary {
                    left: right,
                    operator: BinaryOperator::ShiftLeft,
                    right: Operand::Constant(Constant::Integer(i64::from(k.trailing_zeros()))),
                });
            }
            _ => {}
        }
    }

    None
}

fn simplify_bool_binary(left: Operand, operator: BinaryOperator, right: Operand) -> Option<Rvalue> {
    if let (Operand::Copy(a), Operand::Copy(b)) = (left, right)
        && a == b
    {
        return match operator {
            BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr => {
                Some(Rvalue::Use(Operand::Copy(a)))
            }
            BinaryOperator::Equal => Some(Rvalue::Use(Operand::Constant(Constant::Bool(true)))),
            BinaryOperator::NotEqual => Some(Rvalue::Use(Operand::Constant(Constant::Bool(false)))),
            _ => None,
        };
    }

    let (var, c) = match (left, right) {
        (Operand::Copy(_), Operand::Constant(Constant::Bool(c))) => (left, c),
        (Operand::Constant(Constant::Bool(c)), Operand::Copy(_)) => (right, c),
        _ => return None,
    };

    match (operator, c) {
        (BinaryOperator::Equal | BinaryOperator::LogicalAnd, true)
        | (BinaryOperator::NotEqual | BinaryOperator::LogicalOr, false) => Some(Rvalue::Use(var)),
        (BinaryOperator::Equal, false) | (BinaryOperator::NotEqual, true) => Some(Rvalue::Unary {
            operator: UnaryOperator::Not,
            operand: var,
        }),
        (BinaryOperator::LogicalAnd, false) => {
            Some(Rvalue::Use(Operand::Constant(Constant::Bool(false))))
        }
        (BinaryOperator::LogicalOr, true) => {
            Some(Rvalue::Use(Operand::Constant(Constant::Bool(true))))
        }
        _ => None,
    }
}
