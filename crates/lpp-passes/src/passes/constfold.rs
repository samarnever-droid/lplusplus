//! `const-fold` — scalar pass.
//!
//! Folds an rvalue whose operands are all `Operand::Constant` into
//! `Use(Constant(result))`, using the exact semantics the interpreter
//! applies. The rewrite is in place on a single instruction: the
//! target local, its type, and every other instruction are untouched,
//! and the result is always a primitive constant.
//!
//! Required: the core invariants. Preserved: the core invariants plus
//! the ownership trio — the rewrite removes constant reads only,
//! introduces no references, and never produces a heap-typed result
//! (string concatenation is deliberately not folded).

use lpp_mir::{
    BinaryOperator, Constant, InstructionId, InstructionKind, MirInvariant, MirProgram, Operand,
    Rvalue, UnaryOperator,
};

use crate::{MirPass, PassContext, PassFailure};

/// The invariants an in-place 4F pass preserves. The core invariants
/// are revalidated by the manager after every pass; the ownership trio
/// is preserved by construction because the passes remove reads of
/// primitive constants only.
pub(crate) const IN_PLACE_PRESERVED: &[MirInvariant] = &[
    MirInvariant::References,
    MirInvariant::ControlFlow,
    MirInvariant::Types,
    MirInvariant::DefiniteInitialization,
    MirInvariant::Ownership,
    MirInvariant::NoOwningCycles,
    MirInvariant::OwnershipBalance,
];

/// `const-fold`: fold pure-constant rvalues with oracle-exact
/// semantics.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConstFoldPass;

impl MirPass for ConstFoldPass {
    fn name(&self) -> &'static str {
        "const-fold"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        IN_PLACE_PRESERVED
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        _context: &PassContext<'_>,
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
                    let Some(replacement) = fold_instruction(program, instruction_id) else {
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

/// Fold one instruction's rvalue when the whole rvalue is constant;
/// returns the replacement kind when a fold happened.
fn fold_instruction(
    program: &MirProgram,
    instruction_id: InstructionId,
) -> Option<InstructionKind> {
    let instruction = program.instruction(instruction_id)?;
    let InstructionKind::Assign { target, value } = &instruction.kind else {
        return None;
    };
    let folded = match value {
        Rvalue::Unary {
            operator,
            operand: Operand::Constant(value),
        } => fold_unary(*operator, value),
        Rvalue::Binary {
            left: Operand::Constant(left),
            operator,
            right: Operand::Constant(right),
        } => fold_binary(program, *operator, left, right),
        _ => None,
    }?;
    Some(InstructionKind::Assign {
        target: *target,
        value: Rvalue::Use(Operand::Constant(folded)),
    })
}

/// Fold a constant unary operation with the exact oracle semantics.
/// `None` means the oracle would reject or compute the operation at
/// runtime; the rvalue is kept as-is so behavior is preserved.
#[must_use]
pub fn fold_unary(operator: UnaryOperator, value: &Constant) -> Option<Constant> {
    match (operator, value) {
        (UnaryOperator::Not, Constant::Bool(value)) => Some(Constant::Bool(!value)),
        (UnaryOperator::Negate, Constant::Integer(value)) => {
            Some(Constant::Integer(value.wrapping_neg()))
        }
        (UnaryOperator::Negate, Constant::FloatBits(bits)) => {
            Some(Constant::FloatBits((-f64::from_bits(*bits)).to_bits()))
        }
        _ => None,
    }
}

/// Fold a constant binary operation with the exact oracle semantics.
/// `None` means the oracle would reject or compute the operation at
/// runtime (division/modulo by zero, logical operators on non-bools,
/// string concatenation, mixed-type pairs); the rvalue is kept as-is.
#[must_use]
pub fn fold_binary(
    program: &MirProgram,
    operator: BinaryOperator,
    left: &Constant,
    right: &Constant,
) -> Option<Constant> {
    match (left, right) {
        (Constant::Integer(left), Constant::Integer(right)) => {
            fold_integer(operator, *left, *right)
        }
        (Constant::FloatBits(left), Constant::FloatBits(right)) => {
            fold_float(operator, f64::from_bits(*left), f64::from_bits(*right))
        }
        (Constant::Bool(left), Constant::Bool(right)) => match operator {
            BinaryOperator::LogicalAnd => Some(Constant::Bool(*left && *right)),
            BinaryOperator::LogicalOr => Some(Constant::Bool(*left || *right)),
            BinaryOperator::Equal => Some(Constant::Bool(*left == *right)),
            BinaryOperator::NotEqual => Some(Constant::Bool(*left != *right)),
            _ => None,
        },
        (
            Constant::Character {
                character: left, ..
            },
            Constant::Character {
                character: right, ..
            },
        ) => fold_char(operator, *left, *right),
        (Constant::String { string: left, .. }, Constant::String { string: right, .. }) => {
            let (left, right) = (program.string(*left)?, program.string(*right)?);
            // `Add` on strings allocates a heap node at runtime: never
            // folded (it would change ownership and ARC traffic).
            match operator {
                BinaryOperator::Equal => Some(Constant::Bool(left == right)),
                BinaryOperator::NotEqual => Some(Constant::Bool(left != right)),
                _ => None,
            }
        }
        // Mixed-type pairs are ruled out by the type checker in
        // validated MIR; if one ever appears, runtime behavior is
        // preserved as-is.
        _ => None,
    }
}

fn fold_integer(operator: BinaryOperator, left: i64, right: i64) -> Option<Constant> {
    Some(match operator {
        BinaryOperator::Add => Constant::Integer(left.wrapping_add(right)),
        BinaryOperator::Subtract => Constant::Integer(left.wrapping_sub(right)),
        BinaryOperator::Multiply => Constant::Integer(left.wrapping_mul(right)),
        BinaryOperator::Divide if right != 0 => Constant::Integer(left.wrapping_div(right)),
        BinaryOperator::Modulo if right != 0 => Constant::Integer(left.wrapping_rem(right)),
        BinaryOperator::BitAnd => Constant::Integer(left & right),
        BinaryOperator::BitOr => Constant::Integer(left | right),
        BinaryOperator::BitXor => Constant::Integer(left ^ right),
        BinaryOperator::ShiftLeft => Constant::Integer(left.wrapping_shl(right as u32)),
        BinaryOperator::ShiftRight => Constant::Integer(left.wrapping_shr(right as u32)),
        BinaryOperator::Equal => Constant::Bool(left == right),
        BinaryOperator::NotEqual => Constant::Bool(left != right),
        BinaryOperator::Less => Constant::Bool(left < right),
        BinaryOperator::Greater => Constant::Bool(left > right),
        BinaryOperator::LessEqual => Constant::Bool(left <= right),
        BinaryOperator::GreaterEqual => Constant::Bool(left >= right),
        _ => return None,
    })
}

fn fold_float(operator: BinaryOperator, left: f64, right: f64) -> Option<Constant> {
    // The oracle applies the same IEEE-754 `f64` operations, so the
    // fold is bit-for-bit — inf and NaN included.
    Some(match operator {
        BinaryOperator::Add => Constant::FloatBits((left + right).to_bits()),
        BinaryOperator::Subtract => Constant::FloatBits((left - right).to_bits()),
        BinaryOperator::Multiply => Constant::FloatBits((left * right).to_bits()),
        BinaryOperator::Divide => Constant::FloatBits((left / right).to_bits()),
        BinaryOperator::Modulo => Constant::FloatBits((left % right).to_bits()),
        BinaryOperator::Equal => Constant::Bool(left == right),
        BinaryOperator::NotEqual => Constant::Bool(left != right),
        BinaryOperator::Less => Constant::Bool(left < right),
        BinaryOperator::Greater => Constant::Bool(left > right),
        BinaryOperator::LessEqual => Constant::Bool(left <= right),
        BinaryOperator::GreaterEqual => Constant::Bool(left >= right),
        _ => return None,
    })
}

fn fold_char(operator: BinaryOperator, left: char, right: char) -> Option<Constant> {
    Some(match operator {
        BinaryOperator::Equal => Constant::Bool(left == right),
        BinaryOperator::NotEqual => Constant::Bool(left != right),
        BinaryOperator::Less => Constant::Bool(left < right),
        BinaryOperator::Greater => Constant::Bool(left > right),
        BinaryOperator::LessEqual => Constant::Bool(left <= right),
        BinaryOperator::GreaterEqual => Constant::Bool(left >= right),
        _ => return None,
    })
}
