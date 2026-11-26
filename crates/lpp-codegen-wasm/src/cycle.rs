//! Bacon-Rajan trial-deletion cycle collection for the WASM backend.
//!
//! When a program constructs cycle-capable (`Shared`) types, this module
//! computes the cycle-capable `TypeId` set via the type containment graph and
//! Tarjan strongly connected components, provides cycle-aware `ArcAlloc` /
//! `Retain` / `Release` helpers preserving the 24-byte ARC header and
//! size-class free-list bins, and emits trace dispatch + trial-deletion
//! collector functions.

use std::collections::{BTreeMap, BTreeSet};

use lpp_codegen_api::CodegenError;
use lpp_mir::{
    InstructionKind, MirAggregateId, MirAggregateKind, MirFunctionId, MirFunctionKind, MirLocalId,
    MirLocalKind, MirProgram, Operand, Rvalue,
};
use lpp_types::{PrimitiveType, TypeId, TypeInterner, TypeKind};

use crate::encode::{FB, Val, op};
use crate::{
    ARC_FREE_BIN_BASE, ARC_FREE_BIN_COUNT, ARC_HEADER, ARC_MAGIC, ARC_MAX_EXP, ARC_MIN_EXP, Env,
    Func, GLOBAL_HEAP, H, IMMORTAL_RC, LIST_DATA, LIST_IS_ARC, LIST_LEN, is_managed, unsupported,
};

/// Base WASM global index for persistent cycle-collector state when `has_cycles`.
pub(crate) const CYCLE_GLOBAL_BASE: u32 = 1 + ARC_FREE_BIN_COUNT;
/// Number of mutable `i32` globals emitted for the cycle collector.
pub(crate) const CYCLE_GLOBAL_COUNT: u32 = 13;

const G_COLLECTING: u32 = CYCLE_GLOBAL_BASE;
const G_ROOT_BUF: u32 = CYCLE_GLOBAL_BASE + 1;
const G_ROOT_LEN: u32 = CYCLE_GLOBAL_BASE + 2;
const G_ROOT_CAP: u32 = CYCLE_GLOBAL_BASE + 3;
const G_SUB_BUF: u32 = CYCLE_GLOBAL_BASE + 4;
const G_SUB_LEN: u32 = CYCLE_GLOBAL_BASE + 5;
const G_SUB_CAP: u32 = CYCLE_GLOBAL_BASE + 6;
const G_HT_BUF: u32 = CYCLE_GLOBAL_BASE + 7;
const G_HT_CAP: u32 = CYCLE_GLOBAL_BASE + 8;
const G_STACK_BUF: u32 = CYCLE_GLOBAL_BASE + 9;
const G_STACK_LEN: u32 = CYCLE_GLOBAL_BASE + 10;
const G_STACK_CAP: u32 = CYCLE_GLOBAL_BASE + 11;
const G_TRACE_PHASE: u32 = CYCLE_GLOBAL_BASE + 12;

const FLAG_COLLECTIBLE: i64 = 1;
const FLAG_BUFFERED: i64 = 2;
const COLLECT_THRESHOLD: i64 = 512;
const MAX_COLLECT_ROUNDS: i64 = 64;

/// Computes the set of cycle-capable (`Shared`) types in `program`.
///
/// Mirrors `lpp-ownership`'s containment graph + Tarjan SCC analysis over
/// structs, enums, `List`, `Slice`, `Tuple`, `Map`, `Task`, and closures
/// (`Function`).
pub(crate) fn compute_shared_types(
    program: &MirProgram,
    types: &TypeInterner,
    aggregates: &[MirAggregateId],
) -> Result<BTreeSet<TypeId>, CodegenError> {
    let mut nodes: BTreeSet<TypeId> = BTreeSet::new();
    let mut edges: BTreeMap<TypeId, BTreeSet<TypeId>> = BTreeMap::new();
    let mut callables: Vec<TypeId> = Vec::new();
    let mut async_arg_types: BTreeSet<TypeId> = BTreeSet::new();

    for &agg_id in aggregates {
        if let Some(agg) = program.aggregate(agg_id) {
            nodes.insert(agg.ty);
            match agg.kind {
                MirAggregateKind::Struct => {
                    for &f_id in program.aggregate_fields(agg) {
                        if let Some(f) = program.field(f_id)
                            && is_managed(types, f.ty)
                        {
                            edges.entry(agg.ty).or_default().insert(f.ty);
                            nodes.insert(f.ty);
                        }
                    }
                }
                MirAggregateKind::Enum => {
                    for &v_id in program.aggregate_variants(agg) {
                        if let Some(v) = program.variant(v_id) {
                            for &f_id in program.variant_fields(v) {
                                if let Some(f) = program.field(f_id)
                                    && is_managed(types, f.ty)
                                {
                                    edges.entry(agg.ty).or_default().insert(f.ty);
                                    nodes.insert(f.ty);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    for (_, func) in program.functions() {
        if is_managed(types, func.return_type) {
            nodes.insert(func.return_type);
        }
        let is_async = matches!(func.kind, MirFunctionKind::Async);
        for &p_id in program.function_parameters(func) {
            if let Some(local) = program.local(p_id)
                && is_managed(types, local.ty)
            {
                nodes.insert(local.ty);
                if is_async {
                    async_arg_types.insert(local.ty);
                }
            }
        }
        for &l_id in program.function_locals(func) {
            if let Some(local) = program.local(l_id)
                && is_managed(types, local.ty)
            {
                nodes.insert(local.ty);
            }
        }
        for &b_id in program.function_blocks(func) {
            if let Some(block) = program.block(b_id) {
                for &inst_id in program.block_instructions(block) {
                    if let Some(inst) = program.instruction(inst_id)
                        && let InstructionKind::Assign {
                            value:
                                Rvalue::Call {
                                    callee: Operand::Function(callee_id),
                                    arguments,
                                },
                            ..
                        } = &inst.kind
                        && program
                            .function(*callee_id)
                            .is_some_and(|f| matches!(f.kind, MirFunctionKind::Async))
                    {
                        for arg in program.operands(*arguments) {
                            if let Some(ty) = operand_ty(program, arg)
                                && is_managed(types, ty)
                            {
                                async_arg_types.insert(ty);
                                nodes.insert(ty);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut work: Vec<TypeId> = nodes.iter().copied().collect();
    let mut visited: BTreeSet<TypeId> = BTreeSet::new();
    while let Some(ty) = work.pop() {
        if !visited.insert(ty) {
            continue;
        }
        nodes.insert(ty);
        match types.kind(ty) {
            TypeKind::List(elem) | TypeKind::Slice(elem) => {
                if is_managed(types, elem) {
                    edges.entry(ty).or_default().insert(elem);
                    work.push(elem);
                }
            }
            TypeKind::Tuple(elems) => {
                for &elem in types.list(elems) {
                    if is_managed(types, elem) {
                        edges.entry(ty).or_default().insert(elem);
                        work.push(elem);
                    }
                }
            }
            TypeKind::Map { key, value } => {
                if is_managed(types, key) {
                    edges.entry(ty).or_default().insert(key);
                    work.push(key);
                }
                if is_managed(types, value) {
                    edges.entry(ty).or_default().insert(value);
                    work.push(value);
                }
            }
            TypeKind::Task(res) => {
                if is_managed(types, res) {
                    edges.entry(ty).or_default().insert(res);
                    work.push(res);
                }
                for &arg_ty in &async_arg_types {
                    edges.entry(ty).or_default().insert(arg_ty);
                    work.push(arg_ty);
                }
            }
            TypeKind::Function { .. } => {
                callables.push(ty);
            }
            _ => {}
        }
    }

    // Closures (`TypeKind::Function`) can capture any non-String managed type.
    if !callables.is_empty() {
        let non_str_nodes: Vec<TypeId> = nodes
            .iter()
            .copied()
            .filter(|&t| !matches!(types.kind(t), TypeKind::Primitive(PrimitiveType::String)))
            .collect();
        for c_ty in &callables {
            let entry = edges.entry(*c_ty).or_default();
            for &target in &non_str_nodes {
                entry.insert(target);
            }
        }
    }

    let sccs = tarjan_sccs(&nodes, &edges);
    let mut shared = BTreeSet::new();
    for scc in sccs {
        let is_cyclic = scc.len() > 1
            || (scc.len() == 1
                && edges
                    .get(&scc[0])
                    .is_some_and(|nbrs| nbrs.contains(&scc[0])));
        if is_cyclic {
            for ty in scc {
                shared.insert(ty);
            }
        }
    }
    Ok(shared)
}

fn operand_ty(program: &MirProgram, op: &Operand) -> Option<TypeId> {
    match op {
        Operand::Copy(local_id) => program.local(*local_id).map(|l| l.ty),
        Operand::Function(fn_id) => program.function(*fn_id).map(|f| f.ty),
        Operand::Constant(_) => None,
    }
}

fn tarjan_sccs(
    nodes: &BTreeSet<TypeId>,
    edges: &BTreeMap<TypeId, BTreeSet<TypeId>>,
) -> Vec<Vec<TypeId>> {
    struct Frame {
        v: TypeId,
        nbrs: Vec<TypeId>,
        next_idx: usize,
    }

    let mut index_counter: usize = 0;
    let mut indices: BTreeMap<TypeId, usize> = BTreeMap::new();
    let mut lowlink: BTreeMap<TypeId, usize> = BTreeMap::new();
    let mut on_stack: BTreeSet<TypeId> = BTreeSet::new();
    let mut tarjan_stack: Vec<TypeId> = Vec::new();
    let mut call_stack: Vec<Frame> = Vec::new();
    let mut sccs: Vec<Vec<TypeId>> = Vec::new();

    for &start in nodes {
        if indices.contains_key(&start) {
            continue;
        }
        indices.insert(start, index_counter);
        lowlink.insert(start, index_counter);
        index_counter += 1;
        tarjan_stack.push(start);
        on_stack.insert(start);
        call_stack.push(Frame {
            v: start,
            nbrs: edges
                .get(&start)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default(),
            next_idx: 0,
        });

        while let Some(frame) = call_stack.last_mut() {
            let v = frame.v;
            if frame.next_idx < frame.nbrs.len() {
                let w = frame.nbrs[frame.next_idx];
                frame.next_idx += 1;
                if let std::collections::btree_map::Entry::Vacant(e) = indices.entry(w) {
                    e.insert(index_counter);
                    lowlink.insert(w, index_counter);
                    index_counter += 1;
                    tarjan_stack.push(w);
                    on_stack.insert(w);
                    call_stack.push(Frame {
                        v: w,
                        nbrs: edges
                            .get(&w)
                            .map(|s| s.iter().copied().collect())
                            .unwrap_or_default(),
                        next_idx: 0,
                    });
                } else if on_stack.contains(&w) {
                    let w_idx = indices[&w];
                    let v_low = lowlink.get_mut(&v).expect("lowlink exists");
                    if w_idx < *v_low {
                        *v_low = w_idx;
                    }
                }
            } else {
                let done_v = call_stack.pop().expect("non-empty").v;
                if let Some(parent) = call_stack.last() {
                    let child_low = lowlink[&done_v];
                    let parent_low = lowlink.get_mut(&parent.v).expect("parent lowlink");
                    if child_low < *parent_low {
                        *parent_low = child_low;
                    }
                }
                if lowlink[&done_v] == indices[&done_v] {
                    let mut scc = Vec::new();
                    while let Some(w) = tarjan_stack.pop() {
                        on_stack.remove(&w);
                        scc.push(w);
                        if w == done_v {
                            break;
                        }
                    }
                    sccs.push(scc);
                }
            }
        }
    }
    sccs
}

fn emit_get_free_bin(fb: &mut FB, bin_idx: u32, out_head: u32) {
    for i in 0..ARC_FREE_BIN_COUNT {
        fb.g(bin_idx).i32c(i as i64).op(op::I32_EQ);
        fb.if_();
        fb.gget(ARC_FREE_BIN_BASE + i).s(out_head);
        fb.else_();
    }
    fb.op(op::UNREACHABLE);
    for _ in 0..ARC_FREE_BIN_COUNT {
        fb.end();
    }
}

fn emit_set_free_bin(fb: &mut FB, bin_idx: u32, new_head: u32) {
    for i in 0..ARC_FREE_BIN_COUNT {
        fb.g(bin_idx).i32c(i as i64).op(op::I32_EQ);
        fb.if_();
        fb.g(new_head).gset(ARC_FREE_BIN_BASE + i);
        fb.else_();
    }
    fb.op(op::UNREACHABLE);
    for _ in 0..ARC_FREE_BIN_COUNT {
        fb.end();
    }
}

/// Lowers `H::ArcAlloc`, `H::Retain`, or `H::Release` with cycle-collector support.
pub(crate) fn lower_arc_helper(env: &Env<'_>, h: H) -> Result<crate::LoweredFunc, CodegenError> {
    match h {
        H::ArcAlloc => Ok(lower_cycle_arc_alloc()),
        H::Retain => Ok(lower_cycle_retain()),
        H::Release => Ok(lower_cycle_release(env)),
        _ => Err(unsupported(
            "unexpected helper routed to cycle ARC lowerer",
            None,
        )),
    }
}

fn lower_cycle_arc_alloc() -> (Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>) {
    let mut fb = FB::new(2);
    let payload_size = 0u32;
    let drop_idx = 1u32;
    let total = fb.scratch(Val::I32);
    let exp = fb.scratch(Val::I32);
    let bin_idx = fb.scratch(Val::I32);
    let head = fb.scratch(Val::I32);
    let base = fb.scratch(Val::I32);
    let ptr = fb.scratch(Val::I32);
    let block_size = fb.scratch(Val::I32);
    let new_heap = fb.scratch(Val::I32);
    let cur_pages = fb.scratch(Val::I32);
    let req_pages = fb.scratch(Val::I32);

    fb.g(payload_size).i32c(ARC_HEADER).op(op::I32_ADD).s(total);
    fb.g(total).i32c(32).op(op::I32_LT_S);
    fb.if_();
    fb.i32c(32).s(total);
    fb.end();

    fb.i32c(32)
        .g(total)
        .i32c(1)
        .op(op::I32_SUB)
        .op(op::I32_CLZ)
        .op(op::I32_SUB)
        .s(exp);
    fb.g(exp).i32c(ARC_MAX_EXP).op(op::I32_GT_S);
    fb.if_();
    fb.op(op::UNREACHABLE);
    fb.end();

    fb.g(exp).i32c(ARC_MIN_EXP).op(op::I32_SUB).s(bin_idx);
    emit_get_free_bin(&mut fb, bin_idx, head);

    fb.g(head).i32c(0).op(op::I32_NE);
    fb.if_();
    {
        fb.g(head).s(ptr);
        fb.g(ptr).i32c(ARC_HEADER).op(op::I32_SUB).s(base);
        fb.g(ptr).load32(0).s(head);
        emit_set_free_bin(&mut fb, bin_idx, head);
        fb.i32c(1).g(exp).op(op::I32_SHL).s(block_size);
        fb.g(ptr)
            .i32c(0)
            .g(block_size)
            .i32c(ARC_HEADER)
            .op(op::I32_SUB)
            .memory_fill();
        fb.g(base).i64c(1).store64(0);
        fb.g(base).g(drop_idx).store32(8);
        fb.g(base).i32c(ARC_MAGIC).store32(16);
        fb.g(base).g(exp).store8(20);
        fb.g(base).i32c(0).store8(21);
        fb.g(base).i32c(0).store16(22);
    }
    fb.else_();
    {
        fb.gget(GLOBAL_HEAP)
            .i32c(15)
            .op(op::I32_ADD)
            .i32c(-16)
            .op(op::I32_AND)
            .s(base);
        fb.i32c(1).g(exp).op(op::I32_SHL).s(block_size);
        fb.g(base).g(block_size).op(op::I32_ADD).s(new_heap);
        fb.op(op::MEMORY_SIZE);
        fb.body.push(0x00);
        fb.s(cur_pages);
        fb.g(new_heap)
            .i32c(65535)
            .op(op::I32_ADD)
            .i32c(16)
            .op(op::I32_SHR_U)
            .s(req_pages);
        fb.g(req_pages).g(cur_pages).op(op::I32_GT_U);
        fb.if_();
        fb.g(req_pages).g(cur_pages).op(op::I32_SUB);
        fb.op(op::MEMORY_GROW);
        fb.body.push(0x00);
        fb.i32c(-1).op(op::I32_EQ);
        fb.if_();
        fb.op(op::UNREACHABLE);
        fb.end();
        fb.end();

        fb.g(new_heap).gset(GLOBAL_HEAP);
        fb.g(base).i32c(ARC_HEADER).op(op::I32_ADD).s(ptr);
        fb.g(base).i64c(1).store64(0);
        fb.g(base).g(drop_idx).store32(8);
        fb.g(base).i32c(1).store32(12);
        fb.g(base).i32c(ARC_MAGIC).store32(16);
        fb.g(base).g(exp).store8(20);
        fb.g(base).i32c(0).store8(21);
        fb.g(base).i32c(0).store16(22);
    }
    fb.end();

    fb.g(ptr);
    (vec![Val::I32, Val::I32], vec![Val::I32], fb.extras, fb.body)
}

fn lower_cycle_retain() -> (Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>) {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let base = fb.scratch(Val::I32);
    let rc = fb.scratch(Val::I64);

    fb.g(ptr).i32c(0).op(op::I32_EQ);
    fb.if_();
    fb.g(ptr).op(op::RETURN);
    fb.end();

    fb.g(ptr).i32c(ARC_HEADER).op(op::I32_SUB).s(base);
    fb.g(base).load64(0).s(rc);
    fb.g(rc).i64c(IMMORTAL_RC).op(op::I64_EQ);
    fb.if_();
    fb.g(ptr).op(op::RETURN);
    fb.end();

    fb.g(base).load32(16).i32c(ARC_MAGIC).op(op::I32_NE);
    fb.if_();
    fb.op(op::UNREACHABLE);
    fb.end();

    fb.g(rc).i64c(1).op(op::I64_LT_S);
    fb.if_();
    fb.op(op::UNREACHABLE);
    fb.end();

    fb.g(base).g(rc).i64c(1).op(op::I64_ADD).store64(0);
    fb.g(ptr);
    (vec![Val::I32], vec![Val::I32], fb.extras, fb.body)
}

fn lower_cycle_release(env: &Env<'_>) -> (Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>) {
    let note_root_idx = env
        .note_root_idx
        .expect("note_root_idx set when lowering cycle release");
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let base = fb.scratch(Val::I32);
    let rc = fb.scratch(Val::I64);
    let drop_idx = fb.scratch(Val::I32);
    let exp = fb.scratch(Val::I32);
    let bin_idx = fb.scratch(Val::I32);
    let old_head = fb.scratch(Val::I32);
    let flags = fb.scratch(Val::I32);

    fb.g(ptr).i32c(0).op(op::I32_EQ);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.g(ptr).i32c(ARC_HEADER).op(op::I32_SUB).s(base);
    fb.g(base).load64(0).s(rc);
    fb.g(rc).i64c(IMMORTAL_RC).op(op::I64_EQ);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    // Dead cycle nodes are invalidated (magic = 0) before running destructors,
    // so cyclic cross-member releases during cycle collection return cleanly.
    fb.g(base).load32(16).i32c(ARC_MAGIC).op(op::I32_NE);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.g(rc).i64c(1).op(op::I64_LT_S);
    fb.if_();
    fb.op(op::UNREACHABLE);
    fb.end();

    fb.g(rc).i64c(1).op(op::I64_GT_S);
    fb.if_();
    {
        fb.g(base).g(rc).i64c(1).op(op::I64_SUB).store64(0);
        fb.g(base).load8(21).s(flags);
        fb.g(flags).i32c(FLAG_COLLECTIBLE).op(op::I32_AND);
        fb.if_();
        {
            fb.g(flags)
                .i32c(FLAG_BUFFERED)
                .op(op::I32_AND)
                .op(op::I32_EQZ);
            fb.if_();
            fb.g(ptr);
            fb.call(note_root_idx);
            fb.end();
        }
        fb.end();
        fb.op(op::RETURN);
    }
    fb.end();

    // rc == 1 -> invalidate header, bump generation, run drop_idx, push to bin.
    fb.g(base).i64c(0).store64(0);
    fb.g(base).i32c(0).store32(16);
    fb.g(base)
        .g(base)
        .load32(12)
        .i32c(1)
        .op(op::I32_ADD)
        .store32(12);
    fb.g(base).i32c(0).store8(21);

    fb.g(base).load32(8).s(drop_idx);
    fb.g(drop_idx).i32c(0).op(op::I32_NE);
    fb.if_();
    fb.g(ptr);
    fb.g(drop_idx);
    fb.call_indirect(env.drop_call_type, 0);
    fb.end();

    fb.g(base).load8(20).s(exp);
    fb.g(exp).i32c(ARC_MIN_EXP).op(op::I32_SUB).s(bin_idx);
    emit_get_free_bin(&mut fb, bin_idx, old_head);
    fb.g(ptr).g(old_head).store32(0);
    emit_set_free_bin(&mut fb, bin_idx, ptr);

    (vec![Val::I32], vec![], fb.extras, fb.body)
}

/// Deterministic function indices for the WASM cycle-collector runtime.
pub(crate) struct CycleFnIndices {
    pub(crate) trace_visit: u32,
    pub(crate) mark_collectible: u32,
    pub(crate) ht_find: u32,
    pub(crate) sub_insert: u32,
    pub(crate) stack_push: u32,
    pub(crate) note_root: u32,
    pub(crate) trace_dispatch: u32,
    pub(crate) collect_round: u32,
    pub(crate) collect_cycles: u32,
    total_count: u32,
}

impl CycleFnIndices {
    pub(crate) fn compute(
        runtime_base_idx: u32,
        agg_count: usize,
        has_list: bool,
        closure_env_count: usize,
        has_closure_destroy: bool,
        has_task: bool,
        has_tuple: bool,
    ) -> Self {
        let trace_visit = runtime_base_idx;
        let mark_collectible = runtime_base_idx + 1;
        let ht_find = runtime_base_idx + 2;
        let sub_insert = runtime_base_idx + 3;
        let stack_push = runtime_base_idx + 4;
        let note_root = runtime_base_idx + 5;
        let mut cursor = runtime_base_idx + 6;
        cursor += agg_count as u32;
        if has_list {
            cursor += 1;
        }
        cursor += closure_env_count as u32;
        if has_closure_destroy {
            cursor += 1;
        }
        if has_task {
            cursor += 1;
        }
        if has_tuple {
            cursor += 1;
        }
        let trace_dispatch = cursor;
        let collect_round = cursor + 1;
        let collect_cycles = cursor + 2;
        let total_count = (cursor + 3) - runtime_base_idx;
        Self {
            trace_visit,
            mark_collectible,
            ht_find,
            sub_insert,
            stack_push,
            note_root,
            trace_dispatch,
            collect_round,
            collect_cycles,
            total_count,
        }
    }

    pub(crate) fn fn_count(&self) -> u32 {
        self.total_count
    }
}

/// Appends all cycle-collector runtime and trace functions in exact index order
/// matching `CycleFnIndices::compute`.
pub(crate) fn emit_cycle_runtime_and_traces(
    env: &Env<'_>,
    indices: &CycleFnIndices,
    aggregates: &[MirAggregateId],
    closures: &[MirFunctionId],
    base: u32,
    funcs: &mut Vec<Func>,
) -> Result<(), CodegenError> {
    let arc_alloc_idx = *env
        .helper_index
        .get(&H::ArcAlloc)
        .ok_or_else(|| unsupported("cycle collector requires H::ArcAlloc", None))?;
    let arc_release_idx = *env
        .helper_index
        .get(&H::Release)
        .ok_or_else(|| unsupported("cycle collector requires H::Release", None))?;

    funcs.push(emit_trace_visit(indices));
    funcs.push(emit_mark_collectible());
    funcs.push(emit_ht_find());
    funcs.push(emit_sub_insert(arc_alloc_idx, arc_release_idx));
    funcs.push(emit_stack_push(arc_alloc_idx, arc_release_idx));
    funcs.push(emit_note_root(
        arc_alloc_idx,
        arc_release_idx,
        indices.collect_cycles,
    ));

    let mut dispatch_arms: Vec<(u32, u32)> = Vec::new();
    for &agg_id in aggregates {
        let drop_slot = *env
            .drop_slot
            .get(&agg_id)
            .ok_or_else(|| unsupported("missing aggregate drop slot", None))?;
        let trace_fn_idx = base + funcs.len() as u32;
        funcs.push(emit_trace_agg(env, agg_id, indices.trace_visit)?);
        dispatch_arms.push((drop_slot, trace_fn_idx));
    }
    if let Some(list_slot) = env.list_drop_slot {
        let trace_fn_idx = base + funcs.len() as u32;
        funcs.push(emit_trace_list(indices.trace_visit));
        dispatch_arms.push((list_slot, trace_fn_idx));
    }
    for &closure_id in closures {
        let Some(&env_dtor_slot) = env.env_drop_slot.get(&closure_id) else {
            continue;
        };
        let trace_fn_idx = base + funcs.len() as u32;
        funcs.push(emit_trace_closure_env(env, closure_id, indices.trace_visit));
        dispatch_arms.push((env_dtor_slot, trace_fn_idx));
    }
    if let Some(destroy_slot) = env.closure_destroy_slot {
        let trace_fn_idx = base + funcs.len() as u32;
        funcs.push(emit_trace_closure_destroy(indices.trace_visit));
        dispatch_arms.push((destroy_slot, trace_fn_idx));
    }
    if let Some(task_slot) = env.task_drop_slot {
        let trace_fn_idx = base + funcs.len() as u32;
        funcs.push(emit_trace_task(indices.trace_visit));
        dispatch_arms.push((task_slot, trace_fn_idx));
    }
    if let Some(tuple_slot) = env.tuple_drop_slot {
        let trace_fn_idx = base + funcs.len() as u32;
        funcs.push(emit_trace_tuple(indices.trace_visit));
        dispatch_arms.push((tuple_slot, trace_fn_idx));
    }

    funcs.push(emit_trace_dispatch(&dispatch_arms));
    funcs.push(emit_collect_round(env, indices));
    funcs.push(emit_collect_cycles(indices.collect_round));

    Ok(())
}

fn emit_mark_collectible() -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let base = fb.scratch(Val::I32);
    let flags = fb.scratch(Val::I32);

    fb.g(ptr).i32c(0).op(op::I32_NE);
    fb.if_();
    {
        fb.g(ptr).i32c(ARC_HEADER).op(op::I32_SUB).s(base);
        fb.g(base).load64(0).i64c(IMMORTAL_RC).op(op::I64_NE);
        fb.if_();
        fb.g(base).load8(21).s(flags);
        fb.g(base)
            .g(flags)
            .i32c(FLAG_COLLECTIBLE)
            .op(op::I32_OR)
            .store8(21);
        fb.end();
    }
    fb.end();

    fb.g(ptr);
    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![Val::I32],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_ht_find() -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let cap = fb.scratch(Val::I32);
    let mask = fb.scratch(Val::I32);
    let slot = fb.scratch(Val::I32);
    let val = fb.scratch(Val::I32);
    let idx = fb.scratch(Val::I32);

    fb.gget(G_HT_CAP).s(cap);
    fb.g(cap).i32c(0).op(op::I32_EQ);
    fb.if_();
    fb.i32c(-1).op(op::RETURN);
    fb.end();

    fb.g(cap).i32c(1).op(op::I32_SUB).s(mask);
    fb.g(ptr)
        .i32c(3)
        .op(op::I32_SHR_U)
        .i32c(-1640531527)
        .op(op::I32_MUL)
        .g(mask)
        .op(op::I32_AND)
        .s(slot);

    fb.loop_();
    {
        fb.gget(G_HT_BUF)
            .g(slot)
            .i32c(2)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .load32(0)
            .s(val);
        fb.g(val).i32c(0).op(op::I32_EQ);
        fb.if_();
        fb.i32c(-1).op(op::RETURN);
        fb.end();

        fb.g(val).i32c(1).op(op::I32_SUB).s(idx);
        fb.gget(G_SUB_BUF)
            .g(idx)
            .i32c(4)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .load32(0)
            .g(ptr)
            .op(op::I32_EQ);
        fb.if_();
        fb.g(idx).op(op::RETURN);
        fb.end();

        fb.g(slot)
            .i32c(1)
            .op(op::I32_ADD)
            .g(mask)
            .op(op::I32_AND)
            .s(slot);
        fb.br(0);
    }
    fb.end();

    fb.i32c(-1);
    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![Val::I32],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_sub_insert(arc_alloc_idx: u32, arc_release_idx: u32) -> Func {
    let mut fb = FB::new(3);
    let ptr = 0u32;
    let scratch = 1u32;
    let is_live = 2u32;
    let idx = fb.scratch(Val::I32);
    let entry = fb.scratch(Val::I32);
    let new_cap = fb.scratch(Val::I32);
    let new_buf = fb.scratch(Val::I32);
    let old_buf = fb.scratch(Val::I32);
    let mask = fb.scratch(Val::I32);
    let slot = fb.scratch(Val::I32);
    let i = fb.scratch(Val::I32);
    let re_ptr = fb.scratch(Val::I32);

    fb.gget(G_SUB_LEN).gget(G_SUB_CAP).op(op::I32_EQ);
    fb.if_();
    {
        fb.gget(G_SUB_CAP).i32c(0).op(op::I32_EQ);
        fb.if_();
        fb.i32c(256).s(new_cap);
        fb.else_();
        fb.gget(G_SUB_CAP).i32c(1).op(op::I32_SHL).s(new_cap);
        fb.end();

        fb.g(new_cap).i32c(4).op(op::I32_SHL);
        fb.i32c(0);
        fb.call(arc_alloc_idx);
        fb.s(new_buf);

        fb.gget(G_SUB_LEN).i32c(0).op(op::I32_GT_S);
        fb.if_();
        fb.g(new_buf)
            .gget(G_SUB_BUF)
            .gget(G_SUB_LEN)
            .i32c(4)
            .op(op::I32_SHL)
            .memory_copy();
        fb.end();

        fb.gget(G_SUB_BUF).s(old_buf);
        fb.g(new_buf).gset(G_SUB_BUF);
        fb.g(new_cap).gset(G_SUB_CAP);
        fb.g(old_buf).i32c(0).op(op::I32_NE);
        fb.if_();
        fb.g(old_buf);
        fb.call(arc_release_idx);
        fb.end();
    }
    fb.end();

    fb.gget(G_SUB_LEN).s(idx);
    fb.g(idx).i32c(1).op(op::I32_ADD).gset(G_SUB_LEN);
    fb.gget(G_SUB_BUF)
        .g(idx)
        .i32c(4)
        .op(op::I32_SHL)
        .op(op::I32_ADD)
        .s(entry);
    fb.g(entry).g(ptr).store32(0);
    fb.g(entry).g(is_live).store32(4);
    fb.g(entry).g(scratch).store64(8);

    fb.gget(G_SUB_LEN)
        .i32c(1)
        .op(op::I32_SHL)
        .gget(G_HT_CAP)
        .op(op::I32_GT_S);
    fb.if_();
    {
        fb.gget(G_HT_CAP).i32c(0).op(op::I32_EQ);
        fb.if_();
        fb.i32c(512).s(new_cap);
        fb.else_();
        fb.gget(G_HT_CAP).i32c(1).op(op::I32_SHL).s(new_cap);
        fb.end();

        fb.g(new_cap).i32c(2).op(op::I32_SHL);
        fb.i32c(0);
        fb.call(arc_alloc_idx);
        fb.s(new_buf);
        fb.g(new_buf)
            .i32c(0)
            .g(new_cap)
            .i32c(2)
            .op(op::I32_SHL)
            .memory_fill();

        fb.gget(G_HT_BUF).s(old_buf);
        fb.g(new_buf).gset(G_HT_BUF);
        fb.g(new_cap).gset(G_HT_CAP);
        fb.g(old_buf).i32c(0).op(op::I32_NE);
        fb.if_();
        fb.g(old_buf);
        fb.call(arc_release_idx);
        fb.end();

        fb.gget(G_HT_CAP).i32c(1).op(op::I32_SUB).s(mask);
        fb.i32c(0).s(i);
        fb.block();
        fb.loop_();
        {
            fb.g(i).gget(G_SUB_LEN).op(op::I32_GE_S);
            fb.br_if(1);

            fb.gget(G_SUB_BUF)
                .g(i)
                .i32c(4)
                .op(op::I32_SHL)
                .op(op::I32_ADD)
                .load32(0)
                .s(re_ptr);
            fb.g(re_ptr)
                .i32c(3)
                .op(op::I32_SHR_U)
                .i32c(-1640531527)
                .op(op::I32_MUL)
                .g(mask)
                .op(op::I32_AND)
                .s(slot);

            fb.loop_();
            {
                fb.gget(G_HT_BUF)
                    .g(slot)
                    .i32c(2)
                    .op(op::I32_SHL)
                    .op(op::I32_ADD)
                    .load32(0)
                    .i32c(0)
                    .op(op::I32_NE);
                fb.if_();
                fb.g(slot)
                    .i32c(1)
                    .op(op::I32_ADD)
                    .g(mask)
                    .op(op::I32_AND)
                    .s(slot);
                fb.br(1);
                fb.end();
            }
            fb.end();

            fb.gget(G_HT_BUF)
                .g(slot)
                .i32c(2)
                .op(op::I32_SHL)
                .op(op::I32_ADD)
                .g(i)
                .i32c(1)
                .op(op::I32_ADD)
                .store32(0);

            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0);
        }
        fb.end();
        fb.end();
    }
    fb.else_();
    {
        fb.gget(G_HT_CAP).i32c(1).op(op::I32_SUB).s(mask);
        fb.g(ptr)
            .i32c(3)
            .op(op::I32_SHR_U)
            .i32c(-1640531527)
            .op(op::I32_MUL)
            .g(mask)
            .op(op::I32_AND)
            .s(slot);

        fb.loop_();
        {
            fb.gget(G_HT_BUF)
                .g(slot)
                .i32c(2)
                .op(op::I32_SHL)
                .op(op::I32_ADD)
                .load32(0)
                .i32c(0)
                .op(op::I32_NE);
            fb.if_();
            fb.g(slot)
                .i32c(1)
                .op(op::I32_ADD)
                .g(mask)
                .op(op::I32_AND)
                .s(slot);
            fb.br(1);
            fb.end();
        }
        fb.end();

        fb.gget(G_HT_BUF)
            .g(slot)
            .i32c(2)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .g(idx)
            .i32c(1)
            .op(op::I32_ADD)
            .store32(0);
    }
    fb.end();

    fb.g(idx);
    Func {
        name: None,
        params: vec![Val::I32, Val::I64, Val::I32],
        results: vec![Val::I32],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_stack_push(arc_alloc_idx: u32, arc_release_idx: u32) -> Func {
    let mut fb = FB::new(1);
    let idx = 0u32;
    let new_cap = fb.scratch(Val::I32);
    let new_buf = fb.scratch(Val::I32);
    let old_buf = fb.scratch(Val::I32);

    fb.gget(G_STACK_LEN).gget(G_STACK_CAP).op(op::I32_EQ);
    fb.if_();
    {
        fb.gget(G_STACK_CAP).i32c(0).op(op::I32_EQ);
        fb.if_();
        fb.i32c(256).s(new_cap);
        fb.else_();
        fb.gget(G_STACK_CAP).i32c(1).op(op::I32_SHL).s(new_cap);
        fb.end();

        fb.g(new_cap).i32c(2).op(op::I32_SHL);
        fb.i32c(0);
        fb.call(arc_alloc_idx);
        fb.s(new_buf);

        fb.gget(G_STACK_LEN).i32c(0).op(op::I32_GT_S);
        fb.if_();
        fb.g(new_buf)
            .gget(G_STACK_BUF)
            .gget(G_STACK_LEN)
            .i32c(2)
            .op(op::I32_SHL)
            .memory_copy();
        fb.end();

        fb.gget(G_STACK_BUF).s(old_buf);
        fb.g(new_buf).gset(G_STACK_BUF);
        fb.g(new_cap).gset(G_STACK_CAP);
        fb.g(old_buf).i32c(0).op(op::I32_NE);
        fb.if_();
        fb.g(old_buf);
        fb.call(arc_release_idx);
        fb.end();
    }
    fb.end();

    fb.gget(G_STACK_BUF)
        .gget(G_STACK_LEN)
        .i32c(2)
        .op(op::I32_SHL)
        .op(op::I32_ADD)
        .g(idx)
        .store32(0);
    fb.gget(G_STACK_LEN)
        .i32c(1)
        .op(op::I32_ADD)
        .gset(G_STACK_LEN);

    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_note_root(arc_alloc_idx: u32, arc_release_idx: u32, collect_cycles_idx: u32) -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let base = fb.scratch(Val::I32);
    let flags = fb.scratch(Val::I32);
    let new_cap = fb.scratch(Val::I32);
    let new_buf = fb.scratch(Val::I32);
    let old_buf = fb.scratch(Val::I32);
    let entry = fb.scratch(Val::I32);

    fb.g(ptr).i32c(ARC_HEADER).op(op::I32_SUB).s(base);
    fb.g(base).load8(21).s(flags);
    fb.g(flags).i32c(FLAG_BUFFERED).op(op::I32_AND);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();
    fb.g(base)
        .g(flags)
        .i32c(FLAG_BUFFERED)
        .op(op::I32_OR)
        .store8(21);

    fb.gget(G_ROOT_LEN).gget(G_ROOT_CAP).op(op::I32_EQ);
    fb.if_();
    {
        fb.gget(G_ROOT_CAP).i32c(0).op(op::I32_EQ);
        fb.if_();
        fb.i32c(128).s(new_cap);
        fb.else_();
        fb.gget(G_ROOT_CAP).i32c(1).op(op::I32_SHL).s(new_cap);
        fb.end();

        fb.g(new_cap).i32c(3).op(op::I32_SHL);
        fb.i32c(0);
        fb.call(arc_alloc_idx);
        fb.s(new_buf);

        fb.gget(G_ROOT_LEN).i32c(0).op(op::I32_GT_S);
        fb.if_();
        fb.g(new_buf)
            .gget(G_ROOT_BUF)
            .gget(G_ROOT_LEN)
            .i32c(3)
            .op(op::I32_SHL)
            .memory_copy();
        fb.end();

        fb.gget(G_ROOT_BUF).s(old_buf);
        fb.g(new_buf).gset(G_ROOT_BUF);
        fb.g(new_cap).gset(G_ROOT_CAP);
        fb.g(old_buf).i32c(0).op(op::I32_NE);
        fb.if_();
        fb.g(old_buf);
        fb.call(arc_release_idx);
        fb.end();
    }
    fb.end();

    fb.gget(G_ROOT_BUF)
        .gget(G_ROOT_LEN)
        .i32c(3)
        .op(op::I32_SHL)
        .op(op::I32_ADD)
        .s(entry);
    fb.g(entry).g(ptr).store32(0);
    fb.g(entry).g(base).load32(12).store32(4);
    fb.gget(G_ROOT_LEN).i32c(1).op(op::I32_ADD).gset(G_ROOT_LEN);

    fb.gget(G_COLLECTING).i32c(0).op(op::I32_EQ);
    fb.if_();
    {
        fb.gget(G_ROOT_LEN).i32c(COLLECT_THRESHOLD).op(op::I32_GE_S);
        fb.if_();
        fb.call(collect_cycles_idx);
        fb.end();
    }
    fb.end();

    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_trace_visit(indices: &CycleFnIndices) -> Func {
    let mut fb = FB::new(1);
    let child = 0u32;
    let base = fb.scratch(Val::I32);
    let rc = fb.scratch(Val::I64);
    let idx = fb.scratch(Val::I32);
    let entry = fb.scratch(Val::I32);

    fb.g(child).i32c(0).op(op::I32_EQ);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.g(child).i32c(ARC_HEADER).op(op::I32_SUB).s(base);
    fb.g(base).load32(16).i32c(ARC_MAGIC).op(op::I32_NE);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.g(base).load64(0).s(rc);
    fb.g(rc).i64c(IMMORTAL_RC).op(op::I64_EQ);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.g(base)
        .load8(21)
        .i32c(FLAG_COLLECTIBLE)
        .op(op::I32_AND)
        .op(op::I32_EQZ);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.g(child);
    fb.call(indices.ht_find);
    fb.s(idx);

    fb.gget(G_TRACE_PHASE).i32c(0).op(op::I32_EQ);
    fb.if_();
    {
        fb.g(idx).i32c(0).op(op::I32_LT_S);
        fb.if_();
        {
            fb.g(child);
            fb.g(rc);
            fb.i32c(0);
            fb.call(indices.sub_insert);
            fb.s(idx);
            fb.g(idx);
            fb.call(indices.stack_push);
        }
        fb.end();

        fb.gget(G_SUB_BUF)
            .g(idx)
            .i32c(4)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .s(entry);
        fb.g(entry)
            .g(entry)
            .load64(8)
            .i64c(1)
            .op(op::I64_SUB)
            .store64(8);
    }
    fb.else_();
    {
        fb.g(idx).i32c(0).op(op::I32_GE_S);
        fb.if_();
        {
            fb.gget(G_SUB_BUF)
                .g(idx)
                .i32c(4)
                .op(op::I32_SHL)
                .op(op::I32_ADD)
                .s(entry);
            fb.g(entry).load32(4).i32c(0).op(op::I32_EQ);
            fb.if_();
            fb.g(entry).i32c(1).store32(4);
            fb.g(idx);
            fb.call(indices.stack_push);
            fb.end();
        }
        fb.end();
    }
    fb.end();

    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_trace_agg(
    env: &Env<'_>,
    agg_id: MirAggregateId,
    trace_visit_idx: u32,
) -> Result<Func, CodegenError> {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let tag = fb.scratch(Val::I32);
    let layout = env
        .layouts
        .get(&agg_id)
        .ok_or_else(|| unsupported("missing aggregate layout in trace generator", None))?;

    match layout.kind {
        MirAggregateKind::Struct => {
            for slot in &layout.struct_fields {
                if is_managed(env.types, slot.ty) {
                    fb.g(ptr).load32(slot.offset);
                    fb.call(trace_visit_idx);
                }
            }
        }
        MirAggregateKind::Enum => {
            fb.g(ptr).load32(0).s(tag);
            for (ordinal, fields) in layout.variant_fields.iter().enumerate() {
                let any_managed = fields.iter().any(|slot| is_managed(env.types, slot.ty));
                if !any_managed {
                    continue;
                }
                fb.g(tag).i32c(ordinal as i64).op(op::I32_EQ);
                fb.if_();
                for slot in fields {
                    if is_managed(env.types, slot.ty) {
                        fb.g(ptr).load32(slot.offset);
                        fb.call(trace_visit_idx);
                    }
                }
                fb.end();
            }
        }
    }

    Ok(Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    })
}

fn emit_trace_list(trace_visit_idx: u32) -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let len = fb.scratch(Val::I32);
    let data = fb.scratch(Val::I32);
    let i = fb.scratch(Val::I32);

    fb.g(ptr).load64(LIST_IS_ARC as u32).op(op::I64_EQZ);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.g(ptr)
        .load64(LIST_DATA as u32)
        .op(op::I32_WRAP_I64)
        .s(data);
    fb.g(data).i32c(0).op(op::I32_EQ);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.g(ptr)
        .load64(LIST_LEN as u32)
        .op(op::I32_WRAP_I64)
        .s(len);
    fb.i32c(0).s(i);
    fb.block();
    fb.loop_();
    {
        fb.g(i).g(len).op(op::I32_GE_S);
        fb.br_if(1);

        fb.g(data)
            .g(i)
            .i32c(3)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .load64(0)
            .op(op::I32_WRAP_I64);
        fb.call(trace_visit_idx);

        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
    }
    fb.end();
    fb.end();

    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_trace_closure_env(env: &Env<'_>, closure_id: MirFunctionId, trace_visit_idx: u32) -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let function = env.program.function(closure_id).unwrap();
    let capture_frame: Vec<MirLocalId> = env
        .program
        .function_parameters(function)
        .iter()
        .copied()
        .filter(|&p| matches!(env.program.local(p).unwrap().kind, MirLocalKind::Capture))
        .collect();
    for (i, &slot) in capture_frame.iter().enumerate() {
        let ty = env.program.local(slot).unwrap().ty;
        if is_managed(env.types, ty) {
            fb.g(ptr).load32((8 * i) as u32);
            fb.call(trace_visit_idx);
        }
    }
    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_trace_closure_destroy(trace_visit_idx: u32) -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    fb.g(ptr).load32(8);
    fb.call(trace_visit_idx);
    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_trace_task(trace_visit_idx: u32) -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    fb.g(ptr).load64(8).op(op::I32_WRAP_I64);
    fb.call(trace_visit_idx);

    fb.g(ptr).load32(24).i32c(2).op(op::I32_EQ);
    fb.g(ptr).load32(28).i32c(1).op(op::I32_EQ);
    fb.op(op::I32_AND);
    fb.if_();
    {
        fb.g(ptr).load64(16).op(op::I32_WRAP_I64);
        fb.call(trace_visit_idx);
    }
    fb.end();

    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_trace_tuple(trace_visit_idx: u32) -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let mask = fb.scratch(Val::I64);
    let i = fb.scratch(Val::I32);

    // Match `lpp_drop_tuple`: offset 0 stores the 64-bit managed-slot bitmask
    // where bit `i` marks a managed child pointer at byte offset `16 + 8*i`.
    fb.g(ptr).load64(0).s(mask);
    fb.i32c(0).s(i);
    fb.block();
    fb.loop_();
    {
        fb.g(mask).op(op::I64_EQZ);
        fb.br_if(1);
        fb.g(i).i32c(64).op(op::I32_GE_S);
        fb.br_if(1);

        fb.g(mask).i64c(1).op(op::I64_AND).i64c(0).op(op::I64_NE);
        fb.if_();
        {
            fb.g(ptr)
                .i32c(16)
                .g(i)
                .i32c(3)
                .op(op::I32_SHL)
                .op(op::I32_ADD)
                .op(op::I32_ADD)
                .load64(0)
                .op(op::I32_WRAP_I64);
            fb.call(trace_visit_idx);
        }
        fb.end();

        fb.g(mask).i64c(1).op(op::I64_SHR_U).s(mask);
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
    }
    fb.end();
    fb.end();

    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_trace_dispatch(arms: &[(u32, u32)]) -> Func {
    let mut fb = FB::new(1);
    let ptr = 0u32;
    let drop_idx = fb.scratch(Val::I32);

    fb.g(ptr)
        .i32c(ARC_HEADER)
        .op(op::I32_SUB)
        .load32(8)
        .s(drop_idx);
    for &(slot, trace_fn_idx) in arms {
        fb.g(drop_idx).i32c(slot as i64).op(op::I32_EQ);
        fb.if_();
        fb.g(ptr);
        fb.call(trace_fn_idx);
        fb.op(op::RETURN);
        fb.end();
    }

    Func {
        name: None,
        params: vec![Val::I32],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_collect_round(env: &Env<'_>, indices: &CycleFnIndices) -> Func {
    let mut fb = FB::new(0);
    let i = fb.scratch(Val::I32);
    let entry = fb.scratch(Val::I32);
    let ptr = fb.scratch(Val::I32);
    let generation = fb.scratch(Val::I32);
    let base = fb.scratch(Val::I32);
    let rc = fb.scratch(Val::I64);
    let flags = fb.scratch(Val::I32);
    let idx = fb.scratch(Val::I32);
    let dead_count = fb.scratch(Val::I32);
    let drop_idx = fb.scratch(Val::I32);
    let exp = fb.scratch(Val::I32);
    let bin_idx = fb.scratch(Val::I32);
    let old_head = fb.scratch(Val::I32);

    fb.i32c(0).gset(G_SUB_LEN);
    fb.i32c(0).gset(G_STACK_LEN);
    fb.gget(G_HT_CAP).i32c(0).op(op::I32_GT_S);
    fb.if_();
    fb.gget(G_HT_BUF)
        .i32c(0)
        .gget(G_HT_CAP)
        .i32c(2)
        .op(op::I32_SHL)
        .memory_fill();
    fb.end();

    // Phase 1a: Seed valid buffered roots into sub_nodes + stack.
    fb.i32c(0).s(i);
    fb.block();
    fb.loop_();
    {
        fb.g(i).gget(G_ROOT_LEN).op(op::I32_GE_S);
        fb.br_if(1);

        fb.gget(G_ROOT_BUF)
            .g(i)
            .i32c(3)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .s(entry);
        fb.g(entry).load32(0).s(ptr);
        fb.g(entry).load32(4).s(generation);

        fb.block();
        {
            fb.g(ptr).i32c(0).op(op::I32_EQ);
            fb.br_if(0);
            fb.g(ptr).i32c(ARC_HEADER).op(op::I32_SUB).s(base);
            fb.g(base).load32(16).i32c(ARC_MAGIC).op(op::I32_NE);
            fb.br_if(0);
            fb.g(base).load32(12).g(generation).op(op::I32_NE);
            fb.br_if(0);
            fb.g(base).load64(0).s(rc);
            fb.g(rc).i64c(IMMORTAL_RC).op(op::I64_EQ);
            fb.br_if(0);
            fb.g(rc).i64c(0).op(op::I64_LE_S);
            fb.br_if(0);

            fb.g(base).load8(21).s(flags);
            fb.g(flags)
                .i32c(FLAG_COLLECTIBLE)
                .op(op::I32_AND)
                .op(op::I32_EQZ);
            fb.br_if(0);

            fb.g(base)
                .g(flags)
                .i32c(!FLAG_BUFFERED)
                .op(op::I32_AND)
                .store8(21);

            fb.g(ptr);
            fb.call(indices.ht_find);
            fb.i32c(0).op(op::I32_LT_S);
            fb.if_();
            {
                fb.g(ptr);
                fb.g(rc);
                fb.i32c(0);
                fb.call(indices.sub_insert);
                fb.call(indices.stack_push);
            }
            fb.end();
        }
        fb.end();

        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
    }
    fb.end();
    fb.end();

    fb.i32c(0).gset(G_ROOT_LEN);
    fb.gget(G_SUB_LEN).i32c(0).op(op::I32_EQ);
    fb.if_();
    fb.i32c(0).op(op::RETURN);
    fb.end();

    // Phase 1b: MarkGray DFS (trace_phase = 0).
    fb.i32c(0).gset(G_TRACE_PHASE);
    fb.block();
    fb.loop_();
    {
        fb.gget(G_STACK_LEN).i32c(0).op(op::I32_LE_S);
        fb.br_if(1);

        fb.gget(G_STACK_LEN)
            .i32c(1)
            .op(op::I32_SUB)
            .gset(G_STACK_LEN);
        fb.gget(G_STACK_BUF)
            .gget(G_STACK_LEN)
            .i32c(2)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .load32(0)
            .s(idx);
        fb.gget(G_SUB_BUF)
            .g(idx)
            .i32c(4)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .load32(0);
        fb.call(indices.trace_dispatch);
        fb.br(0);
    }
    fb.end();
    fb.end();

    // Phase 2 & 3: Scan external roots (scratch > 0) and ScanBlack descendants.
    fb.i32c(1).gset(G_TRACE_PHASE);
    fb.i32c(0).s(i);
    fb.block();
    fb.loop_();
    {
        fb.g(i).gget(G_SUB_LEN).op(op::I32_GE_S);
        fb.br_if(1);

        fb.gget(G_SUB_BUF)
            .g(i)
            .i32c(4)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .s(entry);
        fb.g(entry).load64(8).i64c(0).op(op::I64_GT_S);
        fb.if_();
        {
            fb.g(entry).load32(4).i32c(0).op(op::I32_EQ);
            fb.if_();
            fb.g(entry).i32c(1).store32(4);
            fb.g(i);
            fb.call(indices.stack_push);
            fb.end();
        }
        fb.end();

        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
    }
    fb.end();
    fb.end();

    fb.block();
    fb.loop_();
    {
        fb.gget(G_STACK_LEN).i32c(0).op(op::I32_LE_S);
        fb.br_if(1);

        fb.gget(G_STACK_LEN)
            .i32c(1)
            .op(op::I32_SUB)
            .gset(G_STACK_LEN);
        fb.gget(G_STACK_BUF)
            .gget(G_STACK_LEN)
            .i32c(2)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .load32(0)
            .s(idx);
        fb.gget(G_SUB_BUF)
            .g(idx)
            .i32c(4)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .load32(0);
        fb.call(indices.trace_dispatch);
        fb.br(0);
    }
    fb.end();
    fb.end();

    // Phase 4: Invalidate headers of all dead nodes FIRST.
    fb.i32c(0).s(dead_count);
    fb.i32c(0).s(i);
    fb.block();
    fb.loop_();
    {
        fb.g(i).gget(G_SUB_LEN).op(op::I32_GE_S);
        fb.br_if(1);

        fb.gget(G_SUB_BUF)
            .g(i)
            .i32c(4)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .s(entry);
        fb.g(entry).load32(4).i32c(0).op(op::I32_EQ);
        fb.if_();
        {
            fb.g(dead_count).i32c(1).op(op::I32_ADD).s(dead_count);
            fb.g(entry).load32(0).s(ptr);
            fb.g(ptr).i32c(ARC_HEADER).op(op::I32_SUB).s(base);
            fb.g(base).i64c(0).store64(0);
            fb.g(base).i32c(0).store32(16);
            fb.g(base)
                .g(base)
                .load32(12)
                .i32c(1)
                .op(op::I32_ADD)
                .store32(12);
            fb.g(base).i32c(0).store8(21);
        }
        fb.end();

        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
    }
    fb.end();
    fb.end();

    fb.g(dead_count).i32c(0).op(op::I32_EQ);
    fb.if_();
    fb.i32c(0).op(op::RETURN);
    fb.end();

    // Phase 5a: Run destructors on dead nodes.
    fb.i32c(0).s(i);
    fb.block();
    fb.loop_();
    {
        fb.g(i).gget(G_SUB_LEN).op(op::I32_GE_S);
        fb.br_if(1);

        fb.gget(G_SUB_BUF)
            .g(i)
            .i32c(4)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .s(entry);
        fb.g(entry).load32(4).i32c(0).op(op::I32_EQ);
        fb.if_();
        {
            fb.g(entry).load32(0).s(ptr);
            fb.g(ptr)
                .i32c(ARC_HEADER)
                .op(op::I32_SUB)
                .load32(8)
                .s(drop_idx);
            fb.g(drop_idx).i32c(0).op(op::I32_NE);
            fb.if_();
            fb.g(ptr);
            fb.g(drop_idx);
            fb.call_indirect(env.drop_call_type, 0);
            fb.end();
        }
        fb.end();

        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
    }
    fb.end();
    fb.end();

    // Phase 5b: Push all dead blocks onto size-class free bins.
    fb.i32c(0).s(i);
    fb.block();
    fb.loop_();
    {
        fb.g(i).gget(G_SUB_LEN).op(op::I32_GE_S);
        fb.br_if(1);

        fb.gget(G_SUB_BUF)
            .g(i)
            .i32c(4)
            .op(op::I32_SHL)
            .op(op::I32_ADD)
            .s(entry);
        fb.g(entry).load32(4).i32c(0).op(op::I32_EQ);
        fb.if_();
        {
            fb.g(entry).load32(0).s(ptr);
            fb.g(ptr).i32c(ARC_HEADER).op(op::I32_SUB).load8(20).s(exp);
            fb.g(exp).i32c(ARC_MIN_EXP).op(op::I32_SUB).s(bin_idx);
            emit_get_free_bin(&mut fb, bin_idx, old_head);
            fb.g(ptr).g(old_head).store32(0);
            emit_set_free_bin(&mut fb, bin_idx, ptr);
        }
        fb.end();

        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
    }
    fb.end();
    fb.end();

    fb.g(dead_count);
    Func {
        name: None,
        params: vec![],
        results: vec![Val::I32],
        extra: fb.extras,
        body: fb.body,
    }
}

fn emit_collect_cycles(collect_round_idx: u32) -> Func {
    let mut fb = FB::new(0);
    let rounds = fb.scratch(Val::I32);
    let collected = fb.scratch(Val::I32);

    fb.gget(G_COLLECTING).i32c(0).op(op::I32_NE);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.gget(G_ROOT_LEN).i32c(0).op(op::I32_EQ);
    fb.if_();
    fb.op(op::RETURN);
    fb.end();

    fb.i32c(1).gset(G_COLLECTING);
    fb.i32c(0).s(rounds);

    fb.block();
    fb.loop_();
    {
        fb.g(rounds).i32c(MAX_COLLECT_ROUNDS).op(op::I32_GE_S);
        fb.br_if(1);
        fb.gget(G_ROOT_LEN).i32c(0).op(op::I32_LE_S);
        fb.br_if(1);

        fb.call(collect_round_idx);
        fb.s(collected);
        fb.g(collected).i32c(0).op(op::I32_EQ);
        fb.if_();
        fb.gget(G_ROOT_LEN).i32c(0).op(op::I32_EQ);
        fb.br_if(2);
        fb.end();

        fb.g(rounds).i32c(1).op(op::I32_ADD).s(rounds);
        fb.br(0);
    }
    fb.end();
    fb.end();

    fb.i32c(0).gset(G_COLLECTING);
    Func {
        name: None,
        params: vec![],
        results: vec![],
        extra: fb.extras,
        body: fb.body,
    }
}

/// Emits a `mark_collectible` pass-through call when `has_cycles` and `ty` is
/// cycle-capable, leaving the pointer on the WASM value stack.
pub(crate) fn maybe_mark_collectible(env: &Env<'_>, fb: &mut FB, ty: TypeId) {
    if let Some(mark_idx) = env.mark_collectible_idx
        && env.shared_types.contains(&ty)
    {
        fb.call(mark_idx);
    }
}

/// Emits a `mark_collectible` pass-through call when any closure (`Function`)
/// type in the module is cycle-capable.
pub(crate) fn maybe_mark_closure_collectible(env: &Env<'_>, fb: &mut FB) {
    if let Some(mark_idx) = env.mark_collectible_idx
        && env
            .shared_types
            .iter()
            .any(|&t| matches!(env.types.kind(t), TypeKind::Function { .. }))
    {
        fb.call(mark_idx);
    }
}

/// Emits a `mark_collectible` pass-through call when any `Task` type in the
/// module is cycle-capable.
pub(crate) fn maybe_mark_task_collectible(env: &Env<'_>, fb: &mut FB) {
    if let Some(mark_idx) = env.mark_collectible_idx
        && env
            .shared_types
            .iter()
            .any(|&t| matches!(env.types.kind(t), TypeKind::Task(_)))
    {
        fb.call(mark_idx);
    }
}
