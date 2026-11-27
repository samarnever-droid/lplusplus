//! Phase 4E: ownership-balance verification.
//!
//! A deterministic token dataflow over each function's CFG that proves,
//! statically, that every heap-value cell is consumed at most once and
//! that every transfer obeys the consume-vs-borrow call contract the
//! interpreter executes. The runtime's end-of-execution proof is the
//! backstop; the static checker reports first at compile time.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use lpp_hir::OriginId;
use lpp_mir::{
    BasicBlockId, InstructionId, InstructionKind, MirFunctionId, MirFunctionKind, MirInvariant,
    MirLocalId, MirPlace, MirProgram, Operand, PlaceProjection, Rvalue, Terminator,
};
use lpp_passes::{MirPass, PassContext, PassFailure};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind};

use crate::plan::{OwnershipPlan, compute_ownership_plan};

/// Whether the type can name a heap value (a runtime heap node, or an
/// inline tuple that can own one).
fn is_heap_type(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(
        types.kind(ty),
        TypeKind::Function { .. }
            | TypeKind::List(_)
            | TypeKind::Slice(_)
            | TypeKind::Task(_)
            | TypeKind::Tuple(_)
            | TypeKind::Map { .. }
            | TypeKind::Nominal { .. }
            | TypeKind::Primitive(PrimitiveType::String)
    )
}

/// Whether a builtin moves (consumes) its positional value argument.
/// Mirrors the interpreter's `builtin_moves_value` exactly: the same
/// call contract, checked twice.
fn builtin_moves_value(builtin: BuiltinId, position: usize) -> bool {
    let name = builtin
        .descriptor()
        .name
        .strip_prefix("lpp_")
        .unwrap_or(builtin.descriptor().name);
    matches!(
        (name, position),
        ("list_push", 1) | ("list_set", 2) | ("print", 0) | ("write_str", 0)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    /// The cell holds its reference on every path through the point.
    Alive,
    /// The cell was moved out on every path through the point.
    Dead,
    /// The cell's fate is path-dependent.
    MayDead,
}

impl Token {
    fn meet(self, other: Token) -> Token {
        match (self, other) {
            (Self::Alive, Self::Alive) => Self::Alive,
            (Self::Dead, Self::Dead) => Self::Dead,
            _ => Self::MayDead,
        }
    }
}

/// A balance proof failure (code `E4403`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipBalanceErrorKind {
    /// A heap-value cell is consumed or read after being moved out.
    UseAfterMove {
        function: MirFunctionId,
        block: BasicBlockId,
        local: MirLocalId,
    },
    /// A heap-value local has no plan cell; validated, planned MIR
    /// never triggers this.
    MissingCell {
        function: MirFunctionId,
        local: MirLocalId,
    },
    /// A mutable field or slot write targets a concurrently-shared,
    /// non-unique reference without holding an exclusive lock.
    UnsynchronizedConcurrentWrite {
        function: MirFunctionId,
        block: BasicBlockId,
        local: MirLocalId,
    },
    /// A non-`Send` value (such as `Ptr[T]`, `RawPtr`, `Slice[T]`, `StrSlice`,
    /// or an aggregate/container transitively containing one) is transferred
    /// across a `spawn`, `thread_spawn`, or `async` concurrency boundary.
    NonSendTransfer {
        function: MirFunctionId,
        block: BasicBlockId,
        local: MirLocalId,
        ty: TypeId,
    },
}

/// A balance proof failure (code `E4403`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipBalanceError {
    pub kind: OwnershipBalanceErrorKind,
    /// The offending use site for `UseAfterMove`; the local declaration for
    /// structural `MissingCell` errors.
    pub origin: Option<OriginId>,
}

impl OwnershipBalanceError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "E4403"
    }
}

impl std::fmt::Display for OwnershipBalanceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {:?}", self.code(), self.kind)
    }
}

impl std::error::Error for OwnershipBalanceError {}

type State = BTreeMap<MirLocalId, Token>;

/// Whether the cell is not provably alive at this point (absent, dead,
/// or path-dependent) — a consume or read here is a use-after-move.
fn not_alive(state: &State, heap_locals: &BTreeSet<MirLocalId>, local: MirLocalId) -> bool {
    if !heap_locals.contains(&local) {
        return false;
    }
    matches!(
        state.get(&local),
        None | Some(Token::Dead) | Some(Token::MayDead)
    )
}

/// Prove the ownership balance of a planned program. For every
/// heap-value cell the checker tracks one token through the CFG:
/// definitions and borrows keep it alive, moves kill it, and any
/// consume or read of a dead (or path-dependent) cell is
/// `UseAfterMove`. The dataflow is a worklist fixed point over
/// block-ascending successors with a bounded iteration count, so two
/// runs agree byte for byte.
pub fn verify_ownership_balance(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &OwnershipPlan,
) -> Vec<OwnershipBalanceError> {
    let planned: BTreeSet<(MirFunctionId, MirLocalId)> = plan
        .cells
        .iter()
        .map(|cell| (cell.function, cell.local))
        .collect();

    let mut result: Vec<OwnershipBalanceError> = Vec::new();
    for (function_id, function) in program.functions() {
        let locals = program.function_locals(function).to_vec();
        let heap_locals: BTreeSet<MirLocalId> = locals
            .iter()
            .copied()
            .filter(|local| {
                program
                    .local(*local)
                    .is_some_and(|local| is_heap_type(types, local.ty))
            })
            .collect();
        if heap_locals.is_empty() {
            continue;
        }

        let mut violations: BTreeMap<(BasicBlockId, MirLocalId), OriginId> = BTreeMap::new();
        let mut worklist: Vec<BasicBlockId> = Vec::new();
        let mut queued: BTreeSet<BasicBlockId> = BTreeSet::new();

        // Structural guard: every heap-value local is planned.
        for local in &heap_locals {
            if !planned.contains(&(function_id, *local)) {
                result.push(OwnershipBalanceError {
                    kind: OwnershipBalanceErrorKind::MissingCell {
                        function: function_id,
                        local: *local,
                    },
                    origin: program.local(*local).map(|local| local.origin),
                });
            }
        }

        let mut initial: State = BTreeMap::new();
        for parameter in program.function_parameters(function) {
            if heap_locals.contains(parameter) {
                initial.insert(*parameter, Token::Alive);
            }
        }

        let mut in_states: BTreeMap<BasicBlockId, State> = BTreeMap::new();
        let mut out_states: BTreeMap<BasicBlockId, State> = BTreeMap::new();
        in_states.insert(function.entry, initial);
        worklist.push(function.entry);
        queued.insert(function.entry);
        let bound = 64 + program.function_blocks(function).len().saturating_mul(4);
        for _ in 0..bound {
            let Some(block_id) = worklist.pop() else {
                break;
            };
            queued.remove(&block_id);
            let in_state = in_states.get(&block_id).cloned().unwrap_or_default();
            let (out_state, block_violations) =
                apply_block(program, block_id, &in_state, &heap_locals);
            for (key, origin) in block_violations {
                violations.entry(key).or_insert(origin);
            }
            if out_states.get(&block_id) == Some(&out_state) {
                // The transfer is stable for this in-state.
                continue;
            }
            out_states.insert(block_id, out_state.clone());
            for target in successors(program, block_id) {
                // A block with no in-state yet is bottom: the first
                // reach installs the incoming state, later reaches
                // meet it in.
                let changed = match in_states.get(&target) {
                    None => {
                        in_states.insert(target, out_state.clone());
                        true
                    }
                    Some(previous) => {
                        let mut merged: State = BTreeMap::new();
                        let keys: BTreeSet<MirLocalId> =
                            previous.keys().chain(out_state.keys()).copied().collect();
                        for key in keys {
                            let left = previous.get(&key).copied().unwrap_or(Token::Dead);
                            let right = out_state.get(&key).copied().unwrap_or(Token::Dead);
                            let met = left.meet(right);
                            if met != Token::Dead {
                                merged.insert(key, met);
                            }
                        }
                        if *previous == merged {
                            false
                        } else {
                            in_states.insert(target, merged);
                            true
                        }
                    }
                };
                if changed && queued.insert(target) {
                    worklist.push(target);
                }
            }
        }
        for ((block, local), origin) in violations {
            result.push(OwnershipBalanceError {
                kind: OwnershipBalanceErrorKind::UseAfterMove {
                    function: function_id,
                    block,
                    local,
                },
                origin: Some(origin),
            });
        }
    }
    result.extend(verify_concurrent_writes(program, types));
    result.extend(verify_send_sync_boundaries(program, types));
    result
}

fn program_has_concurrency(program: &MirProgram) -> bool {
    for (_, function) in program.functions() {
        if function.kind == MirFunctionKind::Async {
            return true;
        }
        for &block_id in program.function_blocks(function) {
            let Some(block) = program.block(block_id) else {
                continue;
            };
            for &inst_id in program.block_instructions(block) {
                let Some(inst) = program.instruction(inst_id) else {
                    continue;
                };
                if let InstructionKind::Assign { value, .. } = inst.kind {
                    match value {
                        Rvalue::Spawn(_) => return true,
                        Rvalue::Builtin { builtin, .. } => {
                            let name = short_builtin_name(builtin);
                            if matches!(
                                name,
                                "thread_spawn"
                                    | "mutex_lock"
                                    | "mutex_unlock"
                                    | "rwlock_wrlock"
                                    | "rwlock_wrunlock"
                            ) {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    false
}

/// Whether `ty` is `Send` (safe to transfer across thread/task boundaries).
///
/// Raw pointers (`Ptr[T]`, `RawPtr`) and borrowed views (`Slice[T]`, `StrSlice`)
/// are `!Send`, as is any compound type (struct, enum, tuple, list, map, task)
/// that transitively contains a `!Send` type.
#[must_use]
pub fn is_send_type(program: &MirProgram, types: &TypeInterner, ty: TypeId) -> bool {
    let mut visited = BTreeSet::new();
    is_send_type_inner(program, types, ty, &mut visited)
}

/// Whether `ty` is `Sync` (safe to share concurrently across threads).
///
/// Any `!Send` type is also `!Sync`. For heap-allocated aggregates/containers,
/// concurrent mutable access is further enforced by `verify_concurrent_writes`.
#[must_use]
pub fn is_sync_type(program: &MirProgram, types: &TypeInterner, ty: TypeId) -> bool {
    is_send_type(program, types, ty)
}

fn is_send_type_inner(
    program: &MirProgram,
    types: &TypeInterner,
    ty: TypeId,
    visited: &mut BTreeSet<TypeId>,
) -> bool {
    if !visited.insert(ty) {
        return true;
    }
    match types.kind(ty) {
        TypeKind::Ptr(_) | TypeKind::Slice(_) => false,
        TypeKind::Primitive(PrimitiveType::RawPtr | PrimitiveType::StrSlice) => false,
        TypeKind::List(inner) | TypeKind::Task(inner) => {
            is_send_type_inner(program, types, inner, visited)
        }
        TypeKind::Map { key, value } => {
            is_send_type_inner(program, types, key, visited)
                && is_send_type_inner(program, types, value, visited)
        }
        TypeKind::Tuple(elements) => types
            .list(elements)
            .iter()
            .all(|&elem| is_send_type_inner(program, types, elem, visited)),
        TypeKind::Nominal { .. } => {
            for (_, aggregate) in program.aggregates() {
                if aggregate.ty != ty {
                    continue;
                }
                for &field_id in program.aggregate_fields(aggregate) {
                    if let Some(field) = program.field(field_id)
                        && !is_send_type_inner(program, types, field.ty, visited)
                    {
                        return false;
                    }
                }
                for &variant_id in program.aggregate_variants(aggregate) {
                    if let Some(variant) = program.variant(variant_id) {
                        for &field_id in program.variant_fields(variant) {
                            if let Some(field) = program.field(field_id)
                                && !is_send_type_inner(program, types, field.ty, visited)
                            {
                                return false;
                            }
                        }
                    }
                }
            }
            true
        }
        _ => true,
    }
}

fn verify_send_sync_boundaries(
    program: &MirProgram,
    types: &TypeInterner,
) -> Vec<OwnershipBalanceError> {
    let mut violations = Vec::new();
    let mut reported: BTreeSet<(MirFunctionId, BasicBlockId, MirLocalId)> = BTreeSet::new();

    for (fn_id, _) in program.functions() {
        let blocks = reachable_function_blocks(program, fn_id);
        // Track closure capture operands per local in this function so
        // `spawn closure` / `thread_spawn(closure)` checks captured values too.
        let mut closure_captures: BTreeMap<MirLocalId, Vec<MirLocalId>> = BTreeMap::new();
        for &block_id in &blocks {
            let Some(block) = program.block(block_id) else {
                continue;
            };
            for &inst_id in program.block_instructions(block) {
                let Some(inst) = program.instruction(inst_id) else {
                    continue;
                };
                if let InstructionKind::Assign {
                    target,
                    value: Rvalue::MakeClosure { captures, .. },
                } = inst.kind
                {
                    let cap_locals: Vec<MirLocalId> = program
                        .operands(captures)
                        .iter()
                        .filter_map(|op| match *op {
                            Operand::Copy(l) => Some(l),
                            _ => None,
                        })
                        .collect();
                    closure_captures.insert(target, cap_locals);
                }
            }
        }

        let mut check_local = |block_id: BasicBlockId, local: MirLocalId, origin: OriginId| {
            if let Some(local_data) = program.local(local)
                && !is_send_type(program, types, local_data.ty)
                && reported.insert((fn_id, block_id, local))
            {
                violations.push(OwnershipBalanceError {
                    kind: OwnershipBalanceErrorKind::NonSendTransfer {
                        function: fn_id,
                        block: block_id,
                        local,
                        ty: local_data.ty,
                    },
                    origin: Some(origin),
                });
            }
        };

        for &block_id in &blocks {
            let Some(block) = program.block(block_id) else {
                continue;
            };
            for &inst_id in program.block_instructions(block) {
                let Some(inst) = program.instruction(inst_id) else {
                    continue;
                };
                let InstructionKind::Assign { value, .. } = inst.kind else {
                    continue;
                };
                match value {
                    Rvalue::Spawn(Operand::Copy(local)) => {
                        check_local(block_id, local, inst.origin);
                        if let Some(caps) = closure_captures.get(&local) {
                            for &cap in caps {
                                check_local(block_id, cap, inst.origin);
                            }
                        }
                    }
                    Rvalue::Call { callee, arguments } => {
                        let is_async = match callee {
                            Operand::Function(cid) => program
                                .function(cid)
                                .is_some_and(|f| f.kind == MirFunctionKind::Async),
                            _ => false,
                        };
                        if is_async {
                            for &arg in program.operands(arguments) {
                                if let Operand::Copy(local) = arg {
                                    check_local(block_id, local, inst.origin);
                                }
                            }
                        }
                    }
                    Rvalue::Builtin { builtin, arguments }
                        if short_builtin_name(builtin) == "thread_spawn" =>
                    {
                        for &arg in program.operands(arguments) {
                            if let Operand::Copy(local) = arg {
                                check_local(block_id, local, inst.origin);
                                if let Some(caps) = closure_captures.get(&local) {
                                    for &cap in caps {
                                        check_local(block_id, cap, inst.origin);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    violations
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ObjId {
    Param(MirFunctionId, usize),
    Capture(MirFunctionId, usize),
    Alloc(MirFunctionId, InstructionId),
    ReturnFrom(MirFunctionId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConcurBlockState {
    lock_depth: usize,
    objs: BTreeMap<MirLocalId, BTreeSet<ObjId>>,
    contained: BTreeMap<ObjId, BTreeSet<ObjId>>,
    closure_fn: BTreeMap<ObjId, MirFunctionId>,
    closure_caps: BTreeMap<ObjId, Vec<BTreeSet<ObjId>>>,
    shared_objs: BTreeSet<ObjId>,
    unlocked_shared_objs: BTreeSet<ObjId>,
}

impl ConcurBlockState {
    fn entry_for(program: &MirProgram, function_id: MirFunctionId) -> Self {
        let mut objs = BTreeMap::new();
        if let Some(func) = program.function(function_id) {
            let params = program.function_parameters(func);
            let cap_count = program.function_captures(func).len().min(params.len());
            for (pos, &cap_local) in params[..cap_count].iter().enumerate() {
                objs.entry(cap_local)
                    .or_insert_with(BTreeSet::new)
                    .insert(ObjId::Capture(function_id, pos));
            }
            for (pos, &param) in params[cap_count..].iter().enumerate() {
                objs.entry(param)
                    .or_insert_with(BTreeSet::new)
                    .insert(ObjId::Param(function_id, pos));
            }
        }
        Self {
            lock_depth: 0,
            objs,
            contained: BTreeMap::new(),
            closure_fn: BTreeMap::new(),
            closure_caps: BTreeMap::new(),
            shared_objs: BTreeSet::new(),
            unlocked_shared_objs: BTreeSet::new(),
        }
    }

    fn meet_with(&mut self, other: &Self) -> bool {
        let mut changed = false;
        let min_depth = self.lock_depth.min(other.lock_depth);
        if min_depth != self.lock_depth {
            self.lock_depth = min_depth;
            changed = true;
        }
        for (&local, set) in &other.objs {
            let dst = self.objs.entry(local).or_default();
            for &obj in set {
                changed |= dst.insert(obj);
            }
        }
        for (&obj, set) in &other.contained {
            let dst = self.contained.entry(obj).or_default();
            for &child in set {
                changed |= dst.insert(child);
            }
        }
        for (&obj, &cid) in &other.closure_fn {
            if let std::collections::btree_map::Entry::Vacant(e) = self.closure_fn.entry(obj) {
                e.insert(cid);
                changed = true;
            }
        }
        for (&obj, caps) in &other.closure_caps {
            let dst = self.closure_caps.entry(obj).or_insert_with(|| caps.clone());
            if dst.len() < caps.len() {
                dst.resize_with(caps.len(), BTreeSet::new);
            }
            for (i, set) in caps.iter().enumerate() {
                for &c in set {
                    changed |= dst[i].insert(c);
                }
            }
        }
        for &obj in &other.shared_objs {
            changed |= self.shared_objs.insert(obj);
        }
        for &obj in &other.unlocked_shared_objs {
            changed |= self.unlocked_shared_objs.insert(obj);
        }
        changed
    }

    fn reachable_objs(&self, roots: &BTreeSet<ObjId>) -> BTreeSet<ObjId> {
        let mut seen = BTreeSet::new();
        let mut queue: VecDeque<ObjId> = roots.iter().copied().collect();
        while let Some(obj) = queue.pop_front() {
            if !seen.insert(obj) {
                continue;
            }
            if let Some(children) = self.contained.get(&obj) {
                for &child in children {
                    if !seen.contains(&child) {
                        queue.push_back(child);
                    }
                }
            }
            if let Some(caps) = self.closure_caps.get(&obj) {
                for cap_set in caps {
                    for &child in cap_set {
                        if !seen.contains(&child) {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }
        seen
    }

    fn mark_shared_recursive(&mut self, roots: &BTreeSet<ObjId>, unlocked: bool) {
        let reached = self.reachable_objs(roots);
        for obj in reached {
            self.shared_objs.insert(obj);
            if unlocked {
                self.unlocked_shared_objs.insert(obj);
            }
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ConcurSummaries {
    param_shared: BTreeSet<(MirFunctionId, usize)>,
    param_unlocked: BTreeSet<(MirFunctionId, usize)>,
    capture_shared: BTreeSet<(MirFunctionId, usize)>,
    capture_unlocked: BTreeSet<(MirFunctionId, usize)>,
    param_became_shared: BTreeSet<(MirFunctionId, usize)>,
    return_shared: BTreeSet<MirFunctionId>,
}

fn short_builtin_name(builtin: BuiltinId) -> &'static str {
    let name = builtin.descriptor().name;
    name.strip_prefix("lpp_").unwrap_or(name)
}

fn is_mutating_builtin(name: &str) -> bool {
    matches!(
        name,
        "list_push"
            | "list_set"
            | "list_pop"
            | "list_insert"
            | "list_remove"
            | "list_reserve"
            | "list_clear"
            | "list_truncate"
            | "list_swap"
            | "list_reverse"
            | "list_sort"
            | "list_sort_desc"
            | "list_sort_u"
            | "list_extend"
            | "map_put"
            | "map_put_str"
            | "map_put_float"
            | "map_put_str_float"
            | "map_remove"
            | "map_remove_str"
            | "map_clear"
            | "buf_set8"
            | "buf_set16le"
            | "buf_set16be"
            | "buf_set32le"
            | "buf_set32be"
            | "buf_set64le"
            | "buf_set64be"
            | "buf_write"
            | "buf_write_str"
            | "buf_copy"
    )
}

fn is_reader_builtin(name: &str) -> bool {
    matches!(
        name,
        "list_get"
            | "get"
            | "list_pop"
            | "map_get"
            | "map_get_str"
            | "map_get_float"
            | "map_get_str_float"
            | "slice"
            | "slice_get"
    )
}

fn reachable_function_blocks(
    program: &MirProgram,
    function_id: MirFunctionId,
) -> Vec<BasicBlockId> {
    let Some(function) = program.function(function_id) else {
        return Vec::new();
    };
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    visited.insert(function.entry);
    queue.push_back(function.entry);
    while let Some(block_id) = queue.pop_front() {
        for succ in successors(program, block_id) {
            if visited.insert(succ) {
                queue.push_back(succ);
            }
        }
    }
    visited.into_iter().collect()
}

fn insert_place_uses(
    program: &MirProgram,
    place: &MirPlace,
    include_root: bool,
    live: &mut BTreeSet<MirLocalId>,
) {
    if include_root {
        live.insert(place.root);
    }
    for proj in program.place_projections(place) {
        if let PlaceProjection::ListIndex(Operand::Copy(idx)) = *proj {
            live.insert(idx);
        }
    }
}

fn insert_rvalue_uses(program: &MirProgram, rvalue: Rvalue, live: &mut BTreeSet<MirLocalId>) {
    let add_op = |op: Operand, live: &mut BTreeSet<MirLocalId>| {
        if let Operand::Copy(l) = op {
            live.insert(l);
        }
    };
    let add_ops = |range, live: &mut BTreeSet<MirLocalId>| {
        for &op in program.operands(range) {
            if let Operand::Copy(l) = op {
                live.insert(l);
            }
        }
    };
    match rvalue {
        Rvalue::Use(op)
        | Rvalue::Unary { operand: op, .. }
        | Rvalue::Cast { value: op, .. }
        | Rvalue::ListLen(op) => {
            add_op(op, live);
        }
        Rvalue::Binary { left, right, .. } => {
            add_op(left, live);
            add_op(right, live);
        }
        Rvalue::Tuple(ops)
        | Rvalue::List(ops)
        | Rvalue::ConstructStruct { fields: ops, .. }
        | Rvalue::ConstructVariant { fields: ops, .. }
        | Rvalue::ExternalCall { arguments: ops, .. }
        | Rvalue::Builtin { arguments: ops, .. }
        | Rvalue::Intrinsic { arguments: ops, .. }
        | Rvalue::MakeClosure { captures: ops, .. } => add_ops(ops, live),
        Rvalue::Load(place_id) => {
            if let Some(place) = program.place(place_id) {
                insert_place_uses(program, place, true, live);
            }
        }
        Rvalue::Call { callee, arguments } => {
            add_op(callee, live);
            add_ops(arguments, live);
        }
        Rvalue::Await(op) | Rvalue::Spawn(op) => add_op(op, live),
    }
}

/// Backward liveness per reachable instruction: returns `(block_id, inst_idx)`
/// mapped to the set of locals live immediately *after* that instruction.
fn compute_live_after_inst(
    program: &MirProgram,
    function_id: MirFunctionId,
    blocks: &[BasicBlockId],
) -> BTreeMap<(BasicBlockId, usize), BTreeSet<MirLocalId>> {
    let mut live_in: BTreeMap<BasicBlockId, BTreeSet<MirLocalId>> = BTreeMap::new();
    let mut live_after: BTreeMap<(BasicBlockId, usize), BTreeSet<MirLocalId>> = BTreeMap::new();

    let bound = 64 + blocks.len().saturating_mul(6);
    for _ in 0..bound {
        let mut changed = false;
        for &block_id in blocks.iter().rev() {
            let Some(block) = program.block(block_id) else {
                continue;
            };
            let mut live: BTreeSet<MirLocalId> = BTreeSet::new();
            for succ in successors(program, block_id) {
                if let Some(succ_in) = live_in.get(&succ) {
                    live.extend(succ_in.iter().copied());
                }
            }
            match block.terminator {
                Terminator::Return(Some(Operand::Copy(l)))
                | Terminator::Branch {
                    condition: Operand::Copy(l),
                    ..
                }
                | Terminator::SwitchEnum {
                    subject: Operand::Copy(l),
                    ..
                } => {
                    live.insert(l);
                }
                _ => {}
            }

            let insts = program.block_instructions(block);
            for (idx, &inst_id) in insts.iter().enumerate().rev() {
                live_after.insert((block_id, idx), live.clone());
                let Some(inst) = program.instruction(inst_id) else {
                    continue;
                };
                match inst.kind {
                    InstructionKind::Assign { target, value } => {
                        live.remove(&target);
                        insert_rvalue_uses(program, value, &mut live);
                    }
                    InstructionKind::Store { place, value } => {
                        if let Some(mir_place) = program.place(place) {
                            let projs = program.place_projections(mir_place);
                            if projs.is_empty() {
                                live.remove(&mir_place.root);
                                insert_place_uses(program, mir_place, false, &mut live);
                            } else {
                                insert_place_uses(program, mir_place, true, &mut live);
                            }
                        }
                        if let Operand::Copy(l) = value {
                            live.insert(l);
                        }
                    }
                }
            }
            let prev = live_in.entry(block_id).or_default();
            if *prev != live {
                *prev = live;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let _ = function_id;
    live_after
}

fn resolve_call_targets(
    program: &MirProgram,
    state: &ConcurBlockState,
    callee: Operand,
) -> Vec<MirFunctionId> {
    match callee {
        Operand::Function(cid) => vec![cid],
        Operand::Copy(local) => {
            let mut out = BTreeSet::new();
            if let Some(objs) = state.objs.get(&local) {
                for obj in objs {
                    if let Some(&cid) = state.closure_fn.get(obj) {
                        out.insert(cid);
                    }
                }
            }
            if !out.is_empty() {
                return out.into_iter().collect();
            }
            if let Some(mir_local) = program.local(local)
                && program
                    .functions()
                    .find(|(_, f)| f.ty == mir_local.ty)
                    .is_some()
            {
                return program
                    .functions()
                    .filter(|(_, f)| f.ty == mir_local.ty)
                    .map(|(id, _)| id)
                    .collect();
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn operand_objs(state: &ConcurBlockState, op: Operand) -> BTreeSet<ObjId> {
    match op {
        Operand::Copy(local) => state.objs.get(&local).cloned().unwrap_or_default(),
        _ => BTreeSet::new(),
    }
}

struct SpawnCheckCtx<'a> {
    program: &'a MirProgram,
    function_id: MirFunctionId,
    live_after: &'a BTreeSet<MirLocalId>,
    state: &'a mut ConcurBlockState,
    summaries: &'a mut ConcurSummaries,
    summary_changed: &'a mut bool,
}

fn check_spawn_transfer(ctx: SpawnCheckCtx<'_>, spawn_op: Operand, extra_args: &[Operand]) {
    let SpawnCheckCtx {
        program,
        function_id,
        live_after,
        state,
        summaries,
        summary_changed,
    } = ctx;
    let params: BTreeSet<MirLocalId> = program
        .function(function_id)
        .map(|f| program.function_parameters(f).iter().copied().collect())
        .unwrap_or_default();

    let mut live_objs_excluding_spawn = BTreeSet::new();
    let spawn_local = match spawn_op {
        Operand::Copy(l) => Some(l),
        _ => None,
    };
    let mut spawn_handle_reused = false;
    for &live_local in live_after {
        if Some(live_local) == spawn_local {
            spawn_handle_reused = true;
            continue;
        }
        if let Some(objs) = state.objs.get(&live_local) {
            let reached = state.reachable_objs(objs);
            live_objs_excluding_spawn.extend(reached);
        }
    }

    let targets = resolve_call_targets(program, state, spawn_op);
    if let Operand::Copy(local) = spawn_op {
        let closure_objs = state.objs.get(&local).cloned().unwrap_or_default();
        let mut handled_via_closure_obj = false;
        for obj in closure_objs {
            if let Some(caps) = state.closure_caps.get(&obj).cloned() {
                handled_via_closure_obj = true;
                let cid_opt = state.closure_fn.get(&obj).copied();
                for (pos, cap_roots) in caps.iter().enumerate() {
                    let cap_reach = state.reachable_objs(cap_roots);
                    let has_param_or_shared = cap_reach.iter().any(|o| {
                        matches!(o, ObjId::Param(_, _) | ObjId::Capture(_, _))
                            || state.shared_objs.contains(o)
                    });
                    let aliases_live = !cap_reach.is_disjoint(&live_objs_excluding_spawn);
                    if spawn_handle_reused || has_param_or_shared || aliases_live {
                        state.mark_shared_recursive(cap_roots, true);
                        for o in &cap_reach {
                            if let ObjId::Param(pf, pi) = *o {
                                *summary_changed |= summaries.param_became_shared.insert((pf, pi));
                            }
                        }
                        if let Some(cid) = cid_opt {
                            *summary_changed |= summaries.capture_shared.insert((cid, pos));
                            *summary_changed |= summaries.capture_unlocked.insert((cid, pos));
                        }
                        for &cid in &targets {
                            *summary_changed |= summaries.capture_shared.insert((cid, pos));
                            *summary_changed |= summaries.capture_unlocked.insert((cid, pos));
                        }
                    }
                }
            }
        }
        if !handled_via_closure_obj && extra_args.is_empty() {
            // Fallback when closure handle did not carry tracked `closure_caps`
            // (for example, passed in via a parameter or container).
            for cid in &targets {
                let cap_count = program
                    .function(*cid)
                    .map(|f| program.function_captures(f).len())
                    .unwrap_or(0);
                for pos in 0..cap_count {
                    *summary_changed |= summaries.capture_shared.insert((*cid, pos));
                    *summary_changed |= summaries.capture_unlocked.insert((*cid, pos));
                }
            }
            let roots = operand_objs(state, spawn_op);
            state.mark_shared_recursive(&roots, true);
        }
    }

    for &arg_op in extra_args {
        if let Operand::Copy(arg_local) = arg_op {
            let roots = state.objs.get(&arg_local).cloned().unwrap_or_default();
            let reach = state.reachable_objs(&roots);
            let is_param_or_cap = params.contains(&arg_local)
                || reach.iter().any(|o| {
                    matches!(o, ObjId::Param(_, _) | ObjId::Capture(_, _))
                        || state.shared_objs.contains(o)
                });
            let aliases_live =
                !reach.is_disjoint(&live_objs_excluding_spawn) || spawn_handle_reused;
            if is_param_or_cap || aliases_live {
                state.mark_shared_recursive(&roots, true);
                for o in &reach {
                    if let ObjId::Param(pf, pi) = *o {
                        *summary_changed |= summaries.param_became_shared.insert((pf, pi));
                    }
                }
            }
        }
    }
}

/// Apply concurrent-write & synchronization transfer to one block.
#[allow(clippy::too_many_arguments)]
fn apply_concur_block(
    program: &MirProgram,
    types: &TypeInterner,
    function_id: MirFunctionId,
    block_id: BasicBlockId,
    in_state: &ConcurBlockState,
    live_after_map: &BTreeMap<(BasicBlockId, usize), BTreeSet<MirLocalId>>,
    summaries: &mut ConcurSummaries,
    summary_changed: &mut bool,
) -> (
    ConcurBlockState,
    BTreeMap<(BasicBlockId, MirLocalId), OriginId>,
) {
    let block = program
        .block(block_id)
        .expect("verified MIR retains every block");
    let mut state = in_state.clone();
    let mut violations: BTreeMap<(BasicBlockId, MirLocalId), OriginId> = BTreeMap::new();

    for (idx, &inst_id) in program.block_instructions(block).iter().enumerate() {
        let instruction = program
            .instruction(inst_id)
            .expect("verified MIR retains every instruction");
        let empty_live = BTreeSet::new();
        let live_after = live_after_map.get(&(block_id, idx)).unwrap_or(&empty_live);

        let check_write_local =
            |local: MirLocalId,
             state: &ConcurBlockState,
             violations: &mut BTreeMap<(BasicBlockId, MirLocalId), OriginId>| {
                if state.lock_depth == 0
                    && let Some(objs) = state.objs.get(&local)
                    && objs
                        .iter()
                        .any(|obj| state.unlocked_shared_objs.contains(obj))
                {
                    violations
                        .entry((block_id, local))
                        .or_insert(instruction.origin);
                }
            };

        match instruction.kind {
            InstructionKind::Assign { target, value } => {
                let mut new_target_objs = BTreeSet::new();
                match value {
                    Rvalue::Use(op) => {
                        new_target_objs = operand_objs(&state, op);
                        if let Operand::Function(cid) = op {
                            let obj = ObjId::Alloc(function_id, inst_id);
                            state.closure_fn.insert(obj, cid);
                            new_target_objs.insert(obj);
                        }
                    }
                    Rvalue::MakeClosure { function, captures } => {
                        let obj = ObjId::Alloc(function_id, inst_id);
                        state.closure_fn.insert(obj, function);
                        let mut cap_sets = Vec::new();
                        for (pos, &cap_op) in program.operands(captures).iter().enumerate() {
                            let c_objs = operand_objs(&state, cap_op);
                            if c_objs.iter().any(|o| state.shared_objs.contains(o)) {
                                *summary_changed |=
                                    summaries.capture_shared.insert((function, pos));
                                if state.lock_depth == 0
                                    && c_objs
                                        .iter()
                                        .any(|o| state.unlocked_shared_objs.contains(o))
                                {
                                    *summary_changed |=
                                        summaries.capture_unlocked.insert((function, pos));
                                }
                            }
                            cap_sets.push(c_objs);
                        }
                        state.closure_caps.insert(obj, cap_sets);
                        new_target_objs.insert(obj);
                    }
                    Rvalue::Tuple(ops)
                    | Rvalue::List(ops)
                    | Rvalue::ConstructStruct { fields: ops, .. }
                    | Rvalue::ConstructVariant { fields: ops, .. } => {
                        let obj = ObjId::Alloc(function_id, inst_id);
                        let mut children = BTreeSet::new();
                        for &op in program.operands(ops) {
                            children.extend(operand_objs(&state, op));
                        }
                        if !children.is_empty() {
                            state.contained.entry(obj).or_default().extend(children);
                        }
                        new_target_objs.insert(obj);
                    }
                    Rvalue::Load(place_id) => {
                        if program
                            .local(target)
                            .is_some_and(|l| is_heap_type(types, l.ty))
                        {
                            let obj = ObjId::Alloc(function_id, inst_id);
                            new_target_objs.insert(obj);
                            if let Some(place) = program.place(place_id)
                                && let Some(root_objs) = state.objs.get(&place.root).cloned()
                            {
                                let root_shared =
                                    root_objs.iter().any(|o| state.shared_objs.contains(o));
                                let root_unlocked = root_objs
                                    .iter()
                                    .any(|o| state.unlocked_shared_objs.contains(o));
                                if root_shared {
                                    state.shared_objs.insert(obj);
                                }
                                if root_unlocked {
                                    state.unlocked_shared_objs.insert(obj);
                                }
                                for r in root_objs {
                                    if let Some(children) = state.contained.get(&r).cloned() {
                                        for c in children {
                                            new_target_objs.insert(c);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Rvalue::Call { callee, arguments } => {
                        let obj = ObjId::Alloc(function_id, inst_id);
                        new_target_objs.insert(obj);
                        let args = program.operands(arguments);
                        let callees = resolve_call_targets(program, &state, callee);
                        if callees.iter().any(|&cid| {
                            program
                                .function(cid)
                                .is_some_and(|f| f.kind == MirFunctionKind::Async)
                        }) {
                            check_spawn_transfer(
                                SpawnCheckCtx {
                                    program,
                                    function_id,
                                    live_after,
                                    state: &mut state,
                                    summaries,
                                    summary_changed,
                                },
                                callee,
                                args,
                            );
                        }
                        for cid in callees {
                            for (pos, &arg_op) in args.iter().enumerate() {
                                let a_objs = operand_objs(&state, arg_op);
                                let a_shared = a_objs.iter().any(|o| state.shared_objs.contains(o));
                                let a_unlocked = a_objs
                                    .iter()
                                    .any(|o| state.unlocked_shared_objs.contains(o));
                                if a_shared {
                                    *summary_changed |= summaries.param_shared.insert((cid, pos));
                                    if state.lock_depth == 0 && a_unlocked {
                                        *summary_changed |=
                                            summaries.param_unlocked.insert((cid, pos));
                                    }
                                }
                                if summaries.param_became_shared.contains(&(cid, pos)) {
                                    state.mark_shared_recursive(&a_objs, true);
                                    for o in &a_objs {
                                        if let ObjId::Param(pf, pi) = *o {
                                            *summary_changed |=
                                                summaries.param_became_shared.insert((pf, pi));
                                        }
                                    }
                                }
                            }
                            if summaries.return_shared.contains(&cid)
                                && program
                                    .local(target)
                                    .is_some_and(|l| is_heap_type(types, l.ty))
                            {
                                state.shared_objs.insert(obj);
                                state.unlocked_shared_objs.insert(obj);
                            }
                        }
                    }
                    Rvalue::Spawn(op) => {
                        let obj = ObjId::Alloc(function_id, inst_id);
                        new_target_objs.insert(obj);
                        check_spawn_transfer(
                            SpawnCheckCtx {
                                program,
                                function_id,
                                live_after,
                                state: &mut state,
                                summaries,
                                summary_changed,
                            },
                            op,
                            &[],
                        );
                    }
                    Rvalue::Builtin { builtin, arguments } => {
                        let bname = short_builtin_name(builtin);
                        let args = program.operands(arguments);
                        if bname == "mutex_lock" || bname == "rwlock_wrlock" {
                            state.lock_depth = state.lock_depth.saturating_add(1);
                        } else if bname == "mutex_unlock" || bname == "rwlock_wrunlock" {
                            state.lock_depth = state.lock_depth.saturating_sub(1);
                        } else if bname == "thread_spawn" {
                            if let Some(&first) = args.first() {
                                check_spawn_transfer(
                                    SpawnCheckCtx {
                                        program,
                                        function_id,
                                        live_after,
                                        state: &mut state,
                                        summaries,
                                        summary_changed,
                                    },
                                    first,
                                    &args[1..],
                                );
                            }
                        } else {
                            if is_mutating_builtin(bname)
                                && let Some(&Operand::Copy(container)) = args.first()
                            {
                                check_write_local(container, &state, &mut violations);
                                let c_objs =
                                    state.objs.get(&container).cloned().unwrap_or_default();
                                let c_shared = c_objs.iter().any(|o| state.shared_objs.contains(o));
                                let c_unlocked = c_objs
                                    .iter()
                                    .any(|o| state.unlocked_shared_objs.contains(o));
                                for &val_op in args.iter().skip(1) {
                                    let v_objs = operand_objs(&state, val_op);
                                    if !v_objs.is_empty() {
                                        for c_obj in &c_objs {
                                            state
                                                .contained
                                                .entry(*c_obj)
                                                .or_default()
                                                .extend(v_objs.iter().copied());
                                        }
                                        if c_shared {
                                            state.mark_shared_recursive(&v_objs, c_unlocked);
                                        }
                                    }
                                }
                            }
                            if is_reader_builtin(bname)
                                && program
                                    .local(target)
                                    .is_some_and(|l| is_heap_type(types, l.ty))
                            {
                                let obj = ObjId::Alloc(function_id, inst_id);
                                new_target_objs.insert(obj);
                                if let Some(&Operand::Copy(container)) = args.first() {
                                    let c_objs =
                                        state.objs.get(&container).cloned().unwrap_or_default();
                                    let c_shared =
                                        c_objs.iter().any(|o| state.shared_objs.contains(o));
                                    let c_unlocked = c_objs
                                        .iter()
                                        .any(|o| state.unlocked_shared_objs.contains(o));
                                    if c_shared {
                                        state.shared_objs.insert(obj);
                                    }
                                    if c_unlocked {
                                        state.unlocked_shared_objs.insert(obj);
                                    }
                                    for c_obj in c_objs {
                                        if let Some(children) = state.contained.get(&c_obj).cloned()
                                        {
                                            new_target_objs.extend(children);
                                        }
                                    }
                                }
                            } else if program
                                .local(target)
                                .is_some_and(|l| is_heap_type(types, l.ty))
                            {
                                new_target_objs.insert(ObjId::Alloc(function_id, inst_id));
                            }
                        }
                    }
                    _ => {
                        if program
                            .local(target)
                            .is_some_and(|l| is_heap_type(types, l.ty))
                        {
                            new_target_objs.insert(ObjId::Alloc(function_id, inst_id));
                        }
                    }
                }
                state.objs.insert(target, new_target_objs);
            }
            InstructionKind::Store { place, value } => {
                if let Some(mir_place) = program.place(place) {
                    let projections = program.place_projections(mir_place);
                    if projections.is_empty() {
                        let val_objs = operand_objs(&state, value);
                        state.objs.insert(mir_place.root, val_objs);
                    } else {
                        check_write_local(mir_place.root, &state, &mut violations);
                        let val_objs = operand_objs(&state, value);
                        if !val_objs.is_empty()
                            && let Some(root_objs) = state.objs.get(&mir_place.root).cloned()
                        {
                            let root_shared =
                                root_objs.iter().any(|o| state.shared_objs.contains(o));
                            let root_unlocked = root_objs
                                .iter()
                                .any(|o| state.unlocked_shared_objs.contains(o));
                            for r in root_objs {
                                state
                                    .contained
                                    .entry(r)
                                    .or_default()
                                    .extend(val_objs.iter().copied());
                            }
                            if root_shared {
                                state.mark_shared_recursive(&val_objs, root_unlocked);
                            }
                        }
                    }
                }
            }
        }
    }

    if let Terminator::Return(Some(Operand::Copy(ret_local))) = block.terminator
        && let Some(ret_objs) = state.objs.get(&ret_local)
        && ret_objs.iter().any(|o| state.shared_objs.contains(o))
    {
        *summary_changed |= summaries.return_shared.insert(function_id);
    }

    (state, violations)
}

fn verify_concurrent_writes(
    program: &MirProgram,
    types: &TypeInterner,
) -> Vec<OwnershipBalanceError> {
    if !program_has_concurrency(program) {
        return Vec::new();
    }
    let mut summaries = ConcurSummaries::default();
    let mut blocks_by_fn: BTreeMap<MirFunctionId, Vec<BasicBlockId>> = BTreeMap::new();
    let mut live_after_by_fn: BTreeMap<
        MirFunctionId,
        BTreeMap<(BasicBlockId, usize), BTreeSet<MirLocalId>>,
    > = BTreeMap::new();

    for (fn_id, _) in program.functions() {
        let blocks = reachable_function_blocks(program, fn_id);
        let live_after = compute_live_after_inst(program, fn_id, &blocks);
        blocks_by_fn.insert(fn_id, blocks);
        live_after_by_fn.insert(fn_id, live_after);
    }

    let mut final_violations: Vec<OwnershipBalanceError> = Vec::new();
    let max_rounds = 16 + program.functions().count().saturating_mul(2);
    for _ in 0..max_rounds {
        let mut summary_changed = false;
        let mut round_violations = Vec::new();

        for (fn_id, func) in program.functions() {
            let blocks = &blocks_by_fn[&fn_id];
            let live_after_map = &live_after_by_fn[&fn_id];

            let mut initial = ConcurBlockState::entry_for(program, fn_id);
            let total_params = program.function_parameters(func).len();
            let cap_count = program.function_captures(func).len().min(total_params);
            let formal_count = total_params.saturating_sub(cap_count);
            for pos in 0..formal_count {
                let obj = ObjId::Param(fn_id, pos);
                if summaries.param_shared.contains(&(fn_id, pos)) {
                    initial.shared_objs.insert(obj);
                }
                if summaries.param_unlocked.contains(&(fn_id, pos)) {
                    initial.shared_objs.insert(obj);
                    initial.unlocked_shared_objs.insert(obj);
                }
            }
            for pos in 0..cap_count {
                let obj = ObjId::Capture(fn_id, pos);
                if summaries.capture_shared.contains(&(fn_id, pos)) {
                    initial.shared_objs.insert(obj);
                }
                if summaries.capture_unlocked.contains(&(fn_id, pos)) {
                    initial.shared_objs.insert(obj);
                    initial.unlocked_shared_objs.insert(obj);
                }
            }

            let mut in_states: BTreeMap<BasicBlockId, ConcurBlockState> = BTreeMap::new();
            let mut out_states: BTreeMap<BasicBlockId, ConcurBlockState> = BTreeMap::new();
            let mut worklist = Vec::new();
            let mut queued = BTreeSet::new();
            in_states.insert(func.entry, initial);
            worklist.push(func.entry);
            queued.insert(func.entry);

            let mut fn_violations: BTreeMap<(BasicBlockId, MirLocalId), OriginId> = BTreeMap::new();
            let bound = 64 + blocks.len().saturating_mul(4);
            for _ in 0..bound {
                let Some(block_id) = worklist.pop() else {
                    break;
                };
                queued.remove(&block_id);
                let in_state = in_states
                    .get(&block_id)
                    .cloned()
                    .unwrap_or_else(|| ConcurBlockState::entry_for(program, fn_id));
                let (out_state, block_violations) = apply_concur_block(
                    program,
                    types,
                    fn_id,
                    block_id,
                    &in_state,
                    live_after_map,
                    &mut summaries,
                    &mut summary_changed,
                );
                for (key, origin) in block_violations {
                    fn_violations.entry(key).or_insert(origin);
                }
                if out_states.get(&block_id) == Some(&out_state) {
                    continue;
                }
                out_states.insert(block_id, out_state.clone());
                for succ in successors(program, block_id) {
                    let changed = match in_states.get_mut(&succ) {
                        None => {
                            in_states.insert(succ, out_state.clone());
                            true
                        }
                        Some(prev) => prev.meet_with(&out_state),
                    };
                    if changed && queued.insert(succ) {
                        worklist.push(succ);
                    }
                }
            }

            for ((block, local), origin) in fn_violations {
                round_violations.push(OwnershipBalanceError {
                    kind: OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite {
                        function: fn_id,
                        block,
                        local,
                    },
                    origin: Some(origin),
                });
            }
        }

        final_violations = round_violations;
        if !summary_changed {
            break;
        }
    }

    let _ = ObjId::ReturnFrom;
    final_violations
}

fn successors(program: &MirProgram, block_id: BasicBlockId) -> Vec<BasicBlockId> {
    let block = program
        .block(block_id)
        .expect("verified MIR retains every block");
    match block.terminator {
        Terminator::Goto(target) => vec![target],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![then_block, else_block],
        Terminator::SwitchEnum { targets, .. } => {
            let mut targets = program.switch_targets(targets).to_vec();
            targets.sort_unstable();
            targets
        }
        Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// Apply one block's instructions to the in-state; returns the out-state
/// and the violations found along the way.
fn apply_block(
    program: &MirProgram,
    block_id: BasicBlockId,
    in_state: &State,
    heap_locals: &BTreeSet<MirLocalId>,
) -> (State, BTreeMap<(BasicBlockId, MirLocalId), OriginId>) {
    let block = program
        .block(block_id)
        .expect("verified MIR retains every block");
    let mut state: State = in_state.clone();
    let mut violations: BTreeMap<(BasicBlockId, MirLocalId), OriginId> = BTreeMap::new();
    let mut current_origin: OriginId;
    macro_rules! check_local {
        ($local:expr) => {{
            let local = $local;
            if not_alive(&state, heap_locals, local) {
                violations
                    .entry((block_id, local))
                    .or_insert(current_origin);
            }
        }};
    }

    for &instruction_id in program.block_instructions(block) {
        let instruction = program
            .instruction(instruction_id)
            .expect("verified MIR retains every instruction");
        current_origin = instruction.origin;
        match instruction.kind {
            InstructionKind::Assign { target, value } => {
                match value {
                    Rvalue::Use(operand) => {
                        if let Operand::Copy(local) = operand {
                            check_local!(local);
                        }
                    }
                    Rvalue::Unary { operand, .. }
                    | Rvalue::Cast { value: operand, .. }
                    | Rvalue::ListLen(operand) => {
                        if let Operand::Copy(local) = operand {
                            check_local!(local);
                        }
                    }
                    Rvalue::Binary { left, right, .. } => {
                        for operand in [left, right] {
                            if let Operand::Copy(local) = operand {
                                check_local!(local);
                            }
                        }
                    }
                    Rvalue::Tuple(operands)
                    | Rvalue::List(operands)
                    | Rvalue::ConstructStruct {
                        fields: operands, ..
                    }
                    | Rvalue::ConstructVariant {
                        fields: operands, ..
                    } => {
                        // Elements move into the container.
                        for operand in program.operands(operands) {
                            if let Operand::Copy(local) = *operand {
                                check_local!(local);
                                if heap_locals.contains(&local) {
                                    state.insert(local, Token::Dead);
                                }
                            }
                        }
                    }
                    Rvalue::Load(place) => {
                        // The container is read; the target gains a
                        // fresh reference.
                        if let Some(place) = program.place(place) {
                            check_local!(place.root);
                        }
                    }
                    Rvalue::Call {
                        callee, arguments, ..
                    } => {
                        // Invocation is indirection, not transfer: the
                        // callee is read, not consumed — but it must be
                        // alive (a moved callee is a use-after-move).
                        if let Operand::Copy(local) = callee {
                            check_local!(local);
                        }
                        // User-call arguments are borrowed transfers: codegen
                        // retains for the callee's parameter frame and the
                        // source local remains alive. Builtins keep their own
                        // explicit per-position consume contract below.
                        for operand in program.operands(arguments) {
                            if let Operand::Copy(local) = *operand {
                                check_local!(local);
                            }
                        }
                    }
                    Rvalue::ExternalCall { arguments, .. }
                    | Rvalue::Intrinsic { arguments, .. } => {
                        // C / intrinsics receive borrowed scalar/pointer values; the
                        // source values remain owned by their L++ locals.
                        for operand in program.operands(arguments) {
                            if let Operand::Copy(local) = *operand {
                                check_local!(local);
                            }
                        }
                    }
                    Rvalue::Builtin { builtin, arguments } => {
                        for (position, operand) in program.operands(arguments).iter().enumerate() {
                            if let Operand::Copy(local) = *operand {
                                if builtin_moves_value(builtin, position) {
                                    check_local!(local);
                                    if heap_locals.contains(&local) {
                                        state.insert(local, Token::Dead);
                                    }
                                } else {
                                    check_local!(local);
                                }
                            }
                        }
                    }
                    Rvalue::MakeClosure { captures, .. } => {
                        // Captures are borrows: the closure retains, the
                        // source stays alive.
                        for operand in program.operands(captures) {
                            if let Operand::Copy(local) = *operand {
                                check_local!(local);
                            }
                        }
                    }
                    Rvalue::Await(operand) | Rvalue::Spawn(operand) => {
                        // The task/closure handle is a receiver.
                        if let Operand::Copy(local) = operand {
                            check_local!(local);
                        }
                    }
                }
                // The target owns a fresh value (the old one is
                // released at runtime on reassignment).
                if heap_locals.contains(&target) {
                    state.insert(target, Token::Alive);
                }
            }
            InstructionKind::Store { place, value } => {
                if let Operand::Copy(local) = value {
                    check_local!(local);
                }
                if let Some(place) = program.place(place) {
                    let projections = program.place_projections(place).to_vec();
                    if projections.is_empty() {
                        if heap_locals.contains(&place.root) {
                            state.insert(place.root, Token::Alive);
                        }
                    } else {
                        check_local!(place.root);
                    }
                }
            }
        }
    }
    current_origin = block.origin;
    if let Terminator::Return(Some(Operand::Copy(local))) = block.terminator {
        check_local!(local);
        if heap_locals.contains(&local) {
            state.insert(local, Token::Dead);
        }
    }
    (state, violations)
}

/// The pass adapter: requires the core invariants plus `Ownership`
/// (4D), preserves the core invariants plus `Ownership` and
/// `NoOwningCycles`, and establishes `OwnershipBalance` by proof.
pub struct OwnershipBalancePass;

const BALANCE_PRESERVED: &[MirInvariant] = &[
    MirInvariant::References,
    MirInvariant::ControlFlow,
    MirInvariant::Types,
    MirInvariant::DefiniteInitialization,
    MirInvariant::Ownership,
    MirInvariant::NoOwningCycles,
];

impl MirPass for OwnershipBalancePass {
    fn name(&self) -> &'static str {
        "ownership-balance"
    }

    fn required(&self) -> &'static [MirInvariant] {
        &[
            MirInvariant::References,
            MirInvariant::ControlFlow,
            MirInvariant::Types,
            MirInvariant::DefiniteInitialization,
            MirInvariant::Ownership,
        ]
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        BALANCE_PRESERVED
    }

    fn established(&self) -> &'static [MirInvariant] {
        &[MirInvariant::OwnershipBalance]
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let plan = compute_ownership_plan(program, context.types).map_err(|_| PassFailure {
            message: "ownership plan construction failed",
        })?;
        if verify_ownership_balance(program, context.types, &plan).is_empty() {
            Ok(())
        } else {
            Err(PassFailure {
                message: "ownership balance proof failed",
            })
        }
    }
}
