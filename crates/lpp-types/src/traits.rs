use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use lpp_hir::{DefId, DefinitionKind, HirItemId, HirItemKind, HirPackage, TypeParamId};

use crate::pattern::{
    PatternBudget, PatternLimit, is_concrete_type, list_specificity, match_lists,
    match_type_pattern, pattern_lists_overlap, pattern_specificity, patterns_overlap,
    substitute_pattern_list,
};
use crate::{
    TraitImplId, TypeAssignments, TypeId, TypeInterner, TypeKind, TypeListId, TypeSubstitution,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraitGoal {
    pub trait_definition: DefId,
    pub self_type: TypeId,
    pub arguments: TypeListId,
}

impl TraitGoal {
    #[must_use]
    pub const fn new(trait_definition: DefId, self_type: TypeId, arguments: TypeListId) -> Self {
        Self {
            trait_definition,
            self_type,
            arguments,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraitBound {
    pub parameter: TypeParamId,
    pub trait_definition: DefId,
    pub arguments: TypeListId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitRule {
    pub origin: HirItemId,
    pub trait_definition: DefId,
    pub self_pattern: TypeId,
    pub trait_arguments: TypeListId,
    pub parameters: Arc<[TypeParamId]>,
    pub bounds: Arc<[TraitBound]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitSelection {
    pub implementation: TraitImplId,
    pub origin: HirItemId,
    pub substitution: TypeSubstitution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraitSolution {
    Unique(TraitSelection),
    NoSolution,
    Ambiguous(Arc<[TraitImplId]>),
    Deferred,
    Overflow(TraitSolverOverflow),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraitSolverOverflow {
    CandidateLimit,
    GoalLimit,
    WorkLimit,
    DepthLimit,
    Cycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraitSolverLimits {
    pub max_depth: usize,
    pub max_candidates_per_goal: usize,
    pub max_candidates_total: usize,
    pub max_goals: usize,
    pub max_work: usize,
}

impl Default for TraitSolverLimits {
    fn default() -> Self {
        Self {
            max_depth: 32,
            max_candidates_per_goal: 128,
            max_candidates_total: 4_096,
            max_goals: 1_024,
            max_work: 100_000,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TraitSolverStats {
    pub goals: usize,
    pub candidates: usize,
    pub cache_hits: usize,
    pub work: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraitCoherenceError {
    Overlap {
        first: HirItemId,
        second: HirItemId,
        specificity: u32,
    },
    WorkLimit,
    DepthLimit,
    Capacity,
}

impl fmt::Display for TraitCoherenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overlap {
                first,
                second,
                specificity,
            } => write!(
                formatter,
                "equally specific overlapping trait implementations {first:?} and {second:?} (specificity {specificity})"
            ),
            Self::WorkLimit => formatter.write_str("trait coherence work limit exceeded"),
            Self::DepthLimit => formatter.write_str("trait coherence depth limit exceeded"),
            Self::Capacity => formatter.write_str("trait implementation ID space exhausted"),
        }
    }
}

impl std::error::Error for TraitCoherenceError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraitBuildError {
    MissingType { implementation: HirItemId },
    InvalidTrait { implementation: HirItemId },
    Coherence(TraitCoherenceError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TypeHead {
    Any,
    Error,
    Never,
    Primitive(crate::PrimitiveType),
    Tuple,
    List,
    Map,
    Slice,
    Task,
    Function,
    Nominal(DefId),
    BoundVariable,
    InferenceVariable,
    UnresolvedName,
}

#[derive(Debug, Clone)]
struct IndexedRule {
    rule: TraitRule,
    specificity: u32,
}

#[derive(Debug, Clone)]
pub struct TraitIndex {
    rules: Vec<IndexedRule>,
    by_head: BTreeMap<(DefId, TypeHead), Vec<TraitImplId>>,
    memo: BTreeMap<TraitGoal, TraitSolution>,
    stats: TraitSolverStats,
}

impl PartialEq for TraitIndex {
    fn eq(&self, other: &Self) -> bool {
        self.rules.len() == other.rules.len()
            && self.rules.iter().zip(&other.rules).all(|(left, right)| {
                left.rule == right.rule && left.specificity == right.specificity
            })
            && self.by_head == other.by_head
    }
}

impl Eq for TraitIndex {}

impl TraitIndex {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            by_head: BTreeMap::new(),
            memo: BTreeMap::new(),
            stats: TraitSolverStats::default(),
        }
    }

    pub fn from_hir(
        package: &HirPackage,
        assignments: &TypeAssignments,
        interner: &TypeInterner,
    ) -> (Self, Vec<TraitBuildError>) {
        let mut index = Self::new();
        let mut diagnostics = Vec::new();
        for (item_id, item) in package.items.enumerate() {
            let HirItemKind::Impl(implementation) = item.kind else {
                continue;
            };
            let Some(trait_ref) = implementation.trait_ref else {
                continue;
            };
            let Some(trait_type) = assignments.type_ref(trait_ref) else {
                diagnostics.push(TraitBuildError::MissingType {
                    implementation: item_id,
                });
                continue;
            };
            let TypeKind::Nominal {
                definition: trait_definition,
                arguments: trait_arguments,
            } = interner.kind(trait_type)
            else {
                diagnostics.push(TraitBuildError::InvalidTrait {
                    implementation: item_id,
                });
                continue;
            };
            if package.names.definitions[trait_definition].kind != DefinitionKind::Trait {
                diagnostics.push(TraitBuildError::InvalidTrait {
                    implementation: item_id,
                });
                continue;
            }
            let Some(self_pattern) = assignments.type_ref(implementation.target) else {
                diagnostics.push(TraitBuildError::MissingType {
                    implementation: item_id,
                });
                continue;
            };
            let parameters = package
                .type_parameters(implementation.type_parameters)
                .to_vec();
            let mut bounds = Vec::new();
            let mut valid = true;
            for parameter in &parameters {
                let Some(bound_ref) = package.type_parameters[*parameter].bound else {
                    continue;
                };
                let Some(bound_type) = assignments.type_ref(bound_ref) else {
                    valid = false;
                    break;
                };
                let TypeKind::Nominal {
                    definition,
                    arguments,
                } = interner.kind(bound_type)
                else {
                    valid = false;
                    break;
                };
                if package.names.definitions[definition].kind != DefinitionKind::Trait {
                    valid = false;
                    break;
                }
                bounds.push(TraitBound {
                    parameter: *parameter,
                    trait_definition: definition,
                    arguments,
                });
            }
            if !valid {
                diagnostics.push(TraitBuildError::InvalidTrait {
                    implementation: item_id,
                });
                continue;
            }
            let rule = TraitRule {
                origin: item_id,
                trait_definition,
                self_pattern,
                trait_arguments,
                parameters: Arc::from(parameters),
                bounds: Arc::from(bounds),
            };
            if let Err(error) = index.register(interner, rule) {
                diagnostics.push(TraitBuildError::Coherence(error));
            }
        }
        (index, diagnostics)
    }

    pub fn register(
        &mut self,
        interner: &TypeInterner,
        rule: TraitRule,
    ) -> Result<TraitImplId, TraitCoherenceError> {
        let mut budget = PatternBudget::new(100_000, 256);
        let specificity = pattern_specificity(interner, rule.self_pattern, &mut budget)
            .and_then(|self_specificity| {
                list_specificity(interner, rule.trait_arguments, &mut budget)
                    .map(|arguments| self_specificity.saturating_add(arguments))
            })
            .map_err(coherence_limit)?;
        let head = type_head(interner, rule.self_pattern);
        for existing in &self.rules {
            let existing_head = type_head(interner, existing.rule.self_pattern);
            if existing.rule.trait_definition != rule.trait_definition
                || existing.specificity != specificity
                || (head != TypeHead::Any
                    && existing_head != TypeHead::Any
                    && head != existing_head)
            {
                continue;
            }
            let self_overlap = patterns_overlap(
                interner,
                existing.rule.self_pattern,
                rule.self_pattern,
                &mut budget,
            )
            .map_err(coherence_limit)?;
            if !self_overlap {
                continue;
            }
            let argument_overlap = pattern_lists_overlap(
                interner,
                existing.rule.trait_arguments,
                rule.trait_arguments,
                &mut budget,
            )
            .map_err(coherence_limit)?;
            if argument_overlap {
                return Err(TraitCoherenceError::Overlap {
                    first: existing.rule.origin,
                    second: rule.origin,
                    specificity,
                });
            }
        }
        let id = TraitImplId::from_index(self.rules.len()).ok_or(TraitCoherenceError::Capacity)?;
        self.by_head
            .entry((rule.trait_definition, head))
            .or_default()
            .push(id);
        self.rules.push(IndexedRule { rule, specificity });
        self.memo.clear();
        Ok(id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    #[must_use]
    pub fn rule(&self, id: TraitImplId) -> Option<&TraitRule> {
        self.rules.get(id.index()).map(|indexed| &indexed.rule)
    }

    #[must_use]
    pub fn specificity(&self, id: TraitImplId) -> Option<u32> {
        self.rules
            .get(id.index())
            .map(|indexed| indexed.specificity)
    }

    #[must_use]
    pub const fn stats(&self) -> TraitSolverStats {
        self.stats
    }

    #[must_use]
    pub fn memoized_goal_count(&self) -> usize {
        self.memo.len()
    }

    pub fn clear_memo(&mut self) {
        self.memo.clear();
        self.stats = TraitSolverStats::default();
    }

    pub fn solve(
        &mut self,
        interner: &mut TypeInterner,
        goal: TraitGoal,
        limits: TraitSolverLimits,
    ) -> TraitSolution {
        let mut state = SolverState {
            candidates_remaining: limits.max_candidates_total,
            goals_remaining: limits.max_goals,
            pattern: PatternBudget::new(limits.max_work, limits.max_depth),
            active: BTreeSet::new(),
            limits,
        };
        let before_work = state.pattern.consumed();
        let solution = self.solve_inner(interner, goal, 0, &mut state);
        self.stats.work = self
            .stats
            .work
            .saturating_add(state.pattern.consumed().saturating_sub(before_work));
        solution
    }

    fn solve_inner(
        &mut self,
        interner: &mut TypeInterner,
        goal: TraitGoal,
        depth: usize,
        state: &mut SolverState,
    ) -> TraitSolution {
        if depth > state.limits.max_depth {
            return TraitSolution::Overflow(TraitSolverOverflow::DepthLimit);
        }
        if state.goals_remaining == 0 {
            return TraitSolution::Overflow(TraitSolverOverflow::GoalLimit);
        }
        state.goals_remaining -= 1;
        self.stats.goals = self.stats.goals.saturating_add(1);
        match is_concrete_type(interner, goal.self_type, &mut state.pattern) {
            Ok(true) => {}
            Ok(false) => return TraitSolution::Deferred,
            Err(limit) => return TraitSolution::Overflow(solver_limit(limit)),
        }
        for argument in interner.list(goal.arguments) {
            match is_concrete_type(interner, *argument, &mut state.pattern) {
                Ok(true) => {}
                Ok(false) => return TraitSolution::Deferred,
                Err(limit) => return TraitSolution::Overflow(solver_limit(limit)),
            }
        }
        if let Some(solution) = self.memo.get(&goal) {
            self.stats.cache_hits = self.stats.cache_hits.saturating_add(1);
            return solution.clone();
        }
        if !state.active.insert(goal) {
            return TraitSolution::Overflow(TraitSolverOverflow::Cycle);
        }

        let head = type_head(interner, goal.self_type);
        let mut candidates = self
            .by_head
            .get(&(goal.trait_definition, head))
            .cloned()
            .unwrap_or_default();
        if head != TypeHead::Any
            && let Some(fallback) = self.by_head.get(&(goal.trait_definition, TypeHead::Any))
        {
            candidates.extend(fallback.iter().copied());
        }
        candidates.sort_unstable();
        candidates.dedup();
        if candidates.len() > state.limits.max_candidates_per_goal
            || candidates.len() > state.candidates_remaining
        {
            state.active.remove(&goal);
            return TraitSolution::Overflow(TraitSolverOverflow::CandidateLimit);
        }
        state.candidates_remaining -= candidates.len();
        self.stats.candidates = self.stats.candidates.saturating_add(candidates.len());

        let mut applicable = Vec::new();
        let mut uncertain = Vec::new();
        for candidate in candidates {
            let indexed = self.rules[candidate.index()].clone();
            let mut substitution = TypeSubstitution::new();
            let self_matches = match match_type_pattern(
                interner,
                indexed.rule.self_pattern,
                goal.self_type,
                &mut substitution,
                &mut state.pattern,
            ) {
                Ok(result) => result,
                Err(limit) => {
                    state.active.remove(&goal);
                    return TraitSolution::Overflow(solver_limit(limit));
                }
            };
            if !self_matches {
                continue;
            }
            let arguments_match = match match_lists(
                interner,
                indexed.rule.trait_arguments,
                goal.arguments,
                &mut substitution,
                &mut state.pattern,
                depth + 1,
            ) {
                Ok(result) => result,
                Err(limit) => {
                    state.active.remove(&goal);
                    return TraitSolution::Overflow(solver_limit(limit));
                }
            };
            if !arguments_match {
                continue;
            }
            let mut proven = true;
            for bound in indexed.rule.bounds.iter() {
                let Some(self_type) = substitution.get(bound.parameter) else {
                    proven = false;
                    break;
                };
                let arguments = match substitute_pattern_list(
                    interner,
                    bound.arguments,
                    &substitution,
                    &mut state.pattern,
                ) {
                    Ok(arguments) => arguments,
                    Err(limit) => {
                        state.active.remove(&goal);
                        return TraitSolution::Overflow(solver_limit(limit));
                    }
                };
                let bound_goal = TraitGoal::new(bound.trait_definition, self_type, arguments);
                match self.solve_inner(interner, bound_goal, depth + 1, state) {
                    TraitSolution::Unique(_) => {}
                    TraitSolution::NoSolution => {
                        proven = false;
                        break;
                    }
                    TraitSolution::Overflow(overflow) => {
                        state.active.remove(&goal);
                        return TraitSolution::Overflow(overflow);
                    }
                    TraitSolution::Ambiguous(_) | TraitSolution::Deferred => {
                        uncertain.push((candidate, indexed.specificity));
                        proven = false;
                        break;
                    }
                }
            }
            if proven {
                applicable.push((
                    candidate,
                    indexed.specificity,
                    substitution,
                    indexed.rule.origin,
                ));
            }
        }

        state.active.remove(&goal);
        let solution = if applicable.is_empty() {
            if uncertain.is_empty() {
                TraitSolution::NoSolution
            } else {
                let mut candidates = uncertain
                    .into_iter()
                    .map(|(candidate, _)| candidate)
                    .collect::<Vec<_>>();
                candidates.sort_unstable();
                candidates.dedup();
                TraitSolution::Ambiguous(Arc::from(candidates))
            }
        } else {
            let max_specificity = applicable
                .iter()
                .map(|(_, specificity, _, _)| *specificity)
                .max()
                .expect("applicable candidates are nonempty");
            let mut best = applicable
                .into_iter()
                .filter(|(_, specificity, _, _)| *specificity == max_specificity)
                .collect::<Vec<_>>();
            best.sort_by_key(|(candidate, _, _, _)| *candidate);
            let relevant_uncertain = uncertain
                .into_iter()
                .filter(|(_, specificity)| *specificity >= max_specificity)
                .map(|(candidate, _)| candidate)
                .collect::<Vec<_>>();
            if best.len() == 1 && relevant_uncertain.is_empty() {
                let (implementation, _, substitution, origin) = best.pop().unwrap();
                TraitSolution::Unique(TraitSelection {
                    implementation,
                    origin,
                    substitution,
                })
            } else {
                let mut candidates = best
                    .into_iter()
                    .map(|(candidate, _, _, _)| candidate)
                    .chain(relevant_uncertain)
                    .collect::<Vec<_>>();
                candidates.sort_unstable();
                candidates.dedup();
                TraitSolution::Ambiguous(Arc::from(candidates))
            }
        };
        if !matches!(
            solution,
            TraitSolution::Overflow(_) | TraitSolution::Deferred
        ) {
            self.memo.insert(goal, solution.clone());
        }
        solution
    }
}

impl Default for TraitIndex {
    fn default() -> Self {
        Self::new()
    }
}

struct SolverState {
    candidates_remaining: usize,
    goals_remaining: usize,
    pattern: PatternBudget,
    active: BTreeSet<TraitGoal>,
    limits: TraitSolverLimits,
}

fn type_head(interner: &TypeInterner, type_id: TypeId) -> TypeHead {
    match interner.kind(type_id) {
        TypeKind::Error => TypeHead::Error,
        TypeKind::Never => TypeHead::Never,
        TypeKind::Primitive(primitive) => TypeHead::Primitive(primitive),
        TypeKind::Tuple(_) => TypeHead::Tuple,
        TypeKind::List(_) => TypeHead::List,
        TypeKind::Map { .. } => TypeHead::Map,
        TypeKind::Slice(_) => TypeHead::Slice,
        TypeKind::Task(_) => TypeHead::Task,
        TypeKind::Function { .. } => TypeHead::Function,
        TypeKind::Nominal { definition, .. } => TypeHead::Nominal(definition),
        TypeKind::GenericParameter(_) => TypeHead::Any,
        TypeKind::BoundVariable(_) => TypeHead::BoundVariable,
        TypeKind::InferenceVariable(_) => TypeHead::InferenceVariable,
        TypeKind::UnresolvedName { .. } => TypeHead::UnresolvedName,
    }
}

const fn coherence_limit(limit: PatternLimit) -> TraitCoherenceError {
    match limit {
        PatternLimit::Work => TraitCoherenceError::WorkLimit,
        PatternLimit::Depth => TraitCoherenceError::DepthLimit,
    }
}

const fn solver_limit(limit: PatternLimit) -> TraitSolverOverflow {
    match limit {
        PatternLimit::Work => TraitSolverOverflow::WorkLimit,
        PatternLimit::Depth => TraitSolverOverflow::DepthLimit,
    }
}
