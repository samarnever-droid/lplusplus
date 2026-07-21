use crate::escape::StorageClass;
use crate::mir::ir::*;
use crate::semantic::BindingId;
use crate::typecheck::TypeRef;
use std::collections::{HashMap, HashSet};

fn successors(terminator: &Terminator) -> Vec<usize> {
    match terminator {
        Terminator::Goto(target) => vec![target.0],
        Terminator::If { then_block, else_block, .. }
        | Terminator::IfCmp { then_block, else_block, .. } => vec![then_block.0, else_block.0],
        Terminator::Return(_) | Terminator::ReturnOwned(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// Transfer definite live-owned locals through one basic block.
///
/// The set contains locals that are known to hold an initialized ARC reference
/// on every path reaching the current point. `Move` removes its source; an
/// assignment creates/replaces the destination owner.
fn transfer_live(
    instructions: &[MirInstr],
    mut live: HashSet<LocalId>,
    arc_locals: &HashSet<LocalId>,
) -> HashSet<LocalId> {
    for instruction in instructions {
        if let MirInstr::Assign(destination, rvalue) = instruction {
            match rvalue {
                Rvalue::Move(source) => {
                    live.remove(source);
                }
                // Closure construction transfers the owned environment into
                // the closure capsule; the capsule destructor releases it.
                Rvalue::MakeClosure(_, captures) => {
                    if let Some(Operand::Local(environment)) = captures.first() {
                        live.remove(environment);
                    }
                }
                _ => {}
            }
            if arc_locals.contains(destination) {
                live.insert(*destination);
            }
        }
    }
    live
}

/// Insert ARC operations from explicit ownership information in MIR.
///
/// This pass is deliberately conservative. It calculates *definitely live*
/// owners with an intersection dataflow analysis, so it never emits a release
/// for a local that might be uninitialized on a branch. That may leave an
/// unsupported alias case allocated, but avoids the more serious failure of
/// dereferencing/freeing an uninitialized or moved value.
pub fn run_arc_insertion_pass(
    program: &mut MirProgram,
    _escape_map: &HashMap<BindingId, StorageClass>,
) {
    for function in program.functions.values_mut() {
        // All AOT custom-struct allocations use AllocateArcStruct. Therefore
        // every owned custom local has a valid ARC header and can be cleaned at
        // scope exit. Borrowed parameters remain caller-owned and are excluded.
        let arc_locals: HashSet<LocalId> = function
            .locals
            .iter()
            .filter(|local| {
                local.ownership == Ownership::Owned
                    && matches!(
                        &local.ty,
                        TypeRef::Custom(_) | TypeRef::Function | TypeRef::Generic(_, _)
                    )
            })
            .map(|local| local.id)
            .collect();

        if arc_locals.is_empty() {
            continue;
        }

        let block_count = function.blocks.len();
        let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); block_count];
        for block in &function.blocks {
            for successor in successors(&block.terminator) {
                if successor < block_count {
                    predecessors[successor].push(block.id.0);
                }
            }
        }

        // `entry_live[block]` is an intersection over all predecessor exits.
        // Start empty: an empty set is always safe until a fixed point proves
        // that an owner is initialized on every incoming path.
        let mut entry_live: Vec<HashSet<LocalId>> = vec![HashSet::new(); block_count];
        let mut changed = true;
        while changed {
            changed = false;
            for block in &function.blocks {
                if block.id == function.start_block {
                    continue;
                }
                let preds = &predecessors[block.id.0];
                if preds.is_empty() {
                    continue;
                }
                let mut incoming = transfer_live(
                    &function.blocks[preds[0]].instrs,
                    entry_live[preds[0]].clone(),
                    &arc_locals,
                );
                for predecessor in &preds[1..] {
                    let predecessor_exit = transfer_live(
                        &function.blocks[*predecessor].instrs,
                        entry_live[*predecessor].clone(),
                        &arc_locals,
                    );
                    incoming.retain(|local| predecessor_exit.contains(local));
                }
                if incoming != entry_live[block.id.0] {
                    entry_live[block.id.0] = incoming;
                    changed = true;
                }
            }
        }

        for block in &mut function.blocks {
            let mut live = entry_live[block.id.0].clone();
            let original = std::mem::take(&mut block.instrs);
            let mut rewritten = Vec::with_capacity(original.len() + arc_locals.len());

            for instruction in original {
                match &instruction {
                    MirInstr::Assign(destination, rvalue) => {
                        // Copy everything needed from the borrowed instruction
                        // before moving it into the rewritten block.
                        let destination = *destination;
                        let moved_source = match rvalue {
                            Rvalue::Move(source) => Some(*source),
                            // The environment reference becomes owned by the
                            // ARC closure capsule and is released by its
                            // destructor, not by the creating scope.
                            Rvalue::MakeClosure(_, captures) => match captures.first() {
                                Some(Operand::Local(environment)) => Some(*environment),
                                _ => None,
                            },
                            _ => None,
                        };
                        let borrowed_source = match rvalue {
                            Rvalue::Use(Operand::Borrowed(source)) => Some(*source),
                            _ => None,
                        };

                        // Reassignment drops the old owned reference first.
                        if arc_locals.contains(&destination) && live.remove(&destination) {
                            rewritten.push(MirInstr::Release(destination));
                        }
                        rewritten.push(instruction);

                        if let Some(source) = moved_source {
                            live.remove(&source);
                        }
                        if arc_locals.contains(&destination) {
                            live.insert(destination);
                            // A borrow becomes an additional owner at this
                            // assignment boundary ONLY when the destination
                            // is itself an ARC-managed local.  Retaining a
                            // scalar destination (Int/Bool/Float) is UB.
                            if borrowed_source.is_some() {
                                rewritten.push(MirInstr::Retain(destination));
                            }
                        }
                    }
                    MirInstr::AssignField {
                        value: Operand::Borrowed(source),
                        ..
                    } if matches!(
                        &function.locals[source.0].ty,
                        TypeRef::Custom(_) | TypeRef::Generic(_, _)
                    ) => {
                        let source = *source;
                        rewritten.push(instruction);
                        // Struct fields are owning edges under the current ARC
                        // model; preserve the borrowed source for that edge.
                        rewritten.push(MirInstr::Retain(source));
                    }
                    _ => rewritten.push(instruction),
                }
            }

            if let Terminator::Return(_) | Terminator::ReturnOwned(_) = &block.terminator {
                let returned_local = match &block.terminator {
                    Terminator::ReturnOwned(Operand::Local(local)) => Some(*local),
                    _ => None,
                };
                for local in &live {
                    if Some(*local) != returned_local {
                        rewritten.push(MirInstr::Release(*local));
                    }
                }
            }

            block.instrs = rewritten;
        }
    }
}
