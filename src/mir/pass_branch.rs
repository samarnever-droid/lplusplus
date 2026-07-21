//! Fuse comparison temporaries into direct MIR branch terminators.
use crate::ast::BinaryOperator;
use crate::mir::ir::*;

fn comparison(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Less
            | BinaryOperator::Greater
            | BinaryOperator::LessEq
            | BinaryOperator::GreaterEq
    )
}

/// Removes only a final Copy Bool assignment whose sole consumer is the block
/// terminator. No ownership instruction or user value escapes this rewrite.
pub fn run(program: &mut MirProgram) -> usize {
    let mut fused = 0;
    for function in program.functions.values_mut() {
        for block in &mut function.blocks {
            let Terminator::If {
                cond: Operand::Local(condition),
                then_block,
                else_block,
            } = block.terminator.clone()
            else {
                continue;
            };
            let Some(MirInstr::Assign(destination, Rvalue::BinaryOp(op, left, right))) =
                block.instrs.last().cloned()
            else {
                continue;
            };
            if destination != condition
                || !comparison(&op)
                || function.locals[destination.0].ownership != Ownership::Copy
            {
                continue;
            }
            block.instrs.pop();
            block.terminator = Terminator::IfCmp {
                op,
                left,
                right,
                then_block,
                else_block,
            };
            fused += 1;
        }
    }
    fused
}
