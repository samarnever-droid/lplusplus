#![forbid(unsafe_code)]

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
