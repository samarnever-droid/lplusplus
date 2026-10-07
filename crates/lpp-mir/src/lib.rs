#![forbid(unsafe_code)]
#![allow(
    clippy::collapsible_if,
    clippy::manual_range_contains,
    clippy::match_like_matches_macro,
    clippy::needless_lifetimes,
    clippy::manual_is_ascii_check,
    clippy::from_str_radix_10,
    clippy::too_many_arguments,
    clippy::manual_contains,
    clippy::needless_borrow,
    clippy::question_mark
)]

mod builder;
mod ids;
mod interpret;
mod ir;
mod snapshot;
mod storage;
mod verify;

pub use builder::{
    MirBuildError, MirBuildErrorKind, MirBuildOptions, MirCapacity, TypedEntity,
    UnsupportedConstruct, build_mir,
};
pub use ids::{
    BasicBlockId, InstructionId, MirAggregateId, MirFieldId, MirFunctionId, MirLocalId, MirPlaceId,
    MirStringId, MirVariantId,
};
pub use interpret::{
    ArcStats, ExecutionOutcome, ExecutionStats, ExecutionValue, ExecutionValueKind,
    InterpreterError, InterpreterErrorKind, InterpreterLimit, InterpreterLimits, execute_mir,
    execute_mir_arc, execute_mir_with_stats,
};
pub use ir::{
    BasicBlock, Constant, Instruction, InstructionKind, MirAggregate, MirAggregateKind, MirField,
    MirFunction, MirFunctionKind, MirLocal, MirLocalKind, MirPlace, MirProgram, MirVariant,
    Operand, PlaceProjection, Rvalue, Terminator,
};
pub use lpp_hir::{BinaryOperator, UnaryOperator};
pub use snapshot::mir_snapshot;
pub use storage::{ListRange, StorageExhausted};
pub use verify::{
    CORE_MIR_INVARIANTS, DefiniteInitializationLimits, DefiniteInitializationResource, MirEntity,
    MirInvariant, MirVerificationError, MirVerificationErrorKind, verify_mir,
    verify_mir_with_limits,
};
