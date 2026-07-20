//! MIR constant propagation pass.
//!
//! Scans each basic block for known integer constants and folds arithmetic
//! at compile time.  Runs after peephole and before ARC insertion.

use crate::ast::BinaryOperator;
use crate::mir::ir::*;
use std::collections::HashMap;

pub fn run(program: &mut MirProgram) {
    for function in program.functions.values_mut() {
        for block in &mut function.blocks {
            let mut known: HashMap<LocalId, i64> = HashMap::new();

            for instruction in &mut block.instrs {
                if let MirInstr::Assign(dest, rvalue) = instruction {
                    match rvalue {
                        Rvalue::Use(Operand::Int(v)) => {
                            known.insert(*dest, *v);
                        }
                        Rvalue::Use(Operand::Local(src)) => {
                            if let Some(&val) = known.get(src) {
                                *rvalue = Rvalue::Use(Operand::Int(val));
                                known.insert(*dest, val);
                            } else {
                                known.remove(dest);
                            }
                        }
                        Rvalue::BinaryOp(op, left, right) => {
                            let (l_known, r_known) = match (&*left, &*right) {
                                (Operand::Local(l), Operand::Local(r)) => (known.get(l), known.get(r)),
                                (Operand::Int(l), Operand::Local(r)) => (Some(l), known.get(r)),
                                (Operand::Local(l), Operand::Int(r)) => (known.get(l), Some(r)),
                                (Operand::Int(l), Operand::Int(r)) => (Some(l), Some(r)),
                                _ => (None, None),
                            };
                            if let (Some(&l), Some(&r)) = (l_known, r_known) {
                                let op_clone = op.clone();
                                if let Some(result) = fold_int(op_clone, l, r) {
                                    *rvalue = Rvalue::Use(Operand::Int(result));
                                    known.insert(*dest, result);
                                    continue;
                                }
                            }
                            // Propagate constants into operands
                            if let Operand::Local(l) = &*left {
                                if let Some(&v) = known.get(l) { *left = Operand::Int(v); }
                            }
                            if let Operand::Local(r) = &*right {
                                if let Some(&v) = known.get(r) { *right = Operand::Int(v); }
                            }
                            known.remove(dest);
                        }
                        _ => {
                            known.remove(dest);
                        }
                    }
                }
            }
        }
    }
}

fn fold_int(op: BinaryOperator, a: i64, b: i64) -> Option<i64> {
    match op {
        BinaryOperator::Add => Some(a.wrapping_add(b)),
        BinaryOperator::Subtract => Some(a.wrapping_sub(b)),
        BinaryOperator::Multiply => Some(a.wrapping_mul(b)),
        BinaryOperator::Divide if b != 0 => Some(a.wrapping_div(b)),
        BinaryOperator::Modulo if b != 0 => Some(a.wrapping_rem(b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_add_of_constants() {
        let mut program = MirProgram {
            functions: {
                let mut f = std::collections::HashMap::new();
                f.insert(FuncId(0), MirFunction {
                    id: FuncId(0), name: "test".into(), params: vec![],
                    locals: (0..3).map(|i| LocalDecl { id: LocalId(i), ty: crate::typecheck::TypeRef::Int, is_mut: false, debug_name: None, binding_id: None, ownership: Ownership::Copy }).collect(),
                    blocks: vec![MirBlock { id: BlockId(0), instrs: vec![
                        MirInstr::Assign(LocalId(0), Rvalue::Use(Operand::Int(3))),
                        MirInstr::Assign(LocalId(1), Rvalue::Use(Operand::Int(4))),
                        MirInstr::Assign(LocalId(2), Rvalue::BinaryOp(BinaryOperator::Add, Operand::Local(LocalId(0)), Operand::Local(LocalId(1)))),
                    ], terminator: Terminator::Return(None) }],
                    start_block: BlockId(0), return_type: crate::typecheck::TypeRef::Void,
                });
                f
            },
        };
        run(&mut program);
        let last = &program.functions[&FuncId(0)].blocks[0].instrs[2];
        assert!(matches!(last, MirInstr::Assign(LocalId(2), Rvalue::Use(Operand::Int(7)))));
    }

    #[test]
    fn propagates_through_chain() {
        let mut program = MirProgram {
            functions: {
                let mut f = std::collections::HashMap::new();
                f.insert(FuncId(0), MirFunction {
                    id: FuncId(0), name: "chain".into(), params: vec![],
                    locals: (0..4).map(|i| LocalDecl { id: LocalId(i), ty: crate::typecheck::TypeRef::Int, is_mut: false, debug_name: None, binding_id: None, ownership: Ownership::Copy }).collect(),
                    blocks: vec![MirBlock { id: BlockId(0), instrs: vec![
                        MirInstr::Assign(LocalId(0), Rvalue::Use(Operand::Int(10))),
                        MirInstr::Assign(LocalId(1), Rvalue::Use(Operand::Local(LocalId(0)))),
                        MirInstr::Assign(LocalId(2), Rvalue::Use(Operand::Local(LocalId(1)))),
                        MirInstr::Assign(LocalId(3), Rvalue::BinaryOp(BinaryOperator::Add, Operand::Local(LocalId(2)), Operand::Int(5))),
                    ], terminator: Terminator::Return(None) }],
                    start_block: BlockId(0), return_type: crate::typecheck::TypeRef::Void,
                });
                f
            },
        };
        run(&mut program);
        let last = &program.functions[&FuncId(0)].blocks[0].instrs[3];
        assert!(matches!(last, MirInstr::Assign(LocalId(3), Rvalue::Use(Operand::Int(15)))));
    }
}
