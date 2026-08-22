/// Copy propagation pass: eliminates redundant temporary assignments.
///
/// Pattern: `_tmp = expr; _dest = _tmp;`  →  `_dest = expr;`
///
/// This removes one register move per assignment, which matters for tight
/// loops where Cranelift cannot do this itself.
///
/// # Correctness
///
/// The fold rewrites the *definition* of `_tmp` so that it writes `_dest`
/// instead, which means `_tmp` is never assigned at all. That is only legal
/// when nothing else in the function observes `_tmp`.
///
/// Folding unconditionally miscompiles the very common shape
///
/// ```text
///     first := g()          _0 = call g()     ; _0 is `first`
///     mut cur := first      _1 = _0           ; folded: `_0 = call g()` becomes
///     cur = 99              _1 = 99           ;   `_1 = call g()`, so _0 is dead
///     return first          return _0         ; _0 never written → returns 0
/// ```
///
/// so `mut y := x` silently returned garbage whenever `x` was an immutable
/// local initialised from a call. We therefore only fold when the source local
/// is mentioned exactly twice in the whole function: once by the definition we
/// are rewriting and once by the copy we are deleting.
use crate::mir::ir::*;
use std::collections::HashSet;

pub fn run(program: &mut MirProgram) {
    for function in program.functions.values_mut() {
        // Locals that survive beyond the copy must not be folded away.
        let pinned = locals_mentioned_more_than_twice(function);

        for block in &mut function.blocks {
            let instrs = &mut block.instrs;
            let mut i = 0;
            while i + 1 < instrs.len() {
                let fold = match (&instrs[i], &instrs[i + 1]) {
                    // Pattern: _tmp = expr; _dest = _tmp  →  _dest = expr
                    (
                        MirInstr::Assign(tmp, rvalue),
                        MirInstr::Assign(dest, Rvalue::Use(Operand::Local(src))),
                    ) if *src == *tmp && *tmp != *dest && !pinned.contains(tmp) => {
                        Some((*dest, rvalue.clone()))
                    }
                    // Pattern: _tmp = expr; _dest = move(_tmp)  →  _dest = expr
                    (MirInstr::Assign(tmp, rvalue), MirInstr::Assign(dest, Rvalue::Move(src)))
                        if *src == *tmp && *tmp != *dest && !pinned.contains(tmp) =>
                    {
                        Some((*dest, rvalue.clone()))
                    }
                    _ => None,
                };

                if let Some((dest, rvalue)) = fold {
                    instrs[i] = MirInstr::Assign(dest, rvalue);
                    instrs.remove(i + 1);
                } else {
                    i += 1;
                }
            }
        }
    }
}

/// Locals mentioned more than twice anywhere in the function.
///
/// A genuinely foldable temporary is written once (its definition) and read
/// once (the copy being removed) — exactly two mentions. Anything mentioned a
/// third time is read later, reassigned, or live across blocks, and removing
/// its definition would be observable.
fn locals_mentioned_more_than_twice(function: &MirFunction) -> HashSet<LocalId> {
    let mut counts = vec![0u32; function.locals.len()];

    for block in &function.blocks {
        for instruction in &block.instrs {
            count_instr(instruction, &mut counts);
        }
        count_terminator(&block.terminator, &mut counts);
    }

    counts
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 2)
        .map(|(index, _)| LocalId(index))
        .collect()
}

fn bump(local: &LocalId, counts: &mut [u32]) {
    if let Some(slot) = counts.get_mut(local.0) {
        *slot = slot.saturating_add(1);
    }
}

fn count_operand(operand: &Operand, counts: &mut [u32]) {
    match operand {
        Operand::Local(local) | Operand::Borrowed(local) => bump(local, counts),
        _ => {}
    }
}

fn count_operands(operands: &[Operand], counts: &mut [u32]) {
    for operand in operands {
        count_operand(operand, counts);
    }
}

fn count_rvalue(rvalue: &Rvalue, counts: &mut [u32]) {
    match rvalue {
        Rvalue::AllocateTuple(_, values) | Rvalue::MakeTask(_, _, values, _) => {
            count_operands(values, counts)
        }
        Rvalue::TupleField(base, _)
        | Rvalue::SliceLen(base)
        | Rvalue::SliceToStr(base)
        | Rvalue::Await(base) => count_operand(base, counts),
        Rvalue::MakeSlice {
            base,
            start,
            length,
            ..
        } => {
            count_operand(base, counts);
            count_operand(start, counts);
            count_operand(length, counts);
        }
        Rvalue::SliceGet(view, index) => {
            count_operand(view, counts);
            count_operand(index, counts);
        }
        Rvalue::Use(operand) => count_operand(operand, counts),
        Rvalue::Move(local) => bump(local, counts),
        Rvalue::BinaryOp(_, left, right) => {
            count_operand(left, counts);
            count_operand(right, counts);
        }
        Rvalue::CallDirect(_, args) => count_operands(args, counts),
        Rvalue::CallIndirect(callee, args) => {
            count_operand(callee, counts);
            count_operands(args, counts);
        }
        Rvalue::BuiltinCall(_, args) => count_operands(args, counts),
        Rvalue::MakeClosure(_, captures) | Rvalue::MakeStackClosure(_, captures) => {
            count_operands(captures, counts)
        }
        Rvalue::FieldAccess(base, _) => count_operand(base, counts),
        Rvalue::SpawnThread(closure) => count_operand(closure, counts),
        Rvalue::AllocateStruct(_)
        | Rvalue::AllocateArcStruct(_)
        | Rvalue::AllocateStackStruct(_)
        | Rvalue::AllocateList(_)
        | Rvalue::FuncRef(_) => {}
        Rvalue::AllocateArenaStruct(_, arena) => count_operand(arena, counts),
    }
}

fn count_instr(instruction: &MirInstr, counts: &mut [u32]) {
    match instruction {
        MirInstr::Assign(dest, rvalue) => {
            bump(dest, counts);
            count_rvalue(rvalue, counts);
        }
        MirInstr::AssignField { base, value, .. } => {
            bump(base, counts);
            count_operand(value, counts);
        }
        MirInstr::Retain(local) | MirInstr::Release(local) => bump(local, counts),
    }
}

fn count_terminator(terminator: &Terminator, counts: &mut [u32]) {
    match terminator {
        Terminator::Return(Some(operand)) => count_operand(operand, counts),
        Terminator::ReturnOwned(operand) => count_operand(operand, counts),
        Terminator::If { cond, .. } => count_operand(cond, counts),
        Terminator::IfCmp { left, right, .. } => {
            count_operand(left, counts);
            count_operand(right, counts);
        }
        Terminator::Return(None) | Terminator::Goto(_) | Terminator::Unreachable => {}
    }
}
