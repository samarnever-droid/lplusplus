/// Strength reduction pass: replaces expensive operations with cheaper equivalents.
///
/// Currently handles:
/// - `x % power_of_2` → `x & (power_of_2 - 1)` (bitwise AND)
///
/// Cranelift's `idiv` is ~40 cycles on x86; this pass avoids it for the
/// common case of modulo by a power of two.
use crate::ast::BinaryOperator;
use crate::mir::ir::*;

pub fn run(program: &mut MirProgram) {
    for function in program.functions.values_mut() {
        for block in &mut function.blocks {
            for instr in &mut block.instrs {
                if let MirInstr::Assign(dest, Rvalue::BinaryOp(BinaryOperator::Modulo, left, right)) = instr {
                    // x % const where const is a power of 2 → x & (const - 1)
                    if let Operand::Int(val) = right {
                        if *val > 0 && (*val & (*val - 1)) == 0 {
                            *instr = MirInstr::Assign(
                                *dest,
                                Rvalue::BinaryOp(
                                    BinaryOperator::BitAnd,
                                    left.clone(),
                                    Operand::Int(*val - 1),
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
}
