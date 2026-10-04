//! Phase 4D plan proof.
//!
//! The verifier recomputes the escape analysis, containment graph, SCC
//! partition, and arena partition with independent code paths — a
//! recursive depth-first collector, Kosaraju's algorithm, and a
//! recursive escape walk — and proves the plan against them entity by
//! entity. It also proves determinism: the plan's snapshot must equal a
//! fresh recomputation's snapshot.

use std::collections::{BTreeMap, BTreeSet};

use lpp_hir::OriginId;
use lpp_mir::{
    BasicBlockId, InstructionKind, MirAggregateId, MirFunctionId, MirFunctionKind, MirLocalId,
    MirLocalKind, MirProgram, Operand, Rvalue, Terminator,
};
use lpp_types::{TypeId, TypeInterner, TypeKind};

use crate::plan::{
    ContainmentNode, OwnershipPlan, TypeStrategy, ValuePlacement, compute_ownership_plan,
};
use crate::snapshot::ownership_plan_snapshot;

/// A plan proof failure, with the offending entity and origin when
/// available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipPlanErrorKind {
    /// The program no longer yields a plan: the plan belongs to a
    /// different (or mutated) program.
    RecomputationFailed,
    /// The plan's snapshot differs from a fresh recomputation's.
    DeterminismMismatch,
    /// A cell the program requires is absent from the plan.
    MissingCell {
        function: MirFunctionId,
        local: MirLocalId,
    },
    /// A cell appears more than once.
    DuplicateCell {
        function: MirFunctionId,
        local: MirLocalId,
    },
    /// A cell references a function/local pair the program does not
    /// have.
    UnknownCell {
        function: MirFunctionId,
        local: MirLocalId,
    },
    /// A cell's placement contradicts the recomputed escape analysis or
    /// type strategy.
    InvalidCellPlacement {
        function: MirFunctionId,
        local: MirLocalId,
    },
    /// The plan's node sequence deviates from the stable emission order.
    InvalidNodeOrder { position: usize },
    /// A node's containment edges differ from the recomputed graph.
    InvalidNodeEdges { position: usize },
    /// A node's strategy differs from the recomputed cycle partition.
    InvalidNodeStrategy { position: usize },
    /// An arena the recomputation requires is absent.
    MissingArena { function: MirFunctionId },
    /// An arena with no frame cells is present.
    ExtraArena { function: MirFunctionId },
    /// An arena's cell list differs from the recomputed frame cells.
    InvalidArenaCells { function: MirFunctionId },
}

/// A plan proof failure (code `E4402`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipPlanError {
    pub kind: OwnershipPlanErrorKind,
    pub origin: Option<OriginId>,
}

impl OwnershipPlanError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "E4402"
    }
}

impl std::fmt::Display for OwnershipPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {:#?}", self.code(), self.kind)
    }
}

impl std::error::Error for OwnershipPlanError {}

/// The independently recomputed expectation, keyed for entity-by-entity
/// comparison.
struct Expectation {
    /// Expected placement per `(function, local)`.
    cells: BTreeMap<(MirFunctionId, MirLocalId), ValuePlacement>,
    /// Expected node sequence in stable emission order: (kind, ty,
    /// expected edges, cycle member?).
    nodes: Vec<(ContainmentNode, TypeId, Vec<usize>, bool)>,
    /// Expected arena per function, in function id order.
    arenas: BTreeMap<MirFunctionId, Vec<MirLocalId>>,
}

impl Expectation {
    /// Independent recomputation: a recursive DFS collector (not the
    /// planner's iterative BTree walk) and Kosaraju's algorithm (not the
    /// planner's Tarjan).
    fn recompute(program: &MirProgram, types: &TypeInterner) -> Result<Expectation, ()> {
        let mut collector = DfsCollector::new(types);
        for (_, function) in program.functions() {
            collector.collect(program, function.ty);
            collector.collect(program, function.return_type);
        }
        for (_, local) in program.locals() {
            collector.collect(program, local.ty);
        }
        for (_, place) in program.places() {
            collector.collect(program, place.ty);
        }
        for (_, aggregate) in program.aggregates() {
            collector.collect(program, aggregate.ty);
        }
        for (_, field) in program.fields() {
            collector.collect(program, field.ty);
        }
        collector.finish(program)
    }
}

/// Recursive depth-first type collector.
struct DfsCollector<'types> {
    types: &'types TypeInterner,
    visited: BTreeSet<TypeId>,
    containers: BTreeSet<TypeId>,
    callables: BTreeSet<MirFunctionId>,
    nominals: BTreeSet<TypeId>,
}

impl<'types> DfsCollector<'types> {
    fn new(types: &'types TypeInterner) -> Self {
        Self {
            types,
            visited: BTreeSet::new(),
            containers: BTreeSet::new(),
            callables: BTreeSet::new(),
            nominals: BTreeSet::new(),
        }
    }

    fn collect(&mut self, program: &MirProgram, ty: TypeId) {
        if !self.visited.insert(ty) {
            return;
        }
        match self.types.kind(ty) {
            TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
                self.containers.insert(ty);
                self.collect(program, element);
            }
            TypeKind::Tuple(list) => {
                self.containers.insert(ty);
                for element in self.types.list(list) {
                    self.collect(program, *element);
                }
            }
            TypeKind::Map { key, value } => {
                self.containers.insert(ty);
                self.collect(program, key);
                self.collect(program, value);
            }
            TypeKind::Function { parameters, result } => {
                for (function_id, function) in program.functions() {
                    if matches!(
                        function.kind,
                        MirFunctionKind::Closure | MirFunctionKind::Async
                    ) && function.ty == ty
                    {
                        self.callables.insert(function_id);
                    }
                }
                for parameter in self.types.list(parameters) {
                    self.collect(program, *parameter);
                }
                self.collect(program, result);
            }
            TypeKind::Nominal { .. } => {
                self.nominals.insert(ty);
            }
            TypeKind::Primitive(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::GenericParameter(_)
            | TypeKind::BoundVariable(_)
            | TypeKind::InferenceVariable(_)
            | TypeKind::UnresolvedName { .. } => {}
        }
    }

    /// Emit the expectation: node sequence, strategies (Kosaraju), cell
    /// placements, and arenas.
    fn finish(self, program: &MirProgram) -> Result<Expectation, ()> {
        // Node ids in the stable order the contract requires; the id is
        // the position in `order`.
        let mut order: Vec<(ContainmentNode, TypeId)> = Vec::new();
        let mut callable_ids: BTreeMap<MirFunctionId, usize> = BTreeMap::new();
        for (function_id, function) in program.functions() {
            if matches!(
                function.kind,
                MirFunctionKind::Closure | MirFunctionKind::Async
            ) && self.callables.contains(&function_id)
            {
                callable_ids.insert(function_id, order.len());
                order.push((ContainmentNode::Callable(function_id), function.ty));
            }
        }
        let mut aggregate_ids: BTreeMap<MirAggregateId, usize> = BTreeMap::new();
        for (aggregate_id, aggregate) in program.aggregates() {
            if self.nominals.contains(&aggregate.ty) {
                aggregate_ids.insert(aggregate_id, order.len());
                order.push((ContainmentNode::Aggregate(aggregate_id), aggregate.ty));
            }
        }
        let mut container_ids: BTreeMap<TypeId, usize> = BTreeMap::new();
        for kind_filter in 0..5usize {
            let group: Vec<TypeId> = self
                .containers
                .iter()
                .copied()
                .filter(|ty| container_kind(self.types.kind(*ty)) == kind_filter)
                .collect();
            for ty in group {
                let node = match self.types.kind(ty) {
                    TypeKind::List(element) => ContainmentNode::List(element),
                    TypeKind::Slice(element) => ContainmentNode::Slice(element),
                    TypeKind::Task(element) => ContainmentNode::Task(element),
                    TypeKind::Tuple(list) => ContainmentNode::Tuple(list),
                    TypeKind::Map { key, value } => ContainmentNode::Map(key, value),
                    _ => unreachable!("container filter selects container kinds"),
                };
                container_ids.insert(ty, order.len());
                order.push((node, ty));
            }
        }

        // Edges: the node of each payload's own type (direct lookup,
        // not recursive expansion).
        fn targets_for(
            program: &MirProgram,
            types: &TypeInterner,
            callable_ids: &BTreeMap<MirFunctionId, usize>,
            aggregate_ids: &BTreeMap<MirAggregateId, usize>,
            container_ids: &BTreeMap<TypeId, usize>,
            ty: TypeId,
        ) -> Result<Vec<usize>, ()> {
            let mut targets: Vec<usize> = Vec::new();
            match types.kind(ty) {
                TypeKind::List(_)
                | TypeKind::Slice(_)
                | TypeKind::Task(_)
                | TypeKind::Tuple(_)
                | TypeKind::Map { .. } => {
                    if let Some(&node_id) = container_ids.get(&ty) {
                        targets.push(node_id);
                    }
                }
                TypeKind::Function { .. } => {
                    for (&callable_id, &node_id) in callable_ids {
                        let function = program.function(callable_id).ok_or(())?;
                        if function.ty == ty {
                            targets.push(node_id);
                        }
                    }
                }
                TypeKind::Nominal { .. } => {
                    for (&aggregate_id, &node_id) in aggregate_ids {
                        let aggregate = program.aggregate(aggregate_id).ok_or(())?;
                        if aggregate.ty == ty {
                            targets.push(node_id);
                            break;
                        }
                    }
                }
                TypeKind::Primitive(_)
                | TypeKind::Never
                | TypeKind::Error
                | TypeKind::GenericParameter(_)
                | TypeKind::BoundVariable(_)
                | TypeKind::InferenceVariable(_)
                | TypeKind::UnresolvedName { .. } => {}
            }
            targets.sort_unstable();
            targets.dedup();
            Ok(targets)
        }

        let mut nodes: Vec<(ContainmentNode, TypeId, Vec<usize>, bool)> =
            Vec::with_capacity(order.len());
        for (node, ty) in &order {
            let mut edges: Vec<usize> = Vec::new();
            match node {
                ContainmentNode::Callable(callable) => {
                    let function = program.function(*callable).ok_or(())?;
                    for capture in program.function_captures(function) {
                        let local = program.local(*capture).ok_or(())?;
                        edges.extend(targets_for(
                            program,
                            self.types,
                            &callable_ids,
                            &aggregate_ids,
                            &container_ids,
                            local.ty,
                        )?);
                    }
                }
                ContainmentNode::Aggregate(aggregate_id) => {
                    let aggregate = program.aggregate(*aggregate_id).ok_or(())?;
                    for variant_id in program.aggregate_variants(aggregate) {
                        let variant = program.variant(*variant_id).ok_or(())?;
                        for field_id in program.variant_fields(variant) {
                            let field = program.field(*field_id).ok_or(())?;
                            edges.extend(targets_for(
                                program,
                                self.types,
                                &callable_ids,
                                &aggregate_ids,
                                &container_ids,
                                field.ty,
                            )?);
                        }
                    }
                }
                ContainmentNode::List(element)
                | ContainmentNode::Slice(element)
                | ContainmentNode::Task(element) => {
                    edges.extend(targets_for(
                        program,
                        self.types,
                        &callable_ids,
                        &aggregate_ids,
                        &container_ids,
                        *element,
                    )?);
                }
                ContainmentNode::Tuple(list) => {
                    for element in self.types.list(*list) {
                        edges.extend(targets_for(
                            program,
                            self.types,
                            &callable_ids,
                            &aggregate_ids,
                            &container_ids,
                            *element,
                        )?);
                    }
                }
                ContainmentNode::Map(key, value) => {
                    edges.extend(targets_for(
                        program,
                        self.types,
                        &callable_ids,
                        &aggregate_ids,
                        &container_ids,
                        *key,
                    )?);
                    edges.extend(targets_for(
                        program,
                        self.types,
                        &callable_ids,
                        &aggregate_ids,
                        &container_ids,
                        *value,
                    )?);
                }
            }
            edges.sort_unstable();
            edges.dedup();
            nodes.push((*node, *ty, edges, false));
        }

        // Cycle partition via Kosaraju (a different algorithm from the
        // planner's Tarjan): DFS finish order, then reverse-graph DFS.
        let mut reverse: Vec<Vec<usize>> = vec![Vec::new(); order.len()];
        for (source, (_, _, edges, _)) in nodes.iter().enumerate() {
            for target in edges {
                reverse[*target].push(source);
            }
        }
        let mut visited: Vec<bool> = vec![false; order.len()];
        let mut finish_order: Vec<usize> = Vec::with_capacity(order.len());
        for start in 0..order.len() {
            if visited[start] {
                continue;
            }
            visited[start] = true;
            let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
            while let Some((node, cursor)) = stack.pop() {
                let children = &nodes[node].2;
                if cursor < children.len() {
                    stack.push((node, cursor + 1));
                    let child = children[cursor];
                    if !visited[child] {
                        visited[child] = true;
                        stack.push((child, 0));
                    }
                } else {
                    finish_order.push(node);
                }
            }
        }
        let mut shared: Vec<bool> = vec![false; order.len()];
        let mut seen: Vec<bool> = vec![false; order.len()];
        for &start in finish_order.iter().rev() {
            if seen[start] {
                continue;
            }
            let mut scc: Vec<usize> = Vec::new();
            let mut dfs: Vec<usize> = vec![start];
            seen[start] = true;
            while let Some(node) = dfs.pop() {
                scc.push(node);
                for &parent in &reverse[node] {
                    if !seen[parent] {
                        seen[parent] = true;
                        dfs.push(parent);
                    }
                }
            }
            let is_cycle = scc.len() > 1 || nodes[scc[0]].2.contains(&scc[0]);
            if is_cycle {
                for member in &scc {
                    shared[*member] = true;
                }
            }
        }
        for (position, node) in nodes.iter_mut().enumerate() {
            node.3 = shared[position];
        }

        // Cell placements: recursive escape walk.
        let mut cells: BTreeMap<(MirFunctionId, MirLocalId), ValuePlacement> = BTreeMap::new();
        for (function_id, function) in program.functions() {
            let mut escapes = BTreeSet::new();
            let mut seen_blocks = BTreeSet::new();
            mark_block_recursively(program, function.entry, &mut escapes, &mut seen_blocks);
            for &local in program.function_locals(function) {
                let mir_local = program.local(local).ok_or(())?;
                let heap = mir_local.kind == MirLocalKind::Capture || escapes.contains(&local);
                let placement = if !heap {
                    ValuePlacement::Frame
                } else if own_type_is_shared(
                    program,
                    self.types,
                    &callable_ids,
                    &aggregate_ids,
                    &container_ids,
                    &shared,
                    mir_local.ty,
                )? {
                    ValuePlacement::Shared
                } else {
                    ValuePlacement::Owned
                };
                cells.insert((function_id, local), placement);
            }
        }

        // Arenas: frame cells in local declaration order per function.
        let mut arenas: BTreeMap<MirFunctionId, Vec<MirLocalId>> = BTreeMap::new();
        for (function_id, function) in program.functions() {
            let frame: Vec<MirLocalId> = program
                .function_locals(function)
                .iter()
                .copied()
                .filter(|local| {
                    cells
                        .get(&(function_id, *local))
                        .copied()
                        .unwrap_or(ValuePlacement::Frame)
                        == ValuePlacement::Frame
                })
                .collect();
            if !frame.is_empty() {
                arenas.insert(function_id, frame);
            }
        }

        Ok(Expectation {
            cells,
            nodes,
            arenas,
        })
    }
}

fn container_kind(kind: TypeKind) -> usize {
    match kind {
        TypeKind::List(_) => 0,
        TypeKind::Slice(_) => 1,
        TypeKind::Task(_) => 2,
        TypeKind::Tuple(_) => 3,
        TypeKind::Map { .. } => 4,
        _ => unreachable!("container filter selects container kinds"),
    }
}

/// Recursive escape walk: a block marks itself from its instructions and
/// terminator, then recurses to its successors; the visited set keeps
/// CFG loops terminating.
fn mark_block_recursively(
    program: &MirProgram,
    block_id: BasicBlockId,
    escapes: &mut BTreeSet<MirLocalId>,
    seen_blocks: &mut BTreeSet<BasicBlockId>,
) {
    if !seen_blocks.insert(block_id) {
        return;
    }
    let block = program
        .block(block_id)
        .expect("validated MIR keeps blocks resolvable");
    for &instruction_id in program.block_instructions(block) {
        let instruction = program
            .instruction(instruction_id)
            .expect("validated MIR keeps instructions resolvable");
        match instruction.kind {
            InstructionKind::Assign { value, .. } => match value {
                Rvalue::MakeClosure { captures, .. }
                | Rvalue::Call {
                    arguments: captures,
                    ..
                }
                // v1 container/print operations lower to builtin
                // calls; per-builtin retention is not modeled, so
                // every builtin value argument escapes.
                | Rvalue::Builtin {
                    arguments: captures,
                    ..
                } => {
                    for operand in program.operands(captures) {
                        mark_operand(*operand, escapes);
                    }
                }
                Rvalue::Spawn(operand) => mark_operand(operand, escapes),
                Rvalue::Await(operand) => mark_operand(operand, escapes),
                _ => {}
            },
            InstructionKind::Store { .. } => {}
        }
    }
    match block.terminator {
        Terminator::Return(Some(operand)) => mark_operand(operand, escapes),
        Terminator::Return(None) => {}
        Terminator::Goto(target) => {
            mark_block_recursively(program, target, escapes, seen_blocks);
        }
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => {
            mark_block_recursively(program, then_block, escapes, seen_blocks);
            mark_block_recursively(program, else_block, escapes, seen_blocks);
        }
        Terminator::SwitchEnum { targets, .. } => {
            for target in program.switch_targets(targets) {
                mark_block_recursively(program, *target, escapes, seen_blocks);
            }
        }
        Terminator::Unreachable => {}
    }
}

fn mark_operand(operand: Operand, escapes: &mut BTreeSet<MirLocalId>) {
    if let Operand::Copy(local) = operand {
        escapes.insert(local);
    }
}

/// Whether a value of type `ty` is itself a cycle member — the node of
/// its own type is a cycle member — the same non-transitive rule as the
/// planner (a function type expands to every callable with that type).
fn own_type_is_shared(
    program: &MirProgram,
    types: &TypeInterner,
    callable_ids: &BTreeMap<MirFunctionId, usize>,
    aggregate_ids: &BTreeMap<MirAggregateId, usize>,
    container_ids: &BTreeMap<TypeId, usize>,
    shared: &[bool],
    ty: TypeId,
) -> Result<bool, ()> {
    match types.kind(ty) {
        TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Task(_)
        | TypeKind::Tuple(_)
        | TypeKind::Map { .. } => Ok(container_ids
            .get(&ty)
            .is_some_and(|&node_id| shared[node_id])),
        TypeKind::Function { .. } => Ok(callable_ids.iter().any(|(&callable_id, &node_id)| {
            program
                .function(callable_id)
                .is_some_and(|function| function.ty == ty)
                && shared[node_id]
        })),
        TypeKind::Nominal { .. } => {
            for (&aggregate_id, &node_id) in aggregate_ids {
                let aggregate = program.aggregate(aggregate_id).ok_or(())?;
                if aggregate.ty == ty {
                    return Ok(shared[node_id]);
                }
            }
            // A nominal type with no aggregate instance is a structural
            // mismatch, mirroring the planner's E4401 guard.
            Err(())
        }
        TypeKind::Primitive(_)
        | TypeKind::Never
        | TypeKind::Error
        | TypeKind::GenericParameter(_)
        | TypeKind::BoundVariable(_)
        | TypeKind::InferenceVariable(_)
        | TypeKind::UnresolvedName { .. } => Ok(false),
    }
}

/// Prove the plan against the program: recompute everything
/// independently and compare entity by entity. An empty result is the
/// proof.
pub fn verify_ownership_plan(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &OwnershipPlan,
) -> Vec<OwnershipPlanError> {
    let mut errors: Vec<OwnershipPlanError> = Vec::new();
    let expectation = match Expectation::recompute(program, types) {
        Ok(expectation) => expectation,
        Err(()) => {
            errors.push(OwnershipPlanError {
                kind: OwnershipPlanErrorKind::RecomputationFailed,
                origin: None,
            });
            return errors;
        }
    };

    // V5: determinism against a fresh recomputation.
    let fresh = match compute_ownership_plan(program, types) {
        Ok(fresh) => fresh,
        Err(_) => {
            errors.push(OwnershipPlanError {
                kind: OwnershipPlanErrorKind::RecomputationFailed,
                origin: None,
            });
            return errors;
        }
    };
    if ownership_plan_snapshot(plan) != ownership_plan_snapshot(&fresh) {
        errors.push(OwnershipPlanError {
            kind: OwnershipPlanErrorKind::DeterminismMismatch,
            origin: None,
        });
    }

    // V1 + V2: cells.
    let mut seen_cells: BTreeSet<(MirFunctionId, MirLocalId)> = BTreeSet::new();
    for cell in &plan.cells {
        let key = (cell.function, cell.local);
        if !expectation.cells.contains_key(&key) {
            errors.push(OwnershipPlanError {
                kind: OwnershipPlanErrorKind::UnknownCell {
                    function: cell.function,
                    local: cell.local,
                },
                origin: None,
            });
            continue;
        }
        if !seen_cells.insert(key) {
            errors.push(OwnershipPlanError {
                kind: OwnershipPlanErrorKind::DuplicateCell {
                    function: cell.function,
                    local: cell.local,
                },
                origin: None,
            });
            continue;
        }
        let expected = expectation.cells[&key];
        if cell.placement != expected {
            errors.push(OwnershipPlanError {
                kind: OwnershipPlanErrorKind::InvalidCellPlacement {
                    function: cell.function,
                    local: cell.local,
                },
                origin: None,
            });
        }
    }
    for (&(function, local), _) in &expectation.cells {
        if !seen_cells.contains(&(function, local)) {
            errors.push(OwnershipPlanError {
                kind: OwnershipPlanErrorKind::MissingCell { function, local },
                origin: None,
            });
        }
    }

    // V3: nodes, in stable emission order.
    for (position, (expected_node, expected_ty, expected_edges, expected_shared)) in
        expectation.nodes.iter().enumerate()
    {
        match plan.nodes.get(position) {
            None => {
                errors.push(OwnershipPlanError {
                    kind: OwnershipPlanErrorKind::InvalidNodeOrder { position },
                    origin: None,
                });
            }
            Some(node) => {
                if &node.node != expected_node || node.ty != *expected_ty {
                    errors.push(OwnershipPlanError {
                        kind: OwnershipPlanErrorKind::InvalidNodeOrder { position },
                        origin: None,
                    });
                }
                let actual_edges: Vec<usize> = node.contains.iter().map(|id| id.0).collect();
                if actual_edges != *expected_edges {
                    errors.push(OwnershipPlanError {
                        kind: OwnershipPlanErrorKind::InvalidNodeEdges { position },
                        origin: None,
                    });
                }
                let actual_shared = matches!(node.strategy, TypeStrategy::Shared);
                if actual_shared != *expected_shared {
                    errors.push(OwnershipPlanError {
                        kind: OwnershipPlanErrorKind::InvalidNodeStrategy { position },
                        origin: None,
                    });
                }
            }
        }
    }
    for position in expectation.nodes.len()..plan.nodes.len() {
        errors.push(OwnershipPlanError {
            kind: OwnershipPlanErrorKind::InvalidNodeOrder { position },
            origin: None,
        });
    }

    // V4: arenas.
    for arena in &plan.arenas {
        match expectation.arenas.get(&arena.function) {
            None => {
                errors.push(OwnershipPlanError {
                    kind: OwnershipPlanErrorKind::ExtraArena {
                        function: arena.function,
                    },
                    origin: None,
                });
            }
            Some(expected_cells) => {
                if &arena.cells != expected_cells {
                    errors.push(OwnershipPlanError {
                        kind: OwnershipPlanErrorKind::InvalidArenaCells {
                            function: arena.function,
                        },
                        origin: None,
                    });
                }
            }
        }
    }
    for (function, _) in &expectation.arenas {
        if !plan.arenas.iter().any(|arena| &arena.function == function) {
            errors.push(OwnershipPlanError {
                kind: OwnershipPlanErrorKind::MissingArena {
                    function: *function,
                },
                origin: None,
            });
        }
    }

    errors
}
