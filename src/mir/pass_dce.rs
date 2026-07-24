//! Conservative scalar dead-store elimination for C-Speed compilation latency.
//!
//! Only single-block functions are considered. That makes every read visible in
//! one linear instruction stream and avoids CFG/phi/liveness assumptions. The
//! pass never removes calls, allocations, ARC, moves, borrows, division, or
//! modulo because those may have observable effects or traps.

use crate::ast::BinaryOperator;
use crate::mir::ir::{LocalId, MirInstr, MirProgram, Operand, Ownership, Rvalue, Terminator};
use std::collections::HashSet;

fn add_operand_use(live: &mut HashSet<LocalId>, operand: &Operand) {
    if let Operand::Local(id) | Operand::Borrowed(id) = operand {
        live.insert(*id);
    }
}

fn pure_nontrapping(rvalue: &Rvalue) -> bool {
    matches!(
        rvalue,
        Rvalue::Use(_)
            | Rvalue::BinaryOp(
                BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEq
                    | BinaryOperator::GreaterEq,
                _,
                _
            )
    )
}

fn add_rvalue_uses(live: &mut HashSet<LocalId>, rvalue: &Rvalue) {
    match rvalue {
        Rvalue::Use(value) => add_operand_use(live, value),
        Rvalue::BinaryOp(_, left, right) => {
            add_operand_use(live, left);
            add_operand_use(live, right);
        }
        // Calls and closures are not removable, but their scalar arguments are
        // observable inputs and therefore keep their defining assignments live.
        Rvalue::CallDirect(_, args)
        | Rvalue::CallIndirect(_, args)
        | Rvalue::BuiltinCall(_, args)
        | Rvalue::MakeClosure(_, args) => {
            for value in args {
                add_operand_use(live, value);
            }
        }
        Rvalue::FieldAccess(base, _) => add_operand_use(live, base),
        Rvalue::Move(local) => {
            live.insert(*local);
        }
        Rvalue::SpawnThread(closure) => add_operand_use(live, closure),
        Rvalue::AllocateStruct(_) | Rvalue::AllocateArcStruct(_) | Rvalue::AllocateList(_) | Rvalue::FuncRef(_) => {}
    }
}

/// Remove dead assignments from straight-line Copy-only scalar code.
pub fn run(program: &mut MirProgram) -> usize {
    let mut removed = 0;
    for function in program.functions.values_mut() {
        if function.blocks.len() != 1 {
            continue;
        }
        let block = &mut function.blocks[0];
        let mut live = HashSet::new();
        match &block.terminator {
            Terminator::Return(Some(value)) | Terminator::ReturnOwned(value) => {
                add_operand_use(&mut live, value)
            }
            Terminator::If { cond, .. } => add_operand_use(&mut live, cond),
            Terminator::IfCmp { left, right, .. } => {
                add_operand_use(&mut live, left);
                add_operand_use(&mut live, right);
            }
            _ => {}
        }
        let mut kept = Vec::with_capacity(block.instrs.len());
        for instruction in block.instrs.drain(..).rev() {
            match &instruction {
                MirInstr::Assign(destination, rvalue)
                    if function.locals[destination.0].ownership == Ownership::Copy
                        && pure_nontrapping(rvalue)
                        && !live.contains(destination) =>
                {
                    removed += 1;
                }
                MirInstr::Assign(destination, rvalue) => {
                    live.remove(destination);
                    add_rvalue_uses(&mut live, rvalue);
                    kept.push(instruction);
                }
                MirInstr::AssignField { base, value, .. } => {
                    live.insert(*base);
                    add_operand_use(&mut live, value);
                    kept.push(instruction);
                }
                MirInstr::Retain(local) | MirInstr::Release(local) => {
                    live.insert(*local);
                    kept.push(instruction);
                }
            }
        }
        kept.reverse();
        block.instrs = kept;
    }
    removed
}
