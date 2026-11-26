#![forbid(unsafe_code)]

mod aggregates;
mod builtins;
mod check;
mod enum_flow;
mod ids;
mod infer;
mod instances;
mod interner;
mod pattern;
mod places;
mod snapshot;
mod substitution;
mod traits;

pub use aggregates::{AggregateConstructor, AggregateExpressionFact, AggregateFacts};
pub use builtins::{
    BuiltinFact, BuiltinFacts, BuiltinId, BuiltinIndex, semantic_result_type, semantic_spelling,
    type_matches_semantic,
};
pub use check::{
    ShadowInferenceOptions, ShadowTypeError, ShadowTypeOutput, TypeAssignments, infer_hir_package,
};
pub use enum_flow::{EnumFlowFacts, EnumMatchArmFact, EnumMatchFact, EnumTryFact};
pub use ids::{InferVarId, TraitImplId, TypeId, TypeListId};
pub use infer::{
    GeneralizedVariable, InferenceLevel, InferenceTable, InferenceVariable, TypeError, TypeScheme,
    TypeWorkBudget,
};
pub use instances::{
    InstanceCollectionError, InstanceGrowthError, InstanceKey, InstanceLimits, InstancePlanner,
    InstanceRecord, InstanceRequestKind, InstanceRequestOutcome, InstanceStats, InstanceWorkItem,
};
pub use interner::{PrimitiveType, TypeInterner, TypeInternerExhausted, TypeKind};
pub use places::{AugmentedAssignmentFact, PlaceExpressionFact, PlaceFacts, PlaceStatementFact};
pub use snapshot::{SemanticMetrics, semantic_metrics, semantic_snapshot};
pub use substitution::TypeSubstitution;
pub use traits::{
    TraitBound, TraitBuildError, TraitCoherenceError, TraitGoal, TraitIndex, TraitRule,
    TraitSelection, TraitSolution, TraitSolverLimits, TraitSolverOverflow, TraitSolverStats,
};
