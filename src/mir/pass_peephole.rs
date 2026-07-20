//! Safety-preserving scalar MIR simplification for the C-Speed project.
//!
//! This pass only changes `Copy` scalar expressions. It never rewrites calls,
//! allocations, moves, borrows, ARC instructions, or terminators. Therefore it
//! cannot remove an ownership edge; ARC insertion still runs afterwards.

use std::collections::HashMap;
use crate::ast::BinaryOperator;
use crate::mir::ir::{LocalId, MirInstr, MirProgram, Operand, Ownership, Rvalue};

fn int_binary(op: &BinaryOperator, left: i64, right: i64) -> Option<Operand> {
    let value = match op {
        BinaryOperator::Add => Operand::Int(left.checked_add(right)?),
        BinaryOperator::Subtract => Operand::Int(left.checked_sub(right)?),
        BinaryOperator::Multiply => Operand::Int(left.checked_mul(right)?),
        BinaryOperator::Divide if right != 0 => Operand::Int(left.checked_div(right)?),
        BinaryOperator::Modulo if right != 0 => Operand::Int(left.checked_rem(right)?),
        BinaryOperator::Eq => Operand::Bool(left == right),
        BinaryOperator::NotEq => Operand::Bool(left != right),
        BinaryOperator::Less => Operand::Bool(left < right),
        BinaryOperator::Greater => Operand::Bool(left > right),
        BinaryOperator::LessEq => Operand::Bool(left <= right),
        BinaryOperator::GreaterEq => Operand::Bool(left >= right),
        _ => return None,
    };
    Some(value)
}

fn simplify(op: &BinaryOperator, left: &Operand, right: &Operand) -> Option<Operand> {
    if let (Operand::Int(a), Operand::Int(b)) = (left, right) {
        return int_binary(op, *a, *b);
    }
    // Do not apply float identities: NaN and signed-zero are observable.
    match (op, left, right) {
        (BinaryOperator::Add, value, Operand::Int(0))
        | (BinaryOperator::Subtract, value, Operand::Int(0))
        | (BinaryOperator::Multiply, value, Operand::Int(1))
        | (BinaryOperator::Divide, value, Operand::Int(1)) => Some(value.clone()),
        (BinaryOperator::Add, Operand::Int(0), value)
        | (BinaryOperator::Multiply, Operand::Int(1), value) => Some(value.clone()),
        (BinaryOperator::Multiply, _, Operand::Int(0))
        | (BinaryOperator::Multiply, Operand::Int(0), _) => Some(Operand::Int(0)),
        _ => None,
    }
}

fn scalar_constant(value: &Operand) -> bool {
    matches!(value, Operand::Int(_) | Operand::Bool(_))
}

fn substitute(operand: &Operand, constants: &HashMap<LocalId, Operand>) -> Operand {
    match operand {
        // Only plain local scalar reads participate. Borrowed reads are an
        // ownership contract and must remain visible to ARC analysis.
        Operand::Local(id) => constants.get(id).cloned().unwrap_or_else(|| operand.clone()),
        _ => operand.clone(),
    }
}

/// Run before ARC insertion. Propagation is intentionally block-local: a
/// branch/join can change a local's value, so no constant crosses a CFG edge.
pub fn run(program: &mut MirProgram) -> usize {
    let mut rewrites = 0;
    for function in program.functions.values_mut() {
        for block in &mut function.blocks {
            let mut constants: HashMap<LocalId, Operand> = HashMap::new();
            for instruction in &mut block.instrs {
                let replacement = match instruction {
                    MirInstr::Assign(destination, Rvalue::Use(value)) => {
                        Some((*destination, Rvalue::Use(substitute(value, &constants)), false))
                    }
                    MirInstr::Assign(destination, Rvalue::BinaryOp(op, left, right)) => {
                        let left = substitute(left, &constants);
                        let right = substitute(right, &constants);
                        match simplify(op, &left, &right) {
                            Some(value) => Some((*destination, Rvalue::Use(value), true)),
                            None => Some((*destination, Rvalue::BinaryOp(op.clone(), left, right), false)),
                        }
                    }
                    _ => None,
                };
                let Some((destination, rvalue, folded)) = replacement else { continue; };
                *instruction = MirInstr::Assign(destination, rvalue.clone());
                if folded { rewrites += 1; }
                // Constants are tracked only for copy locals. This prevents a
                // user-visible scalar simplification from ever treating an ARC
                // pointer, borrowed field, or container as a duplicable value.
                let copy_local = function.locals[destination.0].ownership == Ownership::Copy;
                match rvalue {
                    Rvalue::Use(value) if copy_local && scalar_constant(&value) => {
                        constants.insert(destination, value);
                    }
                    _ => { constants.remove(&destination); }
                }
            }
        }
    }
    rewrites
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folds_constants_and_keeps_divide_by_zero_unmodified() {
        assert!(matches!(simplify(&BinaryOperator::Multiply, &Operand::Int(9), &Operand::Int(0)), Some(Operand::Int(0))));
        assert!(matches!(simplify(&BinaryOperator::Less, &Operand::Int(2), &Operand::Int(3)), Some(Operand::Bool(true))));
        assert!(simplify(&BinaryOperator::Divide, &Operand::Int(9), &Operand::Int(0)).is_none());
    }
}
