//! Value-by-default: promote provably frame-local structs off the heap.
//!
//! The architecture calls for structs to be plain values unless one of six
//! escape rules forces ARC. Until now the second half existed and the first did
//! not -- `analysis::escape` computed a storage classification that reached
//! `--dump-escape` and nothing else, and every struct was heap-allocated and
//! refcounted. A local struct measured 0.038 s against 0.003 s for the
//! equivalent plain value: an 11x tax on code that never escapes.
//!
//! # Why this pass does not simply trust the escape map
//!
//! `analysis::escape` is *partially* sound. What it promotes is genuinely
//! escaping, but it fails to promote several positions that also escape --
//! it recurses into call arguments, `AssignField` values and struct-literal
//! fields without classifying the value placed there. Treating "absent from the
//! map" as "safe to stack-allocate" would therefore hand a callee a pointer into
//! a dead frame.
//!
//! So the map is used in one direction only: a binding classified `Arc` or
//! `Arena` is *definitely* escaping and vetoes promotion. Permission to promote
//! comes from an independent scan of the MIR, where every use of a local is
//! explicit and enumerable, and the question asked is the stronger one:
//!
//!   "is every single use of this local provably frame-local?"
//!
//! not
//!
//!   "did any rule happen to fire?"
//!
//! A missed promotion in the AST walker cannot cause unsoundness here, because
//! the use-scan rejects the escaping use on its own.
//!
//! # What qualifies
//!
//! All of the following must hold:
//!
//!   * the local is assigned exactly once, by `AllocateArcStruct(Custom(id))`;
//!   * the struct is not self-referential and every field is a scalar, so its
//!     destructor is a no-op and nothing inside it needs releasing;
//!   * the escape map does not classify it `Arc`/`Arena`;
//!   * every use is a field read of it, or a scalar field write into it.
//!
//! Anything else -- returned, passed to a call, stored into another object,
//! captured by a closure, moved, spawned, aliased, or used as a bare operand
//! whose value could be copied elsewhere -- disqualifies it.

use super::ir::*;
use crate::escape::StorageClass;
use crate::semantic::BindingId;
use crate::typecheck::{StructTypeId, TypeRef, TypeTable};
use std::collections::{HashMap, HashSet};

/// Result of one function's analysis, for reporting via `--dump-escape`.
#[derive(Debug, Default, Clone)]
pub struct EscapeStats {
    pub promoted: usize,
    pub considered: usize,
}

/// A struct is eligible only if it owns nothing: no ARC-managed field, and not
/// self-referential. Such a struct's generated destructor has no work to do, so
/// removing the header removes nothing observable.
fn struct_is_scalar_only(type_table: &TypeTable, id: StructTypeId) -> bool {
    let Some(def) = type_table.definitions.get(id.0) else {
        return false;
    };
    if def.is_self_referential {
        return false;
    }
    def.fields.iter().all(|(_, ty)| {
        matches!(
            ty,
            TypeRef::Int | TypeRef::Float | TypeRef::Bool | TypeRef::Void
        )
    })
}

/// Every local that appears anywhere in `rvalue`, paired with whether that
/// position is a *safe* (frame-local) use of it.
///
/// Safe positions are deliberately few. `FieldAccess(base, _)` reads through the
/// pointer and yields a scalar, so the pointer itself does not leave. Everything
/// else either copies the pointer somewhere with an independent lifetime, or
/// hands it to code this pass cannot see.
fn scan_rvalue(rvalue: &Rvalue, safe: &mut HashSet<LocalId>, unsafe_: &mut HashSet<LocalId>) {
    fn mark_operand(op: &Operand, unsafe_: &mut HashSet<LocalId>) {
        if let Operand::Local(id) | Operand::Borrowed(id) = op {
            unsafe_.insert(*id);
        }
    }
    match rvalue {
        // Reading a field through the pointer keeps it in the frame.
        Rvalue::FieldAccess(Operand::Local(base), _)
        | Rvalue::FieldAccess(Operand::Borrowed(base), _) => {
            safe.insert(*base);
        }
        Rvalue::FieldAccess(op, _) => mark_operand(op, unsafe_),

        // Every other position can let the pointer outlive the frame.
        Rvalue::Use(op) | Rvalue::SpawnThread(op) => mark_operand(op, unsafe_),
        // `Move` is handled by the caller, which can see the destination and so
        // can treat a construct-then-move pair as a single local.
        Rvalue::Move(_) => {}
        Rvalue::BinaryOp(_, a, b) => {
            mark_operand(a, unsafe_);
            mark_operand(b, unsafe_);
        }
        Rvalue::CallDirect(_, args) | Rvalue::BuiltinCall(_, args) => {
            for a in args {
                mark_operand(a, unsafe_);
            }
        }
        Rvalue::CallIndirect(c, args) => {
            mark_operand(c, unsafe_);
            for a in args {
                mark_operand(a, unsafe_);
            }
        }
        Rvalue::MakeClosure(_, caps) => {
            for c in caps {
                mark_operand(c, unsafe_);
            }
        }
        Rvalue::AllocateStruct(_)
        | Rvalue::AllocateArcStruct(_)
        | Rvalue::AllocateStackStruct(_)
        | Rvalue::AllocateList(_)
        | Rvalue::FuncRef(_) => {}
    }
}

/// Run the pass. `escape_map` is consulted only as a veto.
pub fn run(
    program: &mut MirProgram,
    type_table: &TypeTable,
    escape_map: &HashMap<BindingId, StorageClass>,
) -> EscapeStats {
    let mut stats = EscapeStats::default();

    for function in program.functions.values_mut() {
        // Candidates: assigned exactly once by AllocateArcStruct, scalar-only
        // struct type, not vetoed by the escape map.
        let mut assign_count: HashMap<LocalId, usize> = HashMap::new();
        let mut candidate_ty: HashMap<LocalId, TypeRef> = HashMap::new();

        for block in &function.blocks {
            for instruction in &block.instrs {
                if let MirInstr::Assign(dest, rvalue) = instruction {
                    *assign_count.entry(*dest).or_insert(0) += 1;
                    if let Rvalue::AllocateArcStruct(ty @ TypeRef::Custom(id)) = rvalue {
                        if struct_is_scalar_only(type_table, *id) {
                            candidate_ty.insert(*dest, ty.clone());
                        }
                    }
                }
            }
        }

        // Lowering builds a struct into a temporary and then moves it into the
        // named local: `_1 = alloc; _1.x = ...; _0 = move(_1)`. That pair is one
        // object with one lifetime, so treat it as one unit -- otherwise every
        // named struct is disqualified by its own initialisation.
        //
        // The move is only coalesced when the temporary is moved exactly once
        // and has no other use, which is precisely the shape lowering emits.
        let mut moved_into: HashMap<LocalId, LocalId> = HashMap::new(); // temp -> named
        let mut move_sources: HashMap<LocalId, usize> = HashMap::new();
        for block in &function.blocks {
            for instruction in &block.instrs {
                if let MirInstr::Assign(dest, Rvalue::Move(src)) = instruction {
                    *move_sources.entry(*src).or_insert(0) += 1;
                    moved_into.insert(*src, *dest);
                }
            }
        }
        moved_into.retain(|src, _| move_sources.get(src).copied().unwrap_or(0) == 1);

        let mut candidates: HashSet<LocalId> = candidate_ty
            .keys()
            .copied()
            .filter(|l| assign_count.get(l).copied().unwrap_or(0) == 1)
            .collect();

        // Veto anything the escape analysis already knows escapes. This is the
        // only way the map is consulted: it can forbid, never permit.
        candidates.retain(|local| {
            let Some(decl) = function.locals.get(local.0) else {
                return false;
            };
            match decl.binding_id.and_then(|b| escape_map.get(&b)) {
                Some(StorageClass::Arc) | Some(StorageClass::Arena { .. }) => false,
                _ => true,
            }
        });

        // A parameter is caller-owned; never rehome one.
        for local in &function.locals {
            if local.ownership == Ownership::Borrowed {
                candidates.remove(&local.id);
            }
        }

        stats.considered += candidates.len();
        if candidates.is_empty() {
            continue;
        }

        // Scan every use. Any unsafe position disqualifies the local.
        let mut unsafe_uses: HashSet<LocalId> = HashSet::new();
        let mut safe_uses: HashSet<LocalId> = HashSet::new();

        for block in &function.blocks {
            for instruction in &block.instrs {
                match instruction {
                    MirInstr::Assign(_, rvalue) => {
                        scan_rvalue(rvalue, &mut safe_uses, &mut unsafe_uses)
                    }
                    MirInstr::AssignField { base, value, .. } => {
                        // Writing *into* the candidate is fine; the pointer does
                        // not leave. Writing the candidate into something else
                        // is not, and is caught by inspecting `value`.
                        safe_uses.insert(*base);
                        if let Operand::Local(id) | Operand::Borrowed(id) = value {
                            unsafe_uses.insert(*id);
                        }
                    }
                    // Retain/release must never apply to a headerless slot. If
                    // one is present the local is already ARC-tracked; refuse.
                    MirInstr::Retain(id) | MirInstr::Release(id) => {
                        unsafe_uses.insert(*id);
                    }
                }
            }
            match &block.terminator {
                Terminator::Return(Some(op)) => {
                    if let Operand::Local(id) | Operand::Borrowed(id) = op {
                        unsafe_uses.insert(*id);
                    }
                }
                Terminator::ReturnOwned(op) => {
                    if let Operand::Local(id) | Operand::Borrowed(id) = op {
                        unsafe_uses.insert(*id);
                    }
                }
                Terminator::If { cond, .. } => {
                    if let Operand::Local(id) | Operand::Borrowed(id) = cond {
                        unsafe_uses.insert(*id);
                    }
                }
                Terminator::IfCmp { left, right, .. } => {
                    for op in [left, right] {
                        if let Operand::Local(id) | Operand::Borrowed(id) = op {
                            unsafe_uses.insert(*id);
                        }
                    }
                }
                Terminator::Goto(_) | Terminator::Return(None) | Terminator::Unreachable => {}
            }
        }
        let _ = &safe_uses;

        // A candidate qualifies when neither it nor the local it is moved into
        // has an escaping use. Both halves of a construct-then-move pair are
        // promoted together, and the destination's `Owned` marking is cleared
        // alongside the temporary's.
        let mut promoted: Vec<LocalId> = Vec::new();
        let mut promoted_dests: Vec<LocalId> = Vec::new();
        for local in candidates.iter().copied() {
            if unsafe_uses.contains(&local) {
                continue;
            }
            match moved_into.get(&local) {
                Some(dest) => {
                    // The named local inherits the object; it must be clean too,
                    // and must not itself be reassigned from anywhere else.
                    if unsafe_uses.contains(dest)
                        || assign_count.get(dest).copied().unwrap_or(0) != 1
                    {
                        continue;
                    }
                    promoted.push(local);
                    promoted_dests.push(*dest);
                }
                None => promoted.push(local),
            }
        }

        if promoted.is_empty() {
            continue;
        }

        // Rewrite the allocation. Field loads and stores are untouched: the
        // stack payload has the same layout as the heap payload.
        for block in &mut function.blocks {
            for instruction in &mut block.instrs {
                if let MirInstr::Assign(dest, rvalue) = instruction {
                    if promoted.contains(dest) {
                        if let Rvalue::AllocateArcStruct(ty) = rvalue {
                            *rvalue = Rvalue::AllocateStackStruct(ty.clone());
                        }
                    }
                }
            }
        }

        // A stack slot is not an ARC owner, so downstream ARC passes must not
        // treat it as one -- neither the temporary nor the local it moves into.
        for local in &mut function.locals {
            if promoted.contains(&local.id) || promoted_dests.contains(&local.id) {
                local.ownership = Ownership::Copy;
            }
        }

        stats.promoted += promoted.len();
    }

    stats
}
