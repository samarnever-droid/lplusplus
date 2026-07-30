//! Move-out: distinguish *transferring* a value to a thread from *sharing* it.
//!
//! # The distinction
//!
//! Crossing a concurrency boundary is not the same thing as sharing. When a
//! value is captured by a spawned closure and the spawning thread never touches
//! it again, ownership simply *moves* — there is one owner before the spawn and
//! one after, never two at once. Nothing needs to be counted.
//!
//! Only when both sides hold a live reference at the same time is a refcount
//! actually earning its keep. Rust draws this line with move semantics and
//! `Send`; L++ can draw it by proving the source binding is dead.
//!
//! # Where this has to live
//!
//! An earlier AST classification attempted to answer this question, but it
//! never reached ARC code generation and was removed when the MIR solver became
//! the single ownership source of truth.
//!
//! ARC is decided entirely from MIR `Ownership`, so the proof has to be done on
//! MIR, which is also where it is easiest: the capture, the spawn and the later
//! uses are all explicit instructions in one function body.
//!
//! # What is proven
//!
//! For a local captured into a closure environment that is then handed to
//! `spawn_thread`, the pattern in MIR is:
//!
//! ```text
//!     _env.cap_0 = borrow(_0)
//!     retain(_0)                  <-- second owner created for the thread
//!     _c = make_closure(f, [_env])
//!     _  = spawn_thread(_c)
//!     ... does anything read _0 after this point? ...
//!     release(_0)
//! ```
//!
//! If nothing reads `_0` after the spawn, the `retain`/`release` pair is pure
//! overhead: the reference is created only to be destroyed, and on the atomic
//! path each half is a locked read-modify-write on a contended line. Eliding
//! both leaves exactly one owner — the thread — which is what a move means.
//!
//! # Conservatism
//!
//! This is a proof of absence, so every uncertainty keeps the retain:
//!
//!   * any read of the local after the spawn, anywhere reachable, is a share;
//!   * a spawn inside a loop is never eligible, because a later iteration can
//!     re-read the binding and "dead after this point" stops being meaningful
//!     (the same objection applies to any back edge, so cycles are excluded
//!     wholesale rather than reasoned about);
//!   * the local must be released exactly once on the paths that follow, so
//!     the pair being removed is genuinely balanced;
//!   * anything reachable by a branch counts as a use, including branches that
//!     rejoin after the spawn.
//!
//! # Use-after-move
//!
//! There is no soundness hole to guard here, precisely *because* the analysis
//! is a liveness proof rather than a declaration. A binding is only treated as
//! moved when no later read exists; if a later read exists the value stays
//! shared and refcounted. That is the opposite ordering from Rust, which
//! declares the move and then rejects later uses. It means L++ needs no
//! "use of moved value" diagnostic for this feature — the case that would
//! trigger one is simply classified as a share instead.

use super::ir::*;
use std::collections::HashSet;

/// Locals whose `retain`/`release` pair around a `spawn_thread` can be elided.
struct MoveOut {
    /// Block and instruction index of the `retain` to drop.
    retain_sites: Vec<(usize, usize)>,
    /// Locals that must also lose their trailing `release`.
    moved_locals: HashSet<LocalId>,
}

fn successors_of(terminator: &Terminator) -> Vec<usize> {
    match terminator {
        Terminator::Goto(target) => vec![target.0],
        Terminator::If { then_block, else_block, .. }
        | Terminator::IfCmp { then_block, else_block, .. } => vec![then_block.0, else_block.0],
        Terminator::Return(_) | Terminator::ReturnOwned(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// True when the control-flow graph contains a cycle reachable from `start`.
///
/// A spawn inside a loop is never eligible for move-out: an earlier iteration's
/// "later use" is a use of the same lexical binding, so deadness after a single
/// textual point says nothing useful.
fn has_cycle(function: &MirFunction) -> bool {
    let n = function.blocks.len();
    let mut state = vec![0u8; n]; // 0 = unvisited, 1 = on stack, 2 = done
    let mut stack = vec![(function.start_block.0, 0usize)];
    if function.start_block.0 >= n {
        return true; // malformed; refuse to optimise
    }
    state[function.start_block.0] = 1;
    while let Some((block, next_succ)) = stack.pop() {
        let succs = successors_of(&function.blocks[block].terminator);
        if next_succ < succs.len() {
            stack.push((block, next_succ + 1));
            let s = succs[next_succ];
            if s >= n {
                return true;
            }
            match state[s] {
                1 => return true, // back edge
                0 => {
                    state[s] = 1;
                    stack.push((s, 0));
                }
                _ => {}
            }
        } else {
            state[block] = 2;
        }
    }
    false
}

/// Does any instruction read `local`, ignoring pure ARC bookkeeping?
fn reads_local(instr: &MirInstr, local: LocalId) -> bool {
    fn in_operand(op: &Operand, local: LocalId) -> bool {
        matches!(op, Operand::Local(id) | Operand::Borrowed(id) if *id == local)
    }
    fn in_rvalue(rv: &Rvalue, local: LocalId) -> bool {
        match rv {
            Rvalue::Use(op) => in_operand(op, local),
            Rvalue::Move(id) => *id == local,
            Rvalue::BinaryOp(_, a, b) => in_operand(a, local) || in_operand(b, local),
            Rvalue::CallDirect(_, args) | Rvalue::BuiltinCall(_, args) => {
                args.iter().any(|a| in_operand(a, local))
            }
            Rvalue::CallIndirect(callee, args) => {
                in_operand(callee, local) || args.iter().any(|a| in_operand(a, local))
            }
            Rvalue::MakeClosure(_, caps) | Rvalue::MakeStackClosure(_, caps) => {
                caps.iter().any(|c| in_operand(c, local))
            }
            Rvalue::FieldAccess(base, _) => in_operand(base, local),
            Rvalue::SpawnThread(op) => in_operand(op, local),
            _ => false,
        }
    }
    match instr {
        MirInstr::Assign(_, rv) => in_rvalue(rv, local),
        MirInstr::AssignField { base, value, .. } => {
            *base == local || in_operand(value, local)
        }
        // Retain/Release are bookkeeping, not a use of the value.
        MirInstr::Retain(_) | MirInstr::Release(_) => false,
    }
}

/// Does a block terminator read `local`? `IfCmp` compares two operands and
/// `Return`/`ReturnOwned` can hand one back, so all are real uses.
fn terminator_reads(t: &Terminator, local: LocalId) -> bool {
    let is = |op: &Operand| matches!(op, Operand::Local(id) | Operand::Borrowed(id) if *id == local);
    match t {
        Terminator::If { cond, .. } => is(cond),
        Terminator::IfCmp { left, right, .. } => is(left) || is(right),
        Terminator::Return(Some(op)) | Terminator::ReturnOwned(op) => is(op),
        _ => false,
    }
}

/// True when `local` is read anywhere reachable after (block, idx).
fn used_after(function: &MirFunction, block: usize, idx: usize, local: LocalId) -> bool {
    for instr in function.blocks[block].instrs.iter().skip(idx + 1) {
        if reads_local(instr, local) {
            return true;
        }
    }
    if terminator_reads(&function.blocks[block].terminator, local) {
        return true;
    }

    let mut seen: HashSet<usize> = HashSet::new();
    let mut work: Vec<usize> = successors_of(&function.blocks[block].terminator);
    while let Some(b) = work.pop() {
        if b >= function.blocks.len() || !seen.insert(b) {
            continue;
        }
        for instr in &function.blocks[b].instrs {
            if reads_local(instr, local) {
                return true;
            }
        }
        if terminator_reads(&function.blocks[b].terminator, local) {
            return true;
        }
        work.extend(successors_of(&function.blocks[b].terminator));
    }
    false
}

/// Find retain/release pairs that exist only to hand a value to a thread.
fn find_move_outs(function: &MirFunction) -> MoveOut {
    let mut result = MoveOut {
        retain_sites: Vec::new(),
        moved_locals: HashSet::new(),
    };
    if has_cycle(function) {
        return result;
    }

    for (bi, block) in function.blocks.iter().enumerate() {
        for (ii, instr) in block.instrs.iter().enumerate() {
            let retained = match instr {
                MirInstr::Retain(local) => *local,
                _ => continue,
            };

            // The retain must be followed, in this block, by a spawn_thread.
            // Anything that reads the retained local in between means the
            // value is genuinely live on this side of the boundary.
            let mut saw_spawn = false;
            let mut interfering_use = false;
            for later in block.instrs.iter().skip(ii + 1) {
                if let MirInstr::Assign(_, Rvalue::SpawnThread(_)) = later {
                    saw_spawn = true;
                    break;
                }
                if reads_local(later, retained) {
                    // Capturing into the closure env is the transfer itself,
                    // not a competing use.
                    let is_capture = matches!(
                        later,
                        MirInstr::AssignField { value: Operand::Borrowed(id), .. }
                            | MirInstr::AssignField { value: Operand::Local(id), .. }
                            if *id == retained
                    )
                        || matches!(
                            later,
                            MirInstr::Assign(
                                _,
                                Rvalue::MakeClosure(_, _) | Rvalue::MakeStackClosure(_, _)
                            )
                        );
                    if !is_capture {
                        interfering_use = true;
                        break;
                    }
                }
            }
            if !saw_spawn || interfering_use {
                continue;
            }

            // Locate the spawn and prove nothing reads the local afterwards.
            let spawn_idx = block
                .instrs
                .iter()
                .enumerate()
                .skip(ii + 1)
                .find(|(_, i)| matches!(i, MirInstr::Assign(_, Rvalue::SpawnThread(_))))
                .map(|(k, _)| k);
            let Some(spawn_idx) = spawn_idx else { continue };

            if used_after(function, bi, spawn_idx, retained) {
                continue; // genuine sharing
            }

            // The trailing release must exist exactly once, so the pair is
            // balanced and removing both is neutral.
            let releases = function
                .blocks
                .iter()
                .flat_map(|b| b.instrs.iter())
                .filter(|i| matches!(i, MirInstr::Release(l) if *l == retained))
                .count();
            if releases != 1 {
                continue;
            }

            result.retain_sites.push((bi, ii));
            result.moved_locals.insert(retained);
        }
    }
    result
}

/// Elide refcount traffic for values that are transferred, not shared.
pub fn run(program: &mut MirProgram) {
    for function in program.functions.values_mut() {
        let plan = find_move_outs(function);
        if plan.retain_sites.is_empty() {
            continue;
        }
        let drop_retain: HashSet<(usize, usize)> = plan.retain_sites.iter().copied().collect();
        for (bi, block) in function.blocks.iter_mut().enumerate() {
            let mut idx = 0;
            block.instrs.retain(|instr| {
                let keep = match instr {
                    MirInstr::Retain(_) => !drop_retain.contains(&(bi, idx)),
                    MirInstr::Release(local) => !plan.moved_locals.contains(local),
                    _ => true,
                };
                idx += 1;
                keep
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typecheck::TypeRef;
    use std::collections::HashMap;

    fn func(blocks: Vec<MirBlock>, nlocals: usize) -> MirFunction {
        MirFunction {
            id: FuncId(0),
            name: "main".to_string(),
            params: Vec::new(),
            locals: (0..nlocals)
                .map(|i| LocalDecl {
                    id: LocalId(i),
                    ty: TypeRef::Custom(crate::typecheck::StructTypeId(0)),
                    is_mut: true,
                    debug_name: None,
                    binding_id: None,
                    ownership: Ownership::Owned,
                })
                .collect(),
            blocks,
            start_block: BlockId(0),
            return_type: TypeRef::Void,
            is_async: false,
        }
    }

    fn spawn_block(extra_after_spawn: Vec<MirInstr>) -> MirBlock {
        let mut instrs = vec![
            MirInstr::AssignField {
                base: LocalId(1),
                field: "cap_0".to_string(),
                value: Operand::Borrowed(LocalId(0)),
            },
            MirInstr::Retain(LocalId(0)),
            MirInstr::Assign(LocalId(2), Rvalue::MakeClosure(FuncId(1), vec![Operand::Local(LocalId(1))])),
            MirInstr::Assign(LocalId(3), Rvalue::SpawnThread(Operand::Local(LocalId(2)))),
        ];
        instrs.extend(extra_after_spawn);
        instrs.push(MirInstr::Release(LocalId(0)));
        MirBlock { id: BlockId(0), instrs, terminator: Terminator::Return(None) }
    }

    fn run_one(f: MirFunction) -> MirFunction {
        let mut functions = HashMap::new();
        functions.insert(FuncId(0), f);
        let mut p = MirProgram { functions };
        run(&mut p);
        p.functions.remove(&FuncId(0)).unwrap()
    }

    #[test]
    fn handoff_elides_retain_and_release() {
        let f = run_one(func(vec![spawn_block(vec![])], 4));
        let retains = f.blocks[0].instrs.iter().filter(|i| matches!(i, MirInstr::Retain(_))).count();
        let releases = f.blocks[0]
            .instrs
            .iter()
            .filter(|i| matches!(i, MirInstr::Release(LocalId(0))))
            .count();
        assert_eq!(retains, 0, "handoff should not retain");
        assert_eq!(releases, 0, "handoff should not release the moved local");
    }

    #[test]
    fn later_use_keeps_refcounting() {
        // Reading _0 after the spawn makes this a genuine share.
        let use_after = vec![MirInstr::Assign(
            LocalId(3),
            Rvalue::FieldAccess(Operand::Borrowed(LocalId(0)), "id".to_string()),
        )];
        let f = run_one(func(vec![spawn_block(use_after)], 4));
        let retains = f.blocks[0].instrs.iter().filter(|i| matches!(i, MirInstr::Retain(_))).count();
        assert_eq!(retains, 1, "shared value must keep its retain");
    }

    #[test]
    fn spawn_in_a_loop_is_never_moved() {
        // Block 0 spawns, then branches back to itself: a later iteration can
        // re-read the binding, so "dead after this point" is meaningless.
        let mut b0 = spawn_block(vec![]);
        b0.terminator = Terminator::Goto(BlockId(0));
        let f = run_one(func(vec![b0], 4));
        let retains = f.blocks[0].instrs.iter().filter(|i| matches!(i, MirInstr::Retain(_))).count();
        assert_eq!(retains, 1, "a spawn inside a cycle must not be optimised");
    }

    #[test]
    fn use_in_a_successor_block_keeps_refcounting() {
        let mut b0 = spawn_block(vec![]);
        b0.terminator = Terminator::Goto(BlockId(1));
        let b1 = MirBlock {
            id: BlockId(1),
            instrs: vec![MirInstr::Assign(
                LocalId(3),
                Rvalue::FieldAccess(Operand::Borrowed(LocalId(0)), "id".to_string()),
            )],
            terminator: Terminator::Return(None),
        };
        let f = run_one(func(vec![b0, b1], 4));
        let retains = f.blocks[0].instrs.iter().filter(|i| matches!(i, MirInstr::Retain(_))).count();
        assert_eq!(retains, 1, "a use in a reachable successor is still a use");
    }

    #[test]
    fn retain_without_spawn_is_untouched() {
        let b0 = MirBlock {
            id: BlockId(0),
            instrs: vec![MirInstr::Retain(LocalId(0)), MirInstr::Release(LocalId(0))],
            terminator: Terminator::Return(None),
        };
        let f = run_one(func(vec![b0], 4));
        let retains = f.blocks[0].instrs.iter().filter(|i| matches!(i, MirInstr::Retain(_))).count();
        assert_eq!(retains, 1, "ordinary retains must not be removed");
    }
}
