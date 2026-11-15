//! Phase 4D ownership planning.
//!
//! The plan is a pure, read-only function of a validated `MirProgram` and
//! its type interner. It decides, for every value cell, whether the cell
//! lives in the function's frame arena (`Frame`), is heap-allocated with
//! recursive deallocation (`Owned`), or is heap-allocated and
//! ARC-managed because its type participates in an ownership cycle
//! (`Shared`). The type containment graph, its cycle partition, and the
//! per-function frame arenas are part of the plan so that the proof in
//! `verify.rs` and the snapshot in `snapshot.rs` can observe them.

use std::collections::{BTreeMap, BTreeSet};

use lpp_hir::{DefId, OriginId};
use lpp_mir::{
    BasicBlockId, InstructionKind, MirAggregateId, MirFieldId, MirFunctionId, MirFunctionKind,
    MirLocalId, MirLocalKind, MirProgram, MirVariantId, Operand, Rvalue, Terminator,
};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind, TypeListId};

/// Stable identity of a containment-graph node inside a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TypeNodeId(pub usize);

/// Placement of one `(function, local)` value cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValuePlacement {
    /// The cell lives in the function's frame arena and is released with
    /// the frame; no ARC traffic.
    Frame,
    /// The cell is heap-allocated and recursively deallocated; its type
    /// participates in no ownership cycle.
    Owned,
    /// The cell is heap-allocated and ARC-managed; its type participates
    /// in an ownership cycle that cycle breaking resolved to shared
    /// ownership.
    Shared,
}

/// Strategy for heap values of a containment-graph node type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeStrategy {
    /// Recursive deallocation is sound: the node is in no cycle.
    Owned,
    /// ARC-managed: the node is a cycle member.
    Shared,
}

/// The kind of a containment-graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContainmentNode {
    /// A value of a closure or async function type; the payload of its
    /// capture cells is contained in the value.
    Callable(MirFunctionId),
    /// A value of a nominal (struct/enum) instance; the variant field
    /// values are contained in the value.
    Aggregate(MirAggregateId),
    /// A value of `List[E]`; the element value is contained.
    List(TypeId),
    /// A value of a slice; the element value is contained.
    Slice(TypeId),
    /// A value of `Task[T]`; the pending result value is contained.
    Task(TypeId),
    /// A tuple value; the element values are contained.
    Tuple(TypeListId),
    /// A map value; the key and value values are contained.
    Map(TypeId, TypeId),
}

/// One containment-graph node with its resolved strategy and edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeNode {
    pub id: TypeNodeId,
    pub node: ContainmentNode,
    pub ty: TypeId,
    pub strategy: TypeStrategy,
    /// Whether values of this type node can reach a `spawn`/task boundary.
    /// Kept strictly distinct from cycle-breaking `strategy == TypeStrategy::Shared`.
    pub concurrent: bool,
    /// Contained value types, in stable order (ascending node id).
    pub contains: Vec<TypeNodeId>,
}

/// The placement decision for one value cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPlan {
    pub function: MirFunctionId,
    pub local: MirLocalId,
    pub placement: ValuePlacement,
    /// Whether this cell can reach or alias a value across a `spawn`/task boundary.
    pub concurrent: bool,
}

/// The frame arena of one function: its frame cells in local declaration
/// order. Functions without frame cells have no arena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaPlan {
    pub function: MirFunctionId,
    pub cells: Vec<MirLocalId>,
}

/// Aggregate counters for the plan, in stable derivation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnershipStats {
    pub cell_count: usize,
    pub frame_cell_count: usize,
    pub owned_cell_count: usize,
    pub shared_cell_count: usize,
    pub concurrent_cell_count: usize,
    pub node_count: usize,
    pub shared_node_count: usize,
    pub concurrent_node_count: usize,
    /// Number of cycles: SCCs of size greater than one plus self-edges.
    pub cycle_count: usize,
    pub arena_count: usize,
}

/// A complete, serializable ownership plan for one MIR program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipPlan {
    pub cells: Vec<CellPlan>,
    pub nodes: Vec<TypeNode>,
    pub arenas: Vec<ArenaPlan>,
    pub stats: OwnershipStats,
}

impl OwnershipPlan {
    /// The ascending type ids of the `Shared` (cycle-member) nodes —
    /// the runtime's pinned set for `execute_mir_arc`.
    #[must_use]
    pub fn pinned_types(&self) -> Vec<TypeId> {
        let types: BTreeSet<TypeId> = self
            .nodes
            .iter()
            .filter(|node| node.strategy == TypeStrategy::Shared)
            .map(|node| node.ty)
            .collect();
        types.iter().copied().collect()
    }

    /// Whether a `(function, local)` cell can reach a `spawn`/task boundary.
    #[must_use]
    pub fn is_cell_concurrent(&self, function: MirFunctionId, local: MirLocalId) -> bool {
        self.cells
            .iter()
            .any(|cell| cell.function == function && cell.local == local && cell.concurrent)
    }

    /// All `(function, local)` cells marked concurrent, in plan order.
    #[must_use]
    pub fn concurrent_cells(&self) -> Vec<(MirFunctionId, MirLocalId)> {
        self.cells
            .iter()
            .filter(|cell| cell.concurrent)
            .map(|cell| (cell.function, cell.local))
            .collect()
    }

    /// The ascending type ids of `TypeNode`s reachable across `spawn`/task
    /// boundaries, kept distinct from cycle-breaking `pinned_types()`.
    #[must_use]
    pub fn concurrent_types(&self) -> Vec<TypeId> {
        let types: BTreeSet<TypeId> = self
            .nodes
            .iter()
            .filter(|node| node.concurrent)
            .map(|node| node.ty)
            .collect();
        types.iter().copied().collect()
    }
}

/// Structural construction failures; validated MIR never triggers them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipErrorKind {
    /// A nominal type has no matching aggregate instance in the program.
    MissingAggregateInstance(TypeId),
    /// A referenced local is absent from the program.
    MissingLocal(MirLocalId),
    /// A referenced function is absent from the program.
    MissingFunction(MirFunctionId),
    /// A referenced field is absent from the program.
    MissingField(MirFieldId),
    /// A referenced variant is absent from the program.
    MissingVariant(MirVariantId),
}

/// A construction failure with the diagnostic origin when one exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipError {
    pub kind: OwnershipErrorKind,
    pub origin: Option<OriginId>,
}

impl OwnershipError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "E4401"
    }
}

impl std::fmt::Display for OwnershipError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {:#?}", self.code(), self.kind)
    }
}

impl std::error::Error for OwnershipError {}

/// Callable and aggregate lookup indexes shared by the planner and the
/// node-target expansion.
struct ProgramIndex {
    /// Closure/async functions by their function type id, in function id
    /// order.
    callables_by_type: BTreeMap<TypeId, Vec<MirFunctionId>>,
    /// Aggregate instances by (definition, type arguments).
    aggregates: BTreeMap<(DefId, TypeListId), MirAggregateId>,
}

impl ProgramIndex {
    fn build(program: &MirProgram) -> Self {
        let mut callables_by_type: BTreeMap<TypeId, Vec<MirFunctionId>> = BTreeMap::new();
        for (id, function) in program.functions() {
            if matches!(
                function.kind,
                MirFunctionKind::Closure | MirFunctionKind::Async
            ) {
                callables_by_type.entry(function.ty).or_default().push(id);
            }
        }
        let mut aggregates = BTreeMap::new();
        for (id, aggregate) in program.aggregates() {
            aggregates.insert((aggregate.definition, aggregate.arguments), id);
        }
        Self {
            callables_by_type,
            aggregates,
        }
    }

    /// The callable functions whose value type is `ty`, in function id
    /// order. Plain (non-value) functions have no entry.
    fn callables_for(&self, ty: TypeId) -> &[MirFunctionId] {
        self.callables_by_type.get(&ty).map_or(&[], Vec::as_slice)
    }

    fn aggregate_for(&self, definition: DefId, arguments: TypeListId) -> Option<MirAggregateId> {
        self.aggregates.get(&(definition, arguments)).copied()
    }
}

/// The set of node kinds referenced anywhere in the program, collected in
/// one pass over the type positions.
struct Needed {
    container_types: BTreeSet<TypeId>,
    callables: BTreeSet<MirFunctionId>,
    nominal_types: BTreeSet<TypeId>,
}

struct Collector<'types> {
    types: &'types TypeInterner,
    index: &'types ProgramIndex,
    needed: Needed,
    visited: BTreeSet<TypeId>,
}

impl<'types> Collector<'types> {
    fn new(types: &'types TypeInterner, index: &'types ProgramIndex) -> Self {
        Self {
            types,
            index,
            needed: Needed {
                container_types: BTreeSet::new(),
                callables: BTreeSet::new(),
                nominal_types: BTreeSet::new(),
            },
            visited: BTreeSet::new(),
        }
    }

    /// Collect every value type the program can hold, recursing through
    /// container constituents.
    fn collect_all(&mut self, program: &MirProgram) {
        for (_, function) in program.functions() {
            self.collect_type(function.ty);
            self.collect_type(function.return_type);
        }
        for (_, local) in program.locals() {
            self.collect_type(local.ty);
        }
        for (_, place) in program.places() {
            self.collect_type(place.ty);
        }
        for (_, aggregate) in program.aggregates() {
            self.collect_type(aggregate.ty);
        }
        for (_, field) in program.fields() {
            self.collect_type(field.ty);
        }
    }

    fn collect_type(&mut self, ty: TypeId) {
        if !self.visited.insert(ty) {
            return;
        }
        match self.types.kind(ty) {
            TypeKind::List(_)
            | TypeKind::Slice(_)
            | TypeKind::Task(_)
            | TypeKind::Tuple(_)
            | TypeKind::Map { .. } => {
                self.needed.container_types.insert(ty);
                for element in self.container_elements(ty) {
                    self.collect_type(element);
                }
            }
            TypeKind::Function { parameters, result } => {
                for callable in self.index.callables_for(ty) {
                    self.needed.callables.insert(*callable);
                }
                for parameter in self.types.list(parameters) {
                    self.collect_type(*parameter);
                }
                self.collect_type(result);
            }
            TypeKind::Nominal { .. } => {
                self.needed.nominal_types.insert(ty);
            }
            TypeKind::Primitive(_)
            | TypeKind::Ptr(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::GenericParameter(_)
            | TypeKind::BoundVariable(_)
            | TypeKind::InferenceVariable(_)
            | TypeKind::UnresolvedName { .. } => {}
        }
    }

    fn container_elements(&self, ty: TypeId) -> Vec<TypeId> {
        let mut elements = Vec::new();
        match self.types.kind(ty) {
            TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
                elements.push(element)
            }
            TypeKind::Tuple(list) => elements.extend_from_slice(self.types.list(list)),
            TypeKind::Map { key, value } => {
                elements.push(key);
                elements.push(value);
            }
            _ => unreachable!("container elements are requested for container kinds only"),
        }
        elements
    }
}

/// Stable lookups from node kinds to node ids, built after node emission
/// and strategy resolution.
struct NodeLookup {
    callables: BTreeMap<MirFunctionId, TypeNodeId>,
    aggregates: BTreeMap<MirAggregateId, TypeNodeId>,
    containers: BTreeMap<TypeId, TypeNodeId>,
    strategies: Vec<TypeStrategy>,
}

impl NodeLookup {
    fn build(nodes: &[TypeNode]) -> Self {
        let mut callables = BTreeMap::new();
        let mut aggregates = BTreeMap::new();
        let mut containers = BTreeMap::new();
        for node in nodes {
            match node.node {
                ContainmentNode::Callable(id) => {
                    callables.insert(id, node.id);
                }
                ContainmentNode::Aggregate(id) => {
                    aggregates.insert(id, node.id);
                }
                ContainmentNode::List(_)
                | ContainmentNode::Slice(_)
                | ContainmentNode::Task(_)
                | ContainmentNode::Tuple(_)
                | ContainmentNode::Map(_, _) => {
                    containers.insert(node.ty, node.id);
                }
            }
        }
        let strategies = nodes.iter().map(|node| node.strategy).collect();
        Self {
            callables,
            aggregates,
            containers,
            strategies,
        }
    }

    fn strategy_of(&self, id: TypeNodeId) -> TypeStrategy {
        self.strategies[id.0]
    }
}

/// The node that directly represents a value of type `ty`: container
/// types resolve to their own node, function types expand to every
/// callable with that type (sound over-approximation), and nominals
/// resolve to their instance node. Primitives have no node. This is the
/// edge function: a container node's `contains` list is the
/// `value_type_nodes` of each of its payload types.
fn value_type_nodes(
    index: &ProgramIndex,
    types: &TypeInterner,
    lookup: &NodeLookup,
    ty: TypeId,
) -> Result<Vec<TypeNodeId>, OwnershipError> {
    let mut nodes = Vec::new();
    match types.kind(ty) {
        TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Task(_)
        | TypeKind::Tuple(_)
        | TypeKind::Map { .. } => {
            if let Some(id) = lookup.containers.get(&ty).copied() {
                nodes.push(id);
            }
        }
        TypeKind::Function { .. } => {
            for callable in index.callables_for(ty) {
                if let Some(id) = lookup.callables.get(callable).copied() {
                    nodes.push(id);
                }
            }
        }
        TypeKind::Nominal {
            definition,
            arguments,
        } => {
            let aggregate = index
                .aggregate_for(definition, arguments)
                .ok_or(OwnershipError {
                    kind: OwnershipErrorKind::MissingAggregateInstance(ty),
                    origin: None,
                })?;
            if let Some(id) = lookup.aggregates.get(&aggregate).copied() {
                nodes.push(id);
            }
        }
        TypeKind::Primitive(_)
        | TypeKind::Ptr(_)
        | TypeKind::Never
        | TypeKind::Error
        | TypeKind::GenericParameter(_)
        | TypeKind::BoundVariable(_)
        | TypeKind::InferenceVariable(_)
        | TypeKind::UnresolvedName { .. } => {}
    }
    nodes.sort_unstable();
    nodes.dedup();
    Ok(nodes)
}

/// Emit the nodes in the contract's stable order and resolve their edges.
fn build_graph(
    program: &MirProgram,
    types: &TypeInterner,
    index: &ProgramIndex,
    needed: &Needed,
) -> Result<Vec<TypeNode>, OwnershipError> {
    let mut nodes: Vec<TypeNode> = Vec::new();

    // 1. Callable function nodes, in function id order.
    for (id, function) in program.functions() {
        if matches!(
            function.kind,
            MirFunctionKind::Closure | MirFunctionKind::Async
        ) && needed.callables.contains(&id)
        {
            nodes.push(TypeNode {
                id: TypeNodeId(nodes.len()),
                node: ContainmentNode::Callable(id),
                ty: function.ty,
                strategy: TypeStrategy::Owned,
                concurrent: false,
                contains: Vec::new(),
            });
        }
    }

    // 2. Aggregate nodes, in aggregate id order.
    for (id, aggregate) in program.aggregates() {
        if needed.nominal_types.contains(&aggregate.ty) {
            nodes.push(TypeNode {
                id: TypeNodeId(nodes.len()),
                node: ContainmentNode::Aggregate(id),
                ty: aggregate.ty,
                strategy: TypeStrategy::Owned,
                concurrent: false,
                contains: Vec::new(),
            });
        }
    }

    // 3. Container nodes: lists, slices, tasks, tuples, maps; each group
    //    in ascending type id order.
    let group_filters: [fn(TypeKind) -> bool; 5] = [
        |kind| matches!(kind, TypeKind::List(_)),
        |kind| matches!(kind, TypeKind::Slice(_)),
        |kind| matches!(kind, TypeKind::Task(_)),
        |kind| matches!(kind, TypeKind::Tuple(_)),
        |kind| matches!(kind, TypeKind::Map { .. }),
    ];
    for filter in group_filters {
        let mut group_types: Vec<TypeId> = needed
            .container_types
            .iter()
            .copied()
            .filter(|ty| filter(types.kind(*ty)))
            .collect();
        group_types.sort_unstable();
        for ty in group_types {
            let node = match types.kind(ty) {
                TypeKind::List(element) => ContainmentNode::List(element),
                TypeKind::Slice(element) => ContainmentNode::Slice(element),
                TypeKind::Task(element) => ContainmentNode::Task(element),
                TypeKind::Tuple(list) => ContainmentNode::Tuple(list),
                TypeKind::Map { key, value } => ContainmentNode::Map(key, value),
                _ => unreachable!("group filter selects container kinds only"),
            };
            nodes.push(TypeNode {
                id: TypeNodeId(nodes.len()),
                node,
                ty,
                strategy: TypeStrategy::Owned,
                concurrent: false,
                contains: Vec::new(),
            });
        }
    }

    let lookup = NodeLookup::build(&nodes);

    // 4. Resolve edges; every target type was collected, so every target
    //    node exists.
    for node in &mut nodes {
        node.contains = match node.node {
            ContainmentNode::Callable(callable) => {
                let function = program.function(callable).ok_or(OwnershipError {
                    kind: OwnershipErrorKind::MissingFunction(callable),
                    origin: None,
                })?;
                let mut targets = Vec::new();
                for capture in program.function_captures(function) {
                    let local = program.local(*capture).ok_or(OwnershipError {
                        kind: OwnershipErrorKind::MissingLocal(*capture),
                        origin: None,
                    })?;
                    targets.extend(value_type_nodes(index, types, &lookup, local.ty)?);
                }
                targets.sort_unstable();
                targets.dedup();
                targets
            }
            ContainmentNode::Aggregate(aggregate_id) => {
                let aggregate = program.aggregate(aggregate_id).ok_or(OwnershipError {
                    kind: OwnershipErrorKind::MissingAggregateInstance(node.ty),
                    origin: None,
                })?;
                let mut targets = Vec::new();
                // Struct fields live directly on the aggregate; enum
                // payload fields live under variants (see the MIR
                // builder's aggregate construction). A containment walk
                // must cover both, or every struct type looks like a
                // leaf and struct cycles are never classified `Shared`.
                for field_id in program.aggregate_fields(aggregate) {
                    let field = program.field(*field_id).ok_or(OwnershipError {
                        kind: OwnershipErrorKind::MissingField(*field_id),
                        origin: None,
                    })?;
                    targets.extend(value_type_nodes(index, types, &lookup, field.ty)?);
                }
                for variant_id in program.aggregate_variants(aggregate) {
                    let variant = program.variant(*variant_id).ok_or(OwnershipError {
                        kind: OwnershipErrorKind::MissingVariant(*variant_id),
                        origin: None,
                    })?;
                    for field_id in program.variant_fields(variant) {
                        let field = program.field(*field_id).ok_or(OwnershipError {
                            kind: OwnershipErrorKind::MissingField(*field_id),
                            origin: None,
                        })?;
                        targets.extend(value_type_nodes(index, types, &lookup, field.ty)?);
                    }
                }
                targets.sort_unstable();
                targets.dedup();
                targets
            }
            ContainmentNode::List(element)
            | ContainmentNode::Slice(element)
            | ContainmentNode::Task(element) => value_type_nodes(index, types, &lookup, element)?,
            ContainmentNode::Tuple(list) => {
                let mut targets = Vec::new();
                for element in types.list(list) {
                    targets.extend(value_type_nodes(index, types, &lookup, *element)?);
                }
                targets.sort_unstable();
                targets.dedup();
                targets
            }
            ContainmentNode::Map(key, value) => {
                let mut targets = Vec::new();
                targets.extend(value_type_nodes(index, types, &lookup, key)?);
                targets.extend(value_type_nodes(index, types, &lookup, value)?);
                targets.sort_unstable();
                targets.dedup();
                targets
            }
        };
    }

    Ok(nodes)
}

const UNVISITED: usize = usize::MAX;

/// Iterative Tarjan strongly connected components. Returns the SCCs and
/// the cycle count (SCCs of size greater than one plus self-edges).
fn strongly_connected_components<'a>(
    node_count: usize,
    edges_of: impl Fn(usize) -> &'a [TypeNodeId],
) -> (Vec<Vec<usize>>, usize) {
    let mut indices = vec![UNVISITED; node_count];
    let mut lowlinks = vec![0usize; node_count];
    let mut on_stack = vec![false; node_count];
    let mut scc_stack: Vec<usize> = Vec::new();
    let mut sccs: Vec<Vec<usize>> = Vec::new();
    let mut next_index: usize = 0;

    for root in 0..node_count {
        if indices[root] != UNVISITED {
            continue;
        }
        indices[root] = next_index;
        lowlinks[root] = next_index;
        next_index += 1;
        scc_stack.push(root);
        on_stack[root] = true;
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];

        while let Some((node, cursor)) = work.pop() {
            let children = edges_of(node);
            if cursor < children.len() {
                let child = children[cursor].0;
                // Re-push the parent with the advanced cursor, then
                // descend into the child.
                work.push((node, cursor + 1));
                if indices[child] == UNVISITED {
                    indices[child] = next_index;
                    lowlinks[child] = next_index;
                    next_index += 1;
                    scc_stack.push(child);
                    on_stack[child] = true;
                    work.push((child, 0));
                } else if on_stack[child] {
                    lowlinks[node] = lowlinks[node].min(indices[child]);
                }
                continue;
            }
            // All children processed.
            if let Some(&(parent, _)) = work.last() {
                lowlinks[parent] = lowlinks[parent].min(lowlinks[node]);
            }
            if lowlinks[node] == indices[node] {
                let mut component = Vec::new();
                loop {
                    let member = scc_stack.pop().expect("SCC stack is nonempty");
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                sccs.push(component);
            }
        }
    }

    let cycle_count = sccs
        .iter()
        .filter(|scc| scc.len() > 1 || edges_of(scc[0]).iter().any(|target| target.0 == scc[0]))
        .count();

    (sccs, cycle_count)
}

/// Escape analysis: the locals whose value crosses the function frame.
/// Marking is a single monotone pass: a use at an escape site marks its
/// source local, and copies never mark their source through the
/// destination.
fn escaping_locals(program: &MirProgram, function_id: MirFunctionId) -> BTreeSet<MirLocalId> {
    let function = program
        .function(function_id)
        .expect("validated MIR keeps functions resolvable");
    let mut escapes = BTreeSet::new();
    for &block_id in program.function_blocks(function) {
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
                    | Rvalue::ExternalCall {
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
                            if let Operand::Copy(local) = *operand {
                                escapes.insert(local);
                            }
                        }
                    }
                    Rvalue::Spawn(Operand::Copy(local)) => {
                        escapes.insert(local);
                    }
                    Rvalue::Await(Operand::Copy(local)) => {
                        escapes.insert(local);
                    }
                    _ => {}
                },
                InstructionKind::Store { .. } => {}
            }
        }
        if let Terminator::Return(Some(Operand::Copy(local))) = block.terminator {
            escapes.insert(local);
        }
    }
    escapes
}

/// Whether a value of type `ty` is itself a cycle member: the node that
/// represents the value's own type is a cycle member. A function type
/// expands to every callable with that type (sound
/// over-approximation); containers and nominals resolve to their own
/// node. Containment is not transitive for cells: an acyclic container
/// of a `Shared` value is itself `Owned` (it releases the shared child
/// instead of being ARC-managed).
fn own_type_is_shared(
    index: &ProgramIndex,
    types: &TypeInterner,
    lookup: &NodeLookup,
    ty: TypeId,
) -> Result<bool, OwnershipError> {
    for id in value_type_nodes(index, types, lookup, ty)? {
        if lookup.strategy_of(id) == TypeStrategy::Shared {
            return Ok(true);
        }
    }
    Ok(false)
}

fn builtin_short_name(builtin: BuiltinId) -> &'static str {
    let name = builtin.descriptor().name;
    name.strip_prefix("lpp_").unwrap_or(name)
}

fn is_tracked_concurrency_type(types: &TypeInterner, ty: TypeId) -> bool {
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

fn is_container_insert_builtin(name: &str) -> bool {
    matches!(
        name,
        "list_push"
            | "list_set"
            | "list_insert"
            | "list_extend"
            | "map_put"
            | "map_put_str"
            | "map_put_float"
            | "map_put_str_float"
    )
}

fn reachable_blocks(program: &MirProgram, function_id: MirFunctionId) -> Vec<BasicBlockId> {
    let Some(function) = program.function(function_id) else {
        return Vec::new();
    };
    let mut visited = BTreeSet::new();
    let mut queue = std::collections::VecDeque::new();
    visited.insert(function.entry);
    queue.push_back(function.entry);
    while let Some(block_id) = queue.pop_front() {
        let Some(block) = program.block(block_id) else {
            continue;
        };
        let push = |target: BasicBlockId,
                    visited: &mut BTreeSet<BasicBlockId>,
                    queue: &mut std::collections::VecDeque<BasicBlockId>| {
            if visited.insert(target) {
                queue.push_back(target);
            }
        };
        match block.terminator {
            Terminator::Goto(target) => push(target, &mut visited, &mut queue),
            Terminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                push(then_block, &mut visited, &mut queue);
                push(else_block, &mut visited, &mut queue);
            }
            Terminator::SwitchEnum { targets, .. } => {
                for &target in program.switch_targets(targets) {
                    push(target, &mut visited, &mut queue);
                }
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
    }
    visited.into_iter().collect()
}

fn collect_function_returns(
    program: &MirProgram,
    reachable_by_fn: &BTreeMap<MirFunctionId, Vec<BasicBlockId>>,
) -> BTreeMap<MirFunctionId, Vec<Operand>> {
    let mut returns: BTreeMap<MirFunctionId, Vec<Operand>> = BTreeMap::new();
    for (&fn_id, blocks) in reachable_by_fn {
        let mut ops = Vec::new();
        for &block_id in blocks {
            if let Some(block) = program.block(block_id)
                && let Terminator::Return(Some(op)) = block.terminator
            {
                ops.push(op);
            }
        }
        returns.insert(fn_id, ops);
    }
    returns
}

fn resolve_callee_candidates(
    program: &MirProgram,
    fn_targets: &BTreeMap<(MirFunctionId, MirLocalId), BTreeSet<MirFunctionId>>,
    fn_id: MirFunctionId,
    callee: Operand,
) -> Vec<MirFunctionId> {
    match callee {
        Operand::Function(cid) => vec![cid],
        Operand::Copy(local) => {
            if let Some(targets) = fn_targets.get(&(fn_id, local))
                && !targets.is_empty()
            {
                return targets.iter().copied().collect();
            }
            let Some(mir_local) = program.local(local) else {
                return Vec::new();
            };
            program
                .functions()
                .filter(|(_, f)| f.ty == mir_local.ty)
                .map(|(id, _)| id)
                .collect()
        }
        _ => Vec::new(),
    }
}

fn compute_fn_targets(
    program: &MirProgram,
    reachable_by_fn: &BTreeMap<MirFunctionId, Vec<BasicBlockId>>,
    returns_by_fn: &BTreeMap<MirFunctionId, Vec<Operand>>,
) -> BTreeMap<(MirFunctionId, MirLocalId), BTreeSet<MirFunctionId>> {
    let mut targets: BTreeMap<(MirFunctionId, MirLocalId), BTreeSet<MirFunctionId>> =
        BTreeMap::new();
    loop {
        let mut changed = false;
        for (&fn_id, blocks) in reachable_by_fn {
            for &block_id in blocks {
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                for &inst_id in program.block_instructions(block) {
                    let Some(inst) = program.instruction(inst_id) else {
                        continue;
                    };
                    let InstructionKind::Assign { target, value } = inst.kind else {
                        continue;
                    };
                    match value {
                        Rvalue::MakeClosure { function, .. } => {
                            changed |= targets.entry((fn_id, target)).or_default().insert(function);
                        }
                        Rvalue::Use(Operand::Function(function)) => {
                            changed |= targets.entry((fn_id, target)).or_default().insert(function);
                        }
                        Rvalue::Use(Operand::Copy(src)) => {
                            if let Some(src_targets) = targets.get(&(fn_id, src)).cloned() {
                                let entry = targets.entry((fn_id, target)).or_default();
                                for cid in src_targets {
                                    changed |= entry.insert(cid);
                                }
                            }
                        }
                        Rvalue::Call { callee, .. } => {
                            let cands = resolve_callee_candidates(program, &targets, fn_id, callee);
                            for cid in cands {
                                if let Some(rets) = returns_by_fn.get(&cid) {
                                    for &ret_op in rets {
                                        match ret_op {
                                            Operand::Function(f) => {
                                                changed |= targets
                                                    .entry((fn_id, target))
                                                    .or_default()
                                                    .insert(f);
                                            }
                                            Operand::Copy(ret_local) => {
                                                if let Some(ret_targets) =
                                                    targets.get(&(cid, ret_local)).cloned()
                                                {
                                                    let entry =
                                                        targets.entry((fn_id, target)).or_default();
                                                    for f in ret_targets {
                                                        changed |= entry.insert(f);
                                                    }
                                                }
                                            }
                                            _ => {}
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
        if !changed {
            break;
        }
    }
    targets
}

fn compute_concurrency(
    program: &MirProgram,
    types: &TypeInterner,
    nodes: &mut [TypeNode],
) -> BTreeSet<(MirFunctionId, MirLocalId)> {
    let mut reachable_by_fn: BTreeMap<MirFunctionId, Vec<BasicBlockId>> = BTreeMap::new();
    for (fn_id, _) in program.functions() {
        reachable_by_fn.insert(fn_id, reachable_blocks(program, fn_id));
    }
    let returns_by_fn = collect_function_returns(program, &reachable_by_fn);
    let fn_targets = compute_fn_targets(program, &reachable_by_fn, &returns_by_fn);

    let mut reaches_spawn: BTreeSet<(MirFunctionId, MirLocalId)> = BTreeSet::new();
    let mut spawned_callables: BTreeSet<MirFunctionId> = BTreeSet::new();

    // Seed directly from `Rvalue::Spawn` and `thread_spawn` calls in reachable blocks.
    for (&fn_id, blocks) in &reachable_by_fn {
        for &block_id in blocks {
            let Some(block) = program.block(block_id) else {
                continue;
            };
            for &inst_id in program.block_instructions(block) {
                let Some(inst) = program.instruction(inst_id) else {
                    continue;
                };
                if let InstructionKind::Assign { target, value } = inst.kind {
                    match value {
                        Rvalue::Spawn(op) => {
                            reaches_spawn.insert((fn_id, target));
                            if let Operand::Copy(local) = op {
                                reaches_spawn.insert((fn_id, local));
                            }
                            for cid in resolve_callee_candidates(program, &fn_targets, fn_id, op) {
                                spawned_callables.insert(cid);
                            }
                        }
                        Rvalue::Builtin { builtin, arguments }
                            if builtin_short_name(builtin) == "thread_spawn" =>
                        {
                            for (pos, &op) in program.operands(arguments).iter().enumerate() {
                                if let Operand::Copy(local) = op {
                                    reaches_spawn.insert((fn_id, local));
                                }
                                if pos == 0 {
                                    for cid in
                                        resolve_callee_candidates(program, &fn_targets, fn_id, op)
                                    {
                                        spawned_callables.insert(cid);
                                    }
                                } else if let Operand::Function(cid) = op {
                                    spawned_callables.insert(cid);
                                }
                            }
                        }
                        Rvalue::Call { callee, arguments } => {
                            let args = program.operands(arguments);
                            for cid in
                                resolve_callee_candidates(program, &fn_targets, fn_id, callee)
                            {
                                let Some(callee_fn) = program.function(cid) else {
                                    continue;
                                };
                                if callee_fn.kind != MirFunctionKind::Async {
                                    continue;
                                }
                                reaches_spawn.insert((fn_id, target));
                                spawned_callables.insert(cid);
                                let cap_count = program.function_captures(callee_fn).len();
                                let params = program.function_parameters(callee_fn);
                                let formal_params = &params[cap_count.min(params.len())..];
                                for (pos, &op) in args.iter().enumerate() {
                                    if let Operand::Copy(local) = op {
                                        reaches_spawn.insert((fn_id, local));
                                    }
                                    if let Some(&param) = formal_params.get(pos) {
                                        reaches_spawn.insert((cid, param));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let local_ty = |local: MirLocalId| program.local(local).map(|l| l.ty);

    // Phase 1: backward propagation for values that reach a spawn boundary.
    loop {
        let mut changed = false;
        let current_callables: Vec<MirFunctionId> = spawned_callables.iter().copied().collect();
        for cid in current_callables {
            if let Some(func) = program.function(cid) {
                let cap_count = program.function_captures(func).len();
                let params = program.function_parameters(func);
                for &cap in params.iter().take(cap_count) {
                    changed |= reaches_spawn.insert((cid, cap));
                }
            }
            if let Some(rets) = returns_by_fn.get(&cid) {
                for &ret_op in rets {
                    if let Operand::Copy(ret_l) = ret_op
                        && local_ty(ret_l).is_some_and(|ty| is_tracked_concurrency_type(types, ty))
                    {
                        changed |= reaches_spawn.insert((cid, ret_l));
                    }
                }
            }
        }

        for (&fn_id, blocks) in &reachable_by_fn {
            for &block_id in blocks {
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                for &inst_id in program.block_instructions(block) {
                    let Some(inst) = program.instruction(inst_id) else {
                        continue;
                    };
                    match inst.kind {
                        InstructionKind::Assign { target, value } => {
                            let target_reaches = reaches_spawn.contains(&(fn_id, target));
                            match value {
                                Rvalue::Use(Operand::Copy(src)) => {
                                    if target_reaches
                                        && local_ty(src).is_some_and(|ty| {
                                            !matches!(
                                                types.kind(ty),
                                                TypeKind::Primitive(PrimitiveType::Void)
                                            )
                                        })
                                    {
                                        changed |= reaches_spawn.insert((fn_id, src));
                                    }
                                }
                                Rvalue::Use(Operand::Function(cid)) => {
                                    if target_reaches {
                                        changed |= spawned_callables.insert(cid);
                                    }
                                }
                                Rvalue::MakeClosure { function, captures } => {
                                    let closure_spawned =
                                        target_reaches || spawned_callables.contains(&function);
                                    let cap_ops = program.operands(captures);
                                    let callee_caps: &[MirLocalId] = program
                                        .function(function)
                                        .map(|f| {
                                            let count = program.function_captures(f).len();
                                            let params = program.function_parameters(f);
                                            &params[..count.min(params.len())]
                                        })
                                        .unwrap_or(&[]);
                                    if closure_spawned {
                                        changed |= spawned_callables.insert(function);
                                        for &op in cap_ops {
                                            if let Operand::Copy(local) = op {
                                                changed |= reaches_spawn.insert((fn_id, local));
                                            }
                                        }
                                        for &fc in callee_caps {
                                            changed |= reaches_spawn.insert((function, fc));
                                        }
                                    }
                                    for (pos, &op) in cap_ops.iter().enumerate() {
                                        let Some(&fc) = callee_caps.get(pos) else {
                                            continue;
                                        };
                                        if let Operand::Copy(local) = op {
                                            if reaches_spawn.contains(&(function, fc)) {
                                                changed |= reaches_spawn.insert((fn_id, local));
                                            }
                                            if reaches_spawn.contains(&(fn_id, local)) {
                                                changed |= reaches_spawn.insert((function, fc));
                                            }
                                        }
                                    }
                                }
                                Rvalue::Tuple(ops)
                                | Rvalue::List(ops)
                                | Rvalue::ConstructStruct { fields: ops, .. }
                                | Rvalue::ConstructVariant { fields: ops, .. } => {
                                    if target_reaches {
                                        for &op in program.operands(ops) {
                                            if let Operand::Copy(local) = op
                                                && local_ty(local).is_some_and(|ty| {
                                                    is_tracked_concurrency_type(types, ty)
                                                })
                                            {
                                                changed |= reaches_spawn.insert((fn_id, local));
                                            }
                                        }
                                    }
                                }
                                Rvalue::Load(place_id) => {
                                    if target_reaches
                                        && local_ty(target).is_some_and(|ty| {
                                            is_tracked_concurrency_type(types, ty)
                                        })
                                        && let Some(place) = program.place(place_id)
                                    {
                                        changed |= reaches_spawn.insert((fn_id, place.root));
                                    }
                                }
                                Rvalue::Await(Operand::Copy(task)) => {
                                    if target_reaches {
                                        changed |= reaches_spawn.insert((fn_id, task));
                                    }
                                }
                                Rvalue::Call { callee, arguments } => {
                                    let args = program.operands(arguments);
                                    for cid in resolve_callee_candidates(
                                        program,
                                        &fn_targets,
                                        fn_id,
                                        callee,
                                    ) {
                                        let formal_params: &[MirLocalId] = program
                                            .function(cid)
                                            .map(|f| {
                                                let count = program.function_captures(f).len();
                                                let params = program.function_parameters(f);
                                                &params[count.min(params.len())..]
                                            })
                                            .unwrap_or(&[]);
                                        for (pos, &op) in args.iter().enumerate() {
                                            if let Operand::Copy(arg_local) = op
                                                && let Some(&param) = formal_params.get(pos)
                                                && reaches_spawn.contains(&(cid, param))
                                            {
                                                changed |= reaches_spawn.insert((fn_id, arg_local));
                                            }
                                        }
                                        if target_reaches
                                            && let Some(rets) = returns_by_fn.get(&cid)
                                        {
                                            for &ret_op in rets {
                                                match ret_op {
                                                    Operand::Copy(ret_l)
                                                        if local_ty(ret_l).is_some_and(|ty| {
                                                            is_tracked_concurrency_type(types, ty)
                                                        }) =>
                                                    {
                                                        changed |=
                                                            reaches_spawn.insert((cid, ret_l));
                                                    }
                                                    Operand::Function(ret_f) => {
                                                        changed |= spawned_callables.insert(ret_f);
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                }
                                Rvalue::Builtin { builtin, arguments } => {
                                    let bname = builtin_short_name(builtin);
                                    let args = program.operands(arguments);
                                    if is_reader_builtin(bname)
                                        && target_reaches
                                        && local_ty(target).is_some_and(|ty| {
                                            is_tracked_concurrency_type(types, ty)
                                        })
                                        && let Some(&Operand::Copy(container)) = args.first()
                                    {
                                        changed |= reaches_spawn.insert((fn_id, container));
                                    }
                                    if is_container_insert_builtin(bname)
                                        && let Some(&Operand::Copy(container)) = args.first()
                                        && reaches_spawn.contains(&(fn_id, container))
                                    {
                                        for &op in args.iter().skip(1) {
                                            if let Operand::Copy(val) = op
                                                && local_ty(val).is_some_and(|ty| {
                                                    is_tracked_concurrency_type(types, ty)
                                                })
                                            {
                                                changed |= reaches_spawn.insert((fn_id, val));
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        InstructionKind::Store { place, value } => {
                            if let Some(mir_place) = program.place(place)
                                && reaches_spawn.contains(&(fn_id, mir_place.root))
                                && let Operand::Copy(val) = value
                                && local_ty(val)
                                    .is_some_and(|ty| is_tracked_concurrency_type(types, ty))
                            {
                                changed |= reaches_spawn.insert((fn_id, val));
                            }
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Phase 2: forward alias/load/call propagation from `reaches_spawn` into `concurrent_cells`.
    let mut concurrent_cells = reaches_spawn;
    loop {
        let mut changed = false;
        for (&fn_id, blocks) in &reachable_by_fn {
            for &block_id in blocks {
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                for &inst_id in program.block_instructions(block) {
                    let Some(inst) = program.instruction(inst_id) else {
                        continue;
                    };
                    let InstructionKind::Assign { target, value } = inst.kind else {
                        continue;
                    };
                    match value {
                        Rvalue::Use(Operand::Copy(src)) => {
                            if concurrent_cells.contains(&(fn_id, src))
                                && local_ty(target).is_some_and(|ty| {
                                    !matches!(
                                        types.kind(ty),
                                        TypeKind::Primitive(PrimitiveType::Void)
                                    )
                                })
                            {
                                changed |= concurrent_cells.insert((fn_id, target));
                            }
                        }
                        Rvalue::Load(place_id) => {
                            if let Some(place) = program.place(place_id)
                                && concurrent_cells.contains(&(fn_id, place.root))
                                && local_ty(target)
                                    .is_some_and(|ty| is_tracked_concurrency_type(types, ty))
                            {
                                changed |= concurrent_cells.insert((fn_id, target));
                            }
                        }
                        Rvalue::Builtin { builtin, arguments } => {
                            let bname = builtin_short_name(builtin);
                            let args = program.operands(arguments);
                            if is_reader_builtin(bname)
                                && let Some(&Operand::Copy(container)) = args.first()
                                && concurrent_cells.contains(&(fn_id, container))
                                && local_ty(target)
                                    .is_some_and(|ty| is_tracked_concurrency_type(types, ty))
                            {
                                changed |= concurrent_cells.insert((fn_id, target));
                            }
                        }
                        Rvalue::MakeClosure { function, captures } => {
                            let cap_ops = program.operands(captures);
                            let callee_caps: &[MirLocalId] = program
                                .function(function)
                                .map(|f| {
                                    let count = program.function_captures(f).len();
                                    let params = program.function_parameters(f);
                                    &params[..count.min(params.len())]
                                })
                                .unwrap_or(&[]);
                            for (pos, &op) in cap_ops.iter().enumerate() {
                                let Some(&fc) = callee_caps.get(pos) else {
                                    continue;
                                };
                                if let Operand::Copy(local) = op
                                    && concurrent_cells.contains(&(fn_id, local))
                                {
                                    changed |= concurrent_cells.insert((function, fc));
                                }
                            }
                        }
                        Rvalue::Call { callee, arguments } => {
                            let args = program.operands(arguments);
                            for cid in
                                resolve_callee_candidates(program, &fn_targets, fn_id, callee)
                            {
                                let formal_params: &[MirLocalId] = program
                                    .function(cid)
                                    .map(|f| {
                                        let count = program.function_captures(f).len();
                                        let params = program.function_parameters(f);
                                        &params[count.min(params.len())..]
                                    })
                                    .unwrap_or(&[]);
                                for (pos, &op) in args.iter().enumerate() {
                                    if let Operand::Copy(arg_local) = op
                                        && concurrent_cells.contains(&(fn_id, arg_local))
                                        && local_ty(arg_local).is_some_and(|ty| {
                                            is_tracked_concurrency_type(types, ty)
                                        })
                                        && let Some(&param) = formal_params.get(pos)
                                    {
                                        changed |= concurrent_cells.insert((cid, param));
                                    }
                                }
                                if local_ty(target)
                                    .is_some_and(|ty| is_tracked_concurrency_type(types, ty))
                                    && let Some(rets) = returns_by_fn.get(&cid)
                                {
                                    for &ret_op in rets {
                                        if let Operand::Copy(ret_l) = ret_op
                                            && concurrent_cells.contains(&(cid, ret_l))
                                        {
                                            changed |= concurrent_cells.insert((fn_id, target));
                                        }
                                    }
                                }
                            }
                        }
                        Rvalue::Await(Operand::Copy(task)) => {
                            if concurrent_cells.contains(&(fn_id, task))
                                && local_ty(target)
                                    .is_some_and(|ty| is_tracked_concurrency_type(types, ty))
                            {
                                changed |= concurrent_cells.insert((fn_id, target));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Mark TypeNode::concurrent from concurrent cells and spawned callables, then close downward.
    let mut concurrent_non_callable_tys: BTreeSet<TypeId> = BTreeSet::new();
    for &(fn_id, local) in &concurrent_cells {
        if let Some(mir_local) = program.local(local) {
            if matches!(types.kind(mir_local.ty), TypeKind::Function { .. }) {
                for cid in
                    resolve_callee_candidates(program, &fn_targets, fn_id, Operand::Copy(local))
                {
                    spawned_callables.insert(cid);
                }
            } else {
                concurrent_non_callable_tys.insert(mir_local.ty);
            }
        }
    }

    for node in nodes.iter_mut() {
        node.concurrent = match node.node {
            ContainmentNode::Callable(cid) => spawned_callables.contains(&cid),
            _ => concurrent_non_callable_tys.contains(&node.ty),
        };
    }

    loop {
        let mut changed = false;
        let snapshot: Vec<bool> = nodes.iter().map(|n| n.concurrent).collect();
        for (idx, is_conc) in snapshot.into_iter().enumerate() {
            if !is_conc {
                continue;
            }
            let children = nodes[idx].contains.clone();
            for child in children {
                if !nodes[child.0].concurrent {
                    nodes[child.0].concurrent = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    concurrent_cells
}

fn plan_cells(
    program: &MirProgram,
    types: &TypeInterner,
    index: &ProgramIndex,
    lookup: &NodeLookup,
    concurrent_cells: &BTreeSet<(MirFunctionId, MirLocalId)>,
) -> Result<Vec<CellPlan>, OwnershipError> {
    let mut cells = Vec::new();
    for (function_id, function) in program.functions() {
        let escapes = escaping_locals(program, function_id);
        for &local in program.function_locals(function) {
            let mir_local = program.local(local).ok_or(OwnershipError {
                kind: OwnershipErrorKind::MissingLocal(local),
                origin: None,
            })?;
            let heap = mir_local.kind == MirLocalKind::Capture || escapes.contains(&local);
            let placement = if !heap {
                ValuePlacement::Frame
            } else if own_type_is_shared(index, types, lookup, mir_local.ty)? {
                ValuePlacement::Shared
            } else {
                ValuePlacement::Owned
            };
            cells.push(CellPlan {
                function: function_id,
                local,
                placement,
                concurrent: concurrent_cells.contains(&(function_id, local)),
            });
        }
    }
    Ok(cells)
}

fn plan_arenas(program: &MirProgram, cells: &[CellPlan]) -> Vec<ArenaPlan> {
    let mut arenas = Vec::new();
    for (function_id, _) in program.functions() {
        let frame: Vec<MirLocalId> = cells
            .iter()
            .filter(|cell| cell.function == function_id && cell.placement == ValuePlacement::Frame)
            .map(|cell| cell.local)
            .collect();
        if !frame.is_empty() {
            arenas.push(ArenaPlan {
                function: function_id,
                cells: frame,
            });
        }
    }
    arenas
}

/// Compute the complete ownership plan for a validated MIR program.
pub fn compute_ownership_plan(
    program: &MirProgram,
    types: &TypeInterner,
) -> Result<OwnershipPlan, OwnershipError> {
    let index = ProgramIndex::build(program);
    let mut collector = Collector::new(types, &index);
    collector.collect_all(program);
    let mut nodes = build_graph(program, types, &index, &collector.needed)?;

    // Cycle breaking: every cycle member becomes strategy `Shared`.
    let (sccs, cycle_count) =
        strongly_connected_components(nodes.len(), |node| &nodes[node].contains);
    let mut cycle_members: BTreeSet<usize> = BTreeSet::new();
    for scc in &sccs {
        let is_cycle = scc.len() > 1
            || nodes[scc[0]]
                .contains
                .iter()
                .any(|target| target.0 == scc[0]);
        if is_cycle {
            for member in scc {
                cycle_members.insert(*member);
            }
        }
    }
    for node in &mut nodes {
        node.strategy = if cycle_members.contains(&node.id.0) {
            TypeStrategy::Shared
        } else {
            TypeStrategy::Owned
        };
    }

    let concurrent_cells = compute_concurrency(program, types, &mut nodes);
    let lookup = NodeLookup::build(&nodes);
    let cells = plan_cells(program, types, &index, &lookup, &concurrent_cells)?;
    let arenas = plan_arenas(program, &cells);

    let stats = OwnershipStats {
        cell_count: cells.len(),
        frame_cell_count: cells
            .iter()
            .filter(|cell| cell.placement == ValuePlacement::Frame)
            .count(),
        owned_cell_count: cells
            .iter()
            .filter(|cell| cell.placement == ValuePlacement::Owned)
            .count(),
        shared_cell_count: cells
            .iter()
            .filter(|cell| cell.placement == ValuePlacement::Shared)
            .count(),
        concurrent_cell_count: cells.iter().filter(|cell| cell.concurrent).count(),
        node_count: nodes.len(),
        shared_node_count: cycle_members.len(),
        concurrent_node_count: nodes.iter().filter(|node| node.concurrent).count(),
        cycle_count,
        arena_count: arenas.len(),
    };

    Ok(OwnershipPlan {
        cells,
        nodes,
        arenas,
        stats,
    })
}
