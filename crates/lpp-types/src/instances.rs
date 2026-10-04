use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::mem::size_of;

use lpp_common::OptimizationOptions;
use lpp_hir::{
    ArenaId, BindingTarget, BodyId, ExprId, ExpressionKind, HirItemId, HirItemKind, HirPackage,
    InstanceId, NameBinding, OriginId, StatementKind, TypeParamId,
};

use crate::pattern::{
    PatternBudget, PatternLimit, is_concrete_type, match_type_pattern, substitute_pattern_list,
};
use crate::{
    AggregateExpressionFact, AggregateFacts, TypeAssignments, TypeInterner, TypeListId,
    TypeSubstitution,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceKey {
    pub item: HirItemId,
    pub arguments: TypeListId,
}

impl InstanceKey {
    #[must_use]
    pub const fn new(item: HirItemId, arguments: TypeListId) -> Self {
        Self { item, arguments }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceRequestKind {
    Required,
    OptionalSpecialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceLimits {
    pub max_instances_global: usize,
    pub max_instances_per_item: usize,
    pub max_depth: usize,
    pub max_fixed_point_rounds: usize,
    pub max_optional_instances: usize,
    pub max_optional_rounds: usize,
}

impl InstanceLimits {
    #[must_use]
    pub const fn for_optimization(options: OptimizationOptions) -> Self {
        let budget = options.budget();
        Self {
            max_instances_global: 65_536,
            max_instances_per_item: 4_096,
            max_depth: 64,
            max_fixed_point_rounds: 64,
            max_optional_instances: budget.optional_specialization_instances as usize,
            max_optional_rounds: budget.fixed_point_iterations as usize,
        }
    }
}

impl Default for InstanceLimits {
    fn default() -> Self {
        Self::for_optimization(OptimizationOptions::development())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceGrowthError {
    GlobalLimit { limit: usize },
    PerItemLimit { item: HirItemId, limit: usize },
    DepthLimit { limit: usize },
    FixedPointLimit { limit: usize },
    OptionalInstanceLimit { limit: usize },
    OptionalRoundLimit { limit: usize },
    Capacity,
    NotActive { instance: InstanceId },
    OutstandingWork,
}

impl fmt::Display for InstanceGrowthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GlobalLimit { limit } => {
                write!(formatter, "generic instance global limit {limit} exceeded")
            }
            Self::PerItemLimit { item, limit } => write!(
                formatter,
                "generic instance limit {limit} exceeded for HIR item {item:?}"
            ),
            Self::DepthLimit { limit } => {
                write!(formatter, "generic instance depth limit {limit} exceeded")
            }
            Self::FixedPointLimit { limit } => write!(
                formatter,
                "generic instance fixed-point limit {limit} exceeded"
            ),
            Self::OptionalInstanceLimit { limit } => write!(
                formatter,
                "optional specialization instance limit {limit} exceeded"
            ),
            Self::OptionalRoundLimit { limit } => write!(
                formatter,
                "optional specialization round limit {limit} exceeded"
            ),
            Self::Capacity => formatter.write_str("generic instance ID space exhausted"),
            Self::NotActive { instance } => {
                write!(formatter, "generic instance {instance:?} is not active")
            }
            Self::OutstandingWork => {
                formatter.write_str("generic instance plan still has outstanding work")
            }
        }
    }
}

impl std::error::Error for InstanceGrowthError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceWorkItem {
    pub id: InstanceId,
    pub key: InstanceKey,
    pub requested_by: Option<InstanceId>,
    pub origin: Option<OriginId>,
    pub kind: InstanceRequestKind,
    pub depth: usize,
    pub round: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceRecord {
    pub id: InstanceId,
    pub key: InstanceKey,
    pub requested_by: Option<InstanceId>,
    pub origin: Option<OriginId>,
    pub kind: InstanceRequestKind,
    pub depth: usize,
    pub round: usize,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceRequestOutcome {
    Queued,
    Existing(InstanceId),
    AlreadyQueued,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstanceStats {
    pub unique_requests: usize,
    pub duplicate_requests: usize,
    pub completed: usize,
    pub peak_pending: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstanceState {
    Queued(PendingInstance),
    Active(InstanceId),
    Complete(InstanceId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingInstance {
    requested_by: Option<InstanceId>,
    origin: Option<OriginId>,
    kind: InstanceRequestKind,
    depth: usize,
    round: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstancePlanner {
    limits: InstanceLimits,
    states: BTreeMap<InstanceKey, InstanceState>,
    pending: BTreeSet<InstanceKey>,
    records: Vec<InstanceRecord>,
    per_item: BTreeMap<HirItemId, usize>,
    optional_instances: usize,
    stats: InstanceStats,
}

impl InstancePlanner {
    #[must_use]
    pub fn new(limits: InstanceLimits) -> Self {
        Self {
            limits,
            states: BTreeMap::new(),
            pending: BTreeSet::new(),
            records: Vec::new(),
            per_item: BTreeMap::new(),
            optional_instances: 0,
            stats: InstanceStats::default(),
        }
    }

    pub fn collect_hir(
        package: &HirPackage,
        assignments: &TypeAssignments,
        aggregates: &AggregateFacts,
        interner: &mut TypeInterner,
        limits: InstanceLimits,
    ) -> (Self, Vec<InstanceCollectionError>) {
        let mut planner = Self::new(limits);
        let mut diagnostics = Vec::new();
        let mut definition_items = vec![None; package.names.definitions.len()];
        for (item_id, item) in package.items.enumerate() {
            if let Some(definition) = item.definition {
                definition_items[definition.index()] = Some(item_id);
            }
        }
        let owners = expression_owners(package);
        let mut dependencies: BTreeMap<HirItemId, Vec<InstanceDependency>> = BTreeMap::new();
        let mut pattern_budget = PatternBudget::new(1_000_000, 256);

        for (expression_id, expression) in package.expressions.enumerate() {
            let ExpressionKind::Name {
                binding: NameBinding::Item(BindingTarget::Definition(definition)),
                ..
            } = expression.kind
            else {
                continue;
            };
            let Some(item_id) = definition_items[definition.index()] else {
                continue;
            };
            let parameters = item_parameters(package, item_id);
            if parameters.is_empty() {
                continue;
            }
            let Some(arguments) = collect_demand_arguments(
                expression_id,
                item_id,
                parameters,
                assignments,
                interner,
                &mut pattern_budget,
                &mut diagnostics,
            ) else {
                continue;
            };
            let owner = owners[expression_id.index()];
            let owner_parameters = owner
                .map(|item| item_parameters(package, item))
                .unwrap_or_default();
            if owner_parameters.is_empty() {
                match list_is_concrete(interner, arguments, &mut pattern_budget) {
                    Ok(true) => {
                        if let Err(error) = planner.request_root(
                            InstanceKey::new(item_id, arguments),
                            Some(expression.origin),
                            InstanceRequestKind::Required,
                        ) {
                            diagnostics.push(InstanceCollectionError::Growth {
                                expression: expression_id,
                                error,
                            });
                        }
                    }
                    Ok(false) => {}
                    Err(limit) => diagnostics.push(collection_limit(expression_id, limit)),
                }
                continue;
            }
            let Some(owner_item) = owner else {
                continue;
            };
            dependencies
                .entry(owner_item)
                .or_default()
                .push(InstanceDependency {
                    item: item_id,
                    arguments,
                    expression: expression_id,
                });
        }
        for (call_expression, fact) in aggregates.expressions() {
            let AggregateExpressionFact::MethodCall { method } = fact else {
                continue;
            };
            let (ExpressionKind::Call { callee, .. } | ExpressionKind::GenericCall { callee, .. }) =
                package.expressions[call_expression].kind
            else {
                continue;
            };
            let parameters = item_parameters(package, method);
            if parameters.is_empty() {
                continue;
            }
            let Some(arguments) = collect_demand_arguments(
                callee,
                method,
                parameters,
                assignments,
                interner,
                &mut pattern_budget,
                &mut diagnostics,
            ) else {
                continue;
            };
            let owner = owners[call_expression.index()];
            let owner_parameters = owner
                .map(|item| item_parameters(package, item))
                .unwrap_or_default();
            if owner_parameters.is_empty() {
                match list_is_concrete(interner, arguments, &mut pattern_budget) {
                    Ok(true) => {
                        if let Err(error) = planner.request_root(
                            InstanceKey::new(method, arguments),
                            Some(package.expressions[call_expression].origin),
                            InstanceRequestKind::Required,
                        ) {
                            diagnostics.push(InstanceCollectionError::Growth {
                                expression: call_expression,
                                error,
                            });
                        }
                    }
                    Ok(false) => {}
                    Err(limit) => diagnostics.push(collection_limit(call_expression, limit)),
                }
                continue;
            }
            let Some(owner_item) = owner else {
                continue;
            };
            dependencies
                .entry(owner_item)
                .or_default()
                .push(InstanceDependency {
                    item: method,
                    arguments,
                    expression: callee,
                });
        }

        for demands in dependencies.values_mut() {
            demands.sort_unstable();
            demands.dedup();
        }

        loop {
            let work = match planner.pop_next() {
                Ok(Some(work)) => work,
                Ok(None) => break,
                Err(error) => {
                    diagnostics.push(InstanceCollectionError::Growth {
                        expression: ExprId::from_raw(0),
                        error,
                    });
                    break;
                }
            };
            let item = work.key.item;
            let parameters = item_parameters(package, item);
            let arguments = interner.list(work.key.arguments).to_vec();
            if parameters.len() != arguments.len() {
                diagnostics.push(InstanceCollectionError::ShapeMismatch {
                    expression: ExprId::from_raw(0),
                });
            } else {
                let mut substitution = TypeSubstitution::new();
                for (parameter, argument) in parameters.iter().zip(arguments) {
                    substitution.insert(*parameter, argument);
                }
                if let Some(demands) = dependencies.get(&work.key.item) {
                    for demand in demands {
                        let arguments = match substitute_pattern_list(
                            interner,
                            demand.arguments,
                            &substitution,
                            &mut pattern_budget,
                        ) {
                            Ok(arguments) => arguments,
                            Err(limit) => {
                                diagnostics.push(collection_limit(demand.expression, limit));
                                continue;
                            }
                        };
                        match list_is_concrete(interner, arguments, &mut pattern_budget) {
                            Ok(true) => {
                                if let Err(error) = planner.request_from(
                                    work.id,
                                    InstanceKey::new(demand.item, arguments),
                                    Some(package.expressions[demand.expression].origin),
                                    InstanceRequestKind::Required,
                                ) {
                                    diagnostics.push(InstanceCollectionError::Growth {
                                        expression: demand.expression,
                                        error,
                                    });
                                }
                            }
                            Ok(false) => {}
                            Err(limit) => {
                                diagnostics.push(collection_limit(demand.expression, limit))
                            }
                        }
                    }
                }
            }
            if let Err(error) = planner.complete(work.id) {
                diagnostics.push(InstanceCollectionError::Growth {
                    expression: ExprId::from_raw(0),
                    error,
                });
                break;
            }
        }
        (planner, diagnostics)
    }

    pub fn request_root(
        &mut self,
        key: InstanceKey,
        origin: Option<OriginId>,
        kind: InstanceRequestKind,
    ) -> Result<InstanceRequestOutcome, InstanceGrowthError> {
        self.request(key, None, origin, kind, 0, 0)
    }

    pub fn request_from(
        &mut self,
        parent: InstanceId,
        key: InstanceKey,
        origin: Option<OriginId>,
        kind: InstanceRequestKind,
    ) -> Result<InstanceRequestOutcome, InstanceGrowthError> {
        let parent = self
            .records
            .get(parent.index())
            .ok_or(InstanceGrowthError::NotActive { instance: parent })?;
        self.request(
            key,
            Some(parent.id),
            origin,
            kind,
            parent.depth.saturating_add(1),
            parent.round.saturating_add(1),
        )
    }

    fn request(
        &mut self,
        key: InstanceKey,
        requested_by: Option<InstanceId>,
        origin: Option<OriginId>,
        kind: InstanceRequestKind,
        depth: usize,
        round: usize,
    ) -> Result<InstanceRequestOutcome, InstanceGrowthError> {
        if let Some(state) = self.states.get_mut(&key) {
            self.stats.duplicate_requests = self.stats.duplicate_requests.saturating_add(1);
            return Ok(match state {
                InstanceState::Queued(pending) => {
                    if pending.kind == InstanceRequestKind::OptionalSpecialization
                        && kind == InstanceRequestKind::Required
                    {
                        pending.kind = InstanceRequestKind::Required;
                        self.optional_instances = self.optional_instances.saturating_sub(1);
                    }
                    if (depth, round) < (pending.depth, pending.round) {
                        pending.requested_by = requested_by;
                        pending.origin = origin;
                        pending.depth = depth;
                        pending.round = round;
                    }
                    InstanceRequestOutcome::AlreadyQueued
                }
                InstanceState::Active(id) | InstanceState::Complete(id) => {
                    InstanceRequestOutcome::Existing(*id)
                }
            });
        }
        if depth > self.limits.max_depth {
            return Err(InstanceGrowthError::DepthLimit {
                limit: self.limits.max_depth,
            });
        }
        if round >= self.limits.max_fixed_point_rounds {
            return Err(InstanceGrowthError::FixedPointLimit {
                limit: self.limits.max_fixed_point_rounds,
            });
        }
        if self.states.len() >= self.limits.max_instances_global {
            return Err(InstanceGrowthError::GlobalLimit {
                limit: self.limits.max_instances_global,
            });
        }
        let per_item = self.per_item.get(&key.item).copied().unwrap_or(0);
        if per_item >= self.limits.max_instances_per_item {
            return Err(InstanceGrowthError::PerItemLimit {
                item: key.item,
                limit: self.limits.max_instances_per_item,
            });
        }
        if kind == InstanceRequestKind::OptionalSpecialization {
            if self.optional_instances >= self.limits.max_optional_instances {
                return Err(InstanceGrowthError::OptionalInstanceLimit {
                    limit: self.limits.max_optional_instances,
                });
            }
            if round >= self.limits.max_optional_rounds {
                return Err(InstanceGrowthError::OptionalRoundLimit {
                    limit: self.limits.max_optional_rounds,
                });
            }
            self.optional_instances += 1;
        }
        self.states.insert(
            key,
            InstanceState::Queued(PendingInstance {
                requested_by,
                origin,
                kind,
                depth,
                round,
            }),
        );
        self.pending.insert(key);
        self.per_item.insert(key.item, per_item + 1);
        self.stats.unique_requests = self.stats.unique_requests.saturating_add(1);
        self.stats.peak_pending = self.stats.peak_pending.max(self.pending.len());
        Ok(InstanceRequestOutcome::Queued)
    }

    pub fn pop_next(&mut self) -> Result<Option<InstanceWorkItem>, InstanceGrowthError> {
        let Some(key) = self.pending.pop_first() else {
            return Ok(None);
        };
        let Some(InstanceState::Queued(pending)) = self.states.get(&key).copied() else {
            return Err(InstanceGrowthError::OutstandingWork);
        };
        let id = InstanceId::from_index(self.records.len()).ok_or(InstanceGrowthError::Capacity)?;
        let work = InstanceWorkItem {
            id,
            key,
            requested_by: pending.requested_by,
            origin: pending.origin,
            kind: pending.kind,
            depth: pending.depth,
            round: pending.round,
        };
        self.records.push(InstanceRecord {
            id,
            key,
            requested_by: pending.requested_by,
            origin: pending.origin,
            kind: pending.kind,
            depth: pending.depth,
            round: pending.round,
            complete: false,
        });
        self.states.insert(key, InstanceState::Active(id));
        Ok(Some(work))
    }

    pub fn complete(&mut self, instance: InstanceId) -> Result<(), InstanceGrowthError> {
        let Some(record) = self.records.get_mut(instance.index()) else {
            return Err(InstanceGrowthError::NotActive { instance });
        };
        if !matches!(self.states.get(&record.key), Some(InstanceState::Active(id)) if *id == instance)
        {
            return Err(InstanceGrowthError::NotActive { instance });
        }
        record.complete = true;
        self.states
            .insert(record.key, InstanceState::Complete(instance));
        self.stats.completed = self.stats.completed.saturating_add(1);
        Ok(())
    }

    pub fn finish(&self) -> Result<(), InstanceGrowthError> {
        if self.pending.is_empty() && self.records.iter().all(|record| record.complete) {
            Ok(())
        } else {
            Err(InstanceGrowthError::OutstandingWork)
        }
    }

    #[must_use]
    pub fn records(&self) -> &[InstanceRecord] {
        &self.records
    }

    #[must_use]
    pub fn instance(&self, key: InstanceKey) -> Option<InstanceId> {
        match self.states.get(&key) {
            Some(InstanceState::Active(id) | InstanceState::Complete(id)) => Some(*id),
            Some(InstanceState::Queued(_)) | None => None,
        }
    }

    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub const fn stats(&self) -> InstanceStats {
        self.stats
    }

    #[must_use]
    pub const fn limits(&self) -> InstanceLimits {
        self.limits
    }

    #[must_use]
    pub const fn compact_key_size() -> usize {
        size_of::<InstanceKey>()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct InstanceDependency {
    item: HirItemId,
    arguments: TypeListId,
    expression: ExprId,
}

fn collect_demand_arguments(
    expression: ExprId,
    item: HirItemId,
    parameters: &[TypeParamId],
    assignments: &TypeAssignments,
    interner: &mut TypeInterner,
    budget: &mut PatternBudget,
    diagnostics: &mut Vec<InstanceCollectionError>,
) -> Option<TypeListId> {
    let (Some(pattern), Some(actual)) =
        (assignments.item(item), assignments.expression(expression))
    else {
        diagnostics.push(InstanceCollectionError::MissingType { expression });
        return None;
    };
    let mut substitution = TypeSubstitution::new();
    match match_type_pattern(interner, pattern, actual, &mut substitution, budget) {
        Ok(true) => {}
        Ok(false) => {
            diagnostics.push(InstanceCollectionError::ShapeMismatch { expression });
            return None;
        }
        Err(limit) => {
            diagnostics.push(collection_limit(expression, limit));
            return None;
        }
    }
    let arguments = parameters
        .iter()
        .map(|parameter| substitution.get(*parameter))
        .collect::<Option<Vec<_>>>()?;
    match interner.intern_list(&arguments) {
        Ok(arguments) => Some(arguments),
        Err(_) => {
            diagnostics.push(InstanceCollectionError::Capacity { expression });
            None
        }
    }
}

fn list_is_concrete(
    interner: &TypeInterner,
    arguments: TypeListId,
    budget: &mut PatternBudget,
) -> Result<bool, PatternLimit> {
    for argument in interner.list(arguments) {
        if !is_concrete_type(interner, *argument, budget)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn expression_owners(package: &HirPackage) -> Vec<Option<HirItemId>> {
    let mut owners = vec![None; package.expressions.len()];
    for (item_id, item) in package.items.enumerate() {
        let mut roots = Vec::new();
        let mut bodies = Vec::new();
        match item.kind {
            HirItemKind::Function(function) => {
                roots.extend(
                    package
                        .locals(function.parameters)
                        .iter()
                        .filter_map(|local| package.locals[*local].default),
                );
                bodies.extend(function.body);
            }
            HirItemKind::Struct(structure) => {
                roots.extend(
                    package
                        .fields(structure.fields)
                        .iter()
                        .filter_map(|field| package.fields[*field].default),
                );
            }
            HirItemKind::Enum(enumeration) => {
                for variant in package.variants(enumeration.variants) {
                    roots.extend(
                        package
                            .fields(package.variants[*variant].fields)
                            .iter()
                            .filter_map(|field| package.fields[*field].default),
                    );
                }
            }
            HirItemKind::Const { value } => roots.push(value),
            HirItemKind::Trait(_)
            | HirItemKind::Impl(_)
            | HirItemKind::Extern(_)
            | HirItemKind::TypeAlias { .. } => {}
        }
        mark_owned_expressions(package, item_id, &roots, &bodies, &mut owners);
    }
    owners
}

fn mark_owned_expressions(
    package: &HirPackage,
    owner: HirItemId,
    expression_roots: &[ExprId],
    body_roots: &[BodyId],
    owners: &mut [Option<HirItemId>],
) {
    let mut expressions = expression_roots.to_vec();
    let mut bodies = body_roots.to_vec();
    let mut seen_expressions = BTreeSet::new();
    let mut seen_bodies = BTreeSet::new();
    while !expressions.is_empty() || !bodies.is_empty() {
        while let Some(body) = bodies.pop() {
            if !seen_bodies.insert(body) {
                continue;
            }
            for statement in package.statements(package.bodies[body].statements) {
                match package.statements[*statement].kind {
                    StatementKind::Let { value, .. } => expressions.push(value),
                    StatementKind::Assign { target, value } => {
                        expressions.push(target);
                        expressions.push(value);
                    }
                    StatementKind::Expression(expression) => expressions.push(expression),
                    StatementKind::Return(value) => expressions.extend(value),
                    StatementKind::If {
                        condition,
                        then_body,
                        else_body,
                    } => {
                        expressions.push(condition);
                        bodies.push(then_body);
                        bodies.extend(else_body);
                    }
                    StatementKind::While { condition, body } => {
                        expressions.push(condition);
                        bodies.push(body);
                    }
                    StatementKind::For { iterable, body, .. } => {
                        expressions.push(iterable);
                        bodies.push(body);
                    }
                    StatementKind::Match { subject, arms } => {
                        expressions.push(subject);
                        bodies.extend(
                            package
                                .match_arms(arms)
                                .iter()
                                .map(|arm| package.match_arms[*arm].body),
                        );
                    }
                    StatementKind::Break | StatementKind::Continue => {}
                }
            }
        }
        while let Some(expression) = expressions.pop() {
            if !seen_expressions.insert(expression) {
                continue;
            }
            owners[expression.index()].get_or_insert(owner);
            match package.expressions[expression].kind {
                ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
                ExpressionKind::Unary { operand, .. } => expressions.push(operand),
                ExpressionKind::Binary { left, right, .. } => {
                    expressions.push(left);
                    expressions.push(right);
                }
                ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                    expressions.extend(package.expressions(elements));
                }
                ExpressionKind::Call { callee, arguments }
                | ExpressionKind::GenericCall {
                    callee, arguments, ..
                } => {
                    expressions.push(callee);
                    expressions.extend(package.expressions(arguments));
                }
                ExpressionKind::Field { base, .. } => expressions.push(base),
                ExpressionKind::Index { base, index } => {
                    expressions.push(base);
                    expressions.push(index);
                }
                ExpressionKind::Try(inner)
                | ExpressionKind::Await(inner)
                | ExpressionKind::Spawn(inner) => expressions.push(inner),
                ExpressionKind::Closure { body, .. } => bodies.push(body),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceCollectionError {
    MissingType {
        expression: ExprId,
    },
    ShapeMismatch {
        expression: ExprId,
    },
    WorkLimit {
        expression: ExprId,
    },
    DepthLimit {
        expression: ExprId,
    },
    Capacity {
        expression: ExprId,
    },
    Growth {
        expression: ExprId,
        error: InstanceGrowthError,
    },
}

fn collection_limit(expression: ExprId, limit: PatternLimit) -> InstanceCollectionError {
    match limit {
        PatternLimit::Work => InstanceCollectionError::WorkLimit { expression },
        PatternLimit::Depth => InstanceCollectionError::DepthLimit { expression },
    }
}

fn item_parameters(package: &HirPackage, item: HirItemId) -> &[TypeParamId] {
    let range = match package.items[item].kind {
        HirItemKind::Function(function) => Some(function.type_parameters),
        HirItemKind::Struct(structure) => Some(structure.type_parameters),
        HirItemKind::Enum(enumeration) => Some(enumeration.type_parameters),
        HirItemKind::Trait(trait_) => Some(trait_.type_parameters),
        HirItemKind::Impl(implementation) => Some(implementation.type_parameters),
        HirItemKind::Extern(_) | HirItemKind::Const { .. } | HirItemKind::TypeAlias { .. } => None,
    };
    range
        .map(|range| package.type_parameters(range))
        .unwrap_or_default()
}
