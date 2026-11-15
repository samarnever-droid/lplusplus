//! Shared ownership-driven lowering analysis for native codegen backends.
//!
//! Bridges `lpp_ownership`'s `Frame` vs `Owned` vs `Shared` classification
//! into concrete Cranelift and LLVM lowering choices:
//! - Non-escaping `Frame` POD structs/tuples -> stack slots (`StackSlot` / `alloca`).
//! - Escaping `Owned` POD structs/tuples -> destructor-free ARC headers
//!   (`lpp_arc_alloc_with_destructor(size, null)`).
//! - `Shared` types -> collectible cycle-tracked heap allocations.

use std::collections::{BTreeMap, BTreeSet};

use lpp_mir::{
    InstructionKind, MirAggregateKind, MirFunction, MirFunctionId, MirLocalId, MirLocalKind,
    MirProgram, Operand, PlaceProjection, Rvalue, Terminator,
};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{PrimitiveType, TypeId, TypeInterner, TypeKind};

use crate::layout::{AggregateLayout, aggregate_layout, tuple_layout};

/// Summary of ownership-driven lowering decisions shared by native backends.
#[derive(Debug, Clone, Default)]
pub struct OwnershipLoweringPlan {
    /// Cycle-member types requiring runtime cycle collection registration.
    pub shared_types: BTreeSet<TypeId>,
    /// Non-escaping POD struct/tuple locals eligible for stack allocation,
    /// mapped to their minimum byte size (`>= 8`).
    pub stack_locals: BTreeMap<(MirFunctionId, MirLocalId), u32>,
}

/// Returns `true` when `ty` is represented as an ARC-managed heap pointer in native ABIs.
#[must_use]
pub fn is_managed_value(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(
        types.kind(ty),
        TypeKind::Primitive(PrimitiveType::String)
            | TypeKind::List(_)
            | TypeKind::Nominal { .. }
            | TypeKind::Task(_)
            | TypeKind::Function { .. }
            | TypeKind::Tuple(_)
    )
}

/// Returns `true` when an aggregate layout contains zero ARC-managed fields.
#[must_use]
pub fn is_pod_aggregate(layout: &AggregateLayout, types: &TypeInterner) -> bool {
    match layout.kind {
        MirAggregateKind::Struct => layout
            .struct_fields
            .iter()
            .all(|slot| !is_managed_value(types, slot.ty)),
        MirAggregateKind::Enum => layout
            .variant_fields
            .iter()
            .flatten()
            .all(|slot| !is_managed_value(types, slot.ty)),
    }
}

/// Returns `true` when a tuple type contains zero ARC-managed elements.
#[must_use]
pub fn is_pod_tuple(types: &TypeInterner, ty: TypeId) -> bool {
    tuple_layout(types, ty).is_some_and(|(slots, _)| {
        slots
            .iter()
            .all(|(_, elem_ty)| !is_managed_value(types, *elem_ty))
    })
}

/// Computes `Shared` cycle types and stack-eligible `Frame` POD locals for a MIR program.
#[must_use]
pub fn build_ownership_lowering_plan(
    program: &MirProgram,
    types: &TypeInterner,
) -> OwnershipLoweringPlan {
    let Ok(ownership) = compute_ownership_plan(program, types) else {
        return OwnershipLoweringPlan::default();
    };
    let shared_types: BTreeSet<TypeId> = ownership.pinned_types().into_iter().collect();

    let mut aggregate_sizes: BTreeMap<TypeId, u32> = BTreeMap::new();
    for (agg_id, agg) in program.aggregates() {
        let layout = aggregate_layout(program, types, agg_id);
        if agg.kind == MirAggregateKind::Struct && is_pod_aggregate(&layout, types) {
            aggregate_sizes.insert(agg.ty, layout.total_size.max(8));
        }
    }

    let mut stack_locals: BTreeMap<(MirFunctionId, MirLocalId), u32> = BTreeMap::new();
    for arena in &ownership.arenas {
        let Some(function) = program.function(arena.function) else {
            continue;
        };
        let mut candidate_sizes: BTreeMap<MirLocalId, (TypeId, u32)> = BTreeMap::new();
        for &local in &arena.cells {
            let Some(descriptor) = program.local(local) else {
                continue;
            };
            if !matches!(
                descriptor.kind,
                MirLocalKind::User | MirLocalKind::Temporary
            ) || shared_types.contains(&descriptor.ty)
            {
                continue;
            }
            if let Some(byte_size) = pod_stack_size(types, &aggregate_sizes, descriptor.ty) {
                candidate_sizes.insert(local, (descriptor.ty, byte_size));
            }
        }
        if candidate_sizes.is_empty() {
            continue;
        }
        for local in select_stack_locals(program, function, &candidate_sizes) {
            let (_, byte_size) = candidate_sizes[&local];
            stack_locals.insert((arena.function, local), byte_size);
        }
    }

    OwnershipLoweringPlan {
        shared_types,
        stack_locals,
    }
}

fn pod_stack_size(
    types: &TypeInterner,
    pod_struct_sizes: &BTreeMap<TypeId, u32>,
    ty: TypeId,
) -> Option<u32> {
    match types.kind(ty) {
        TypeKind::Nominal { .. } => pod_struct_sizes.get(&ty).copied(),
        TypeKind::Tuple(_) if is_pod_tuple(types, ty) => {
            tuple_layout(types, ty).map(|(_, total_size)| total_size.max(8))
        }
        _ => None,
    }
}

fn operand_local(operand: &Operand) -> Option<MirLocalId> {
    match operand {
        Operand::Copy(id) => Some(*id),
        _ => None,
    }
}

fn operand_mentions_local(operand: &Operand, local: MirLocalId) -> bool {
    operand_local(operand) == Some(local)
}

fn place_escapes_or_reads_whole_local(
    program: &MirProgram,
    place_id: lpp_mir::MirPlaceId,
    local: MirLocalId,
) -> bool {
    let Some(place) = program.place(place_id) else {
        return true;
    };
    let projections = program.place_projections(place);
    if place.root == local && projections.is_empty() {
        return true;
    }
    projections.iter().any(|proj| match proj {
        PlaceProjection::ListIndex(index_op) => operand_mentions_local(index_op, local),
        PlaceProjection::Downcast(_)
        | PlaceProjection::Field(_)
        | PlaceProjection::TupleField(_) => false,
    })
}

fn collect_rvalue_operand_locals(program: &MirProgram, rvalue: &Rvalue, out: &mut Vec<MirLocalId>) {
    let push_op = |op: &Operand, dest: &mut Vec<MirLocalId>| {
        if let Some(id) = operand_local(op) {
            dest.push(id);
        }
    };
    match rvalue {
        Rvalue::Use(op)
        | Rvalue::Unary { operand: op, .. }
        | Rvalue::Cast { value: op, .. }
        | Rvalue::ListLen(op)
        | Rvalue::Await(op)
        | Rvalue::Spawn(op) => push_op(op, out),
        Rvalue::Binary { left, right, .. } => {
            push_op(left, out);
            push_op(right, out);
        }
        Rvalue::Load(place_id) => {
            if let Some(place) = program.place(*place_id) {
                let projections = program.place_projections(place);
                if projections.is_empty() {
                    out.push(place.root);
                }
                for proj in projections {
                    if let PlaceProjection::ListIndex(idx) = proj {
                        push_op(idx, out);
                    }
                }
            }
        }
        Rvalue::Tuple(ops)
        | Rvalue::List(ops)
        | Rvalue::ConstructStruct { fields: ops, .. }
        | Rvalue::ConstructVariant { fields: ops, .. }
        | Rvalue::ExternalCall { arguments: ops, .. }
        | Rvalue::Builtin { arguments: ops, .. }
        | Rvalue::Intrinsic { arguments: ops, .. }
        | Rvalue::MakeClosure { captures: ops, .. } => {
            for op in program.operands(*ops) {
                push_op(op, out);
            }
        }
        Rvalue::Call { callee, arguments } => {
            push_op(callee, out);
            for op in program.operands(*arguments) {
                push_op(op, out);
            }
        }
    }
}

/// Identifies candidate locals safe for stack allocation:
/// 1. Constructor temporaries (`Temporary`, constructed directly, only ever moved
///    via `Rvalue::Use(Copy(temp))` into same-typed candidate targets).
/// 2. User/temp locals constructed directly or initialized exclusively from such
///    constructor temporaries, accessed only through field/tuple-field projections.
fn select_stack_locals(
    program: &MirProgram,
    function: &MirFunction,
    candidates: &BTreeMap<MirLocalId, (TypeId, u32)>,
) -> Vec<MirLocalId> {
    let mut temp_sources: BTreeMap<MirLocalId, Vec<MirLocalId>> = BTreeMap::new();
    for &local in candidates.keys() {
        if program
            .local(local)
            .is_some_and(|desc| desc.kind == MirLocalKind::Temporary)
        {
            temp_sources.insert(local, Vec::new());
        }
    }

    let mut invalid_temp: BTreeSet<MirLocalId> = BTreeSet::new();
    let mut escaped: BTreeSet<MirLocalId> = BTreeSet::new();
    let mut user_deps: BTreeMap<MirLocalId, Vec<MirLocalId>> = BTreeMap::new();
    let mut has_direct_construct: BTreeSet<MirLocalId> = BTreeSet::new();

    for &block_id in program.function_blocks(function) {
        let Some(block) = program.block(block_id) else {
            return Vec::new();
        };
        for &inst_id in program.block_instructions(block) {
            let Some(inst) = program.instruction(inst_id) else {
                return Vec::new();
            };
            match &inst.kind {
                InstructionKind::Assign { target, value } => {
                    let direct_construct =
                        matches!(value, Rvalue::ConstructStruct { .. } | Rvalue::Tuple(_));
                    let copy_src = match value {
                        Rvalue::Use(Operand::Copy(src)) => Some(*src),
                        _ => None,
                    };

                    if candidates.contains_key(target) {
                        if direct_construct {
                            has_direct_construct.insert(*target);
                        } else if let Some(src) = copy_src
                            && candidates
                                .get(&src)
                                .is_some_and(|(src_ty, _)| *src_ty == candidates[target].0)
                            && temp_sources.contains_key(&src)
                        {
                            user_deps.entry(*target).or_default().push(src);
                            temp_sources.entry(src).or_default().push(*target);
                        } else {
                            escaped.insert(*target);
                            invalid_temp.insert(*target);
                        }
                    }

                    // Any occurrence of a candidate in `Use(Copy(c))` where the assignment
                    // target is also a same-typed candidate counts only as a temp->target move
                    // handled above; every other operand/place use disqualifies it.
                    let handled_move = copy_src.is_some_and(|src| {
                        candidates.contains_key(target)
                            && candidates
                                .get(&src)
                                .is_some_and(|(src_ty, _)| *src_ty == candidates[target].0)
                            && temp_sources.contains_key(&src)
                    });
                    if !handled_move {
                        let mut mentions = Vec::new();
                        collect_rvalue_operand_locals(program, value, &mut mentions);
                        for m in mentions {
                            if candidates.contains_key(&m) {
                                escaped.insert(m);
                                invalid_temp.insert(m);
                            }
                        }
                    } else if let Some(src) = copy_src
                        && *target == src
                    {
                        escaped.insert(src);
                        invalid_temp.insert(src);
                    }

                    // Temporaries acting as pure constructor sources must never even be projected.
                    if let Rvalue::Load(place_id) = value
                        && let Some(place) = program.place(*place_id)
                    {
                        if temp_sources.contains_key(&place.root) {
                            invalid_temp.insert(place.root);
                        }
                        if candidates.contains_key(&place.root)
                            && program.place_projections(place).is_empty()
                        {
                            escaped.insert(place.root);
                        }
                    }
                }
                InstructionKind::Store { place, value } => {
                    if let Some(id) = operand_local(value)
                        && candidates.contains_key(&id)
                    {
                        escaped.insert(id);
                        invalid_temp.insert(id);
                    }
                    if let Some(p) = program.place(*place) {
                        if temp_sources.contains_key(&p.root) {
                            invalid_temp.insert(p.root);
                        }
                        for &cand in candidates.keys() {
                            if place_escapes_or_reads_whole_local(program, *place, cand) {
                                escaped.insert(cand);
                                invalid_temp.insert(cand);
                            }
                        }
                    }
                }
            }
        }
        let term_op = match &block.terminator {
            Terminator::Branch { condition, .. } => Some(condition),
            Terminator::SwitchEnum { subject, .. } => Some(subject),
            Terminator::Return(Some(op)) => Some(op),
            Terminator::Goto(_) | Terminator::Return(None) | Terminator::Unreachable => None,
        };
        if let Some(op) = term_op
            && let Some(id) = operand_local(op)
            && candidates.contains_key(&id)
        {
            escaped.insert(id);
            invalid_temp.insert(id);
        }
    }

    // Iterate to fixed point: a user local is rejected if any of its temp sources is invalid,
    // and a temp source is invalid if any of its destination locals was rejected/escaped.
    let mut rejected: BTreeSet<MirLocalId> = escaped.clone();
    loop {
        let mut changed = false;
        for (&local, deps) in &user_deps {
            if !rejected.contains(&local)
                && deps
                    .iter()
                    .any(|src| invalid_temp.contains(src) || rejected.contains(src))
            {
                rejected.insert(local);
                changed = true;
            }
        }
        for (&tmp, dsts) in &temp_sources {
            if !invalid_temp.contains(&tmp)
                && (dsts.is_empty() || dsts.iter().any(|dst| rejected.contains(dst)))
            {
                invalid_temp.insert(tmp);
                rejected.insert(tmp);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    candidates
        .keys()
        .copied()
        .filter(|local| {
            let is_valid_temp = temp_sources.get(local).is_some_and(|dsts| {
                !dsts.is_empty() && !invalid_temp.contains(local) && !rejected.contains(local)
            });
            let is_valid_dest = !temp_sources.contains_key(local)
                && !rejected.contains(local)
                && (has_direct_construct.contains(local) || user_deps.contains_key(local));
            is_valid_temp || is_valid_dest
        })
        .collect()
}
