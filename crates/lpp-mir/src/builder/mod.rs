use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use lpp_common::{SourceMap, Span};
use lpp_hir::{
    ArenaId, BodyId, ExprId, ExpressionKind, HirItemId, HirItemKind, HirPackage, IdRange,
    InstanceId, Literal, LocalId, MatchArmId, NameBinding, OriginId, ScopeId, StatementKind,
    StmtId, TypeRefId,
};
use lpp_types::{
    AggregateConstructor, AggregateExpressionFact, InstanceKey, InstanceRequestKind, PrimitiveType,
    ShadowTypeOutput, TypeError, TypeId, TypeKind, TypeListId, TypeSubstitution, TypeWorkBudget,
};

use crate::ids::{
    BasicBlockId, MirAggregateId, MirExternalFunctionId, MirFieldId, MirFunctionId, MirLocalId,
    MirPlaceId, MirStringId, MirVariantId,
};
use crate::ir::{
    BasicBlock, Constant, Instruction, InstructionKind, MirAggregate, MirAggregateKind,
    MirExternalFunction, MirField, MirFunction, MirFunctionKind, MirLocal, MirLocalKind, MirPlace,
    MirProgram, MirVariant, Operand, PlaceProjection, Rvalue, Terminator,
};
use crate::storage::{ListRange, StorageExhausted};

mod aggregate;
mod expression;
mod function;
mod statement;

use function::FunctionDescriptor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirBuildOptions {
    pub max_functions: usize,
    pub max_aggregates: usize,
    pub max_aggregate_fields: usize,
    pub max_aggregate_variants: usize,
    pub max_blocks: usize,
    pub max_switch_targets: usize,
    pub max_locals: usize,
    pub max_places: usize,
    pub max_projections: usize,
    pub max_instructions: usize,
    pub max_operands: usize,
    pub max_expression_depth: usize,
    pub max_type_work: usize,
    pub max_type_depth: usize,
    pub max_strings: usize,
}

impl Default for MirBuildOptions {
    fn default() -> Self {
        Self {
            max_functions: 100_000,
            max_aggregates: 100_000,
            max_aggregate_fields: 1_000_000,
            max_aggregate_variants: 1_000_000,
            max_blocks: 1_000_000,
            max_switch_targets: 4_000_000,
            max_locals: 4_000_000,
            max_places: 8_000_000,
            max_projections: 16_000_000,
            max_instructions: 8_000_000,
            max_operands: 16_000_000,
            max_expression_depth: 256,
            max_type_work: 8_000_000,
            max_type_depth: TypeWorkBudget::DEFAULT_MAX_DEPTH,
            max_strings: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirCapacity {
    Functions,
    Aggregates,
    AggregateFields,
    AggregateVariants,
    Blocks,
    SwitchTargets,
    Locals,
    Places,
    Projections,
    Instructions,
    Operands,
    ExpressionDepth,
    TypeWork,
    TypeDepth,
    Strings,
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedConstruct {
    GenericFunction,
    AsyncFunction,
    VariadicFunction,
    GenericCall,
    Field,
    Index,
    Await,
    Spawn,
    Closure,
    Destructuring,
    FieldOrIndexAssignment,
    For,
    ExternalFunction,
    ModuleValue,
    UnresolvedName,
    FormattedString,
    ParameterDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedEntity {
    Function(HirItemId),
    Local(LocalId),
    Expression(ExprId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirBuildErrorKind {
    Capacity(MirCapacity),
    Unsupported(UnsupportedConstruct),
    MissingType(TypedEntity),
    InvalidFunctionType(TypeId),
    InvalidExternalAbi,
    InvalidExternalLibrary,
    UnsupportedExternalSignature,
    InvalidAssignmentTarget,
    MissingReturn,
    InvalidInstancePlan {
        diagnostics: usize,
    },
    InvalidInstanceArity {
        instance: InstanceId,
        expected: usize,
        actual: usize,
    },
    MissingFunctionInstance {
        item: HirItemId,
        ty: TypeId,
    },
    MissingExplicitInstance {
        item: HirItemId,
    },
    MissingAggregateFact {
        expression: ExprId,
    },
    MissingPlaceFact {
        expression: ExprId,
    },
    MissingStatementPlaceFact {
        statement: StmtId,
    },
    MissingEnumMatchFact {
        statement: StmtId,
    },
    MissingEnumArmFact {
        arm: MatchArmId,
    },
    MissingEnumTryFact {
        expression: ExprId,
    },
    InvalidAggregateType(TypeId),
    GenericTypeMaterialization,
    MissingBuiltinFact {
        expression: ExprId,
    },
    InvalidBuiltinArity {
        expected: usize,
        actual: usize,
    },
    InvalidLiteral,
    InvalidAwaitTarget(TypeId),
    InvalidSpawnTarget(TypeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirBuildError {
    pub origin: OriginId,
    pub kind: MirBuildErrorKind,
}

impl MirBuildError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            MirBuildErrorKind::Capacity(_) => "E4001",
            MirBuildErrorKind::Unsupported(_)
            | MirBuildErrorKind::InvalidExternalAbi
            | MirBuildErrorKind::InvalidExternalLibrary
            | MirBuildErrorKind::UnsupportedExternalSignature => "E4002",
            MirBuildErrorKind::MissingType(_) | MirBuildErrorKind::InvalidFunctionType(_) => {
                "E4003"
            }
            MirBuildErrorKind::InvalidAssignmentTarget => "E4004",
            MirBuildErrorKind::MissingReturn => "E4005",
            MirBuildErrorKind::InvalidInstancePlan { .. }
            | MirBuildErrorKind::InvalidInstanceArity { .. }
            | MirBuildErrorKind::MissingFunctionInstance { .. }
            | MirBuildErrorKind::MissingExplicitInstance { .. }
            | MirBuildErrorKind::MissingAggregateFact { .. }
            | MirBuildErrorKind::MissingPlaceFact { .. }
            | MirBuildErrorKind::MissingStatementPlaceFact { .. }
            | MirBuildErrorKind::MissingEnumMatchFact { .. }
            | MirBuildErrorKind::MissingEnumArmFact { .. }
            | MirBuildErrorKind::MissingEnumTryFact { .. }
            | MirBuildErrorKind::InvalidAggregateType(_)
            | MirBuildErrorKind::GenericTypeMaterialization
            | MirBuildErrorKind::MissingBuiltinFact { .. }
            | MirBuildErrorKind::InvalidBuiltinArity { .. }
            | MirBuildErrorKind::InvalidLiteral
            | MirBuildErrorKind::InvalidAwaitTarget(_)
            | MirBuildErrorKind::InvalidSpawnTarget(_) => "E4006",
        }
    }
}

impl fmt::Display for MirBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {:?}", self.code(), self.kind)
    }
}

impl std::error::Error for MirBuildError {}

#[derive(Debug, Default)]
struct BuildCounts {
    functions: usize,
    aggregates: usize,
    aggregate_fields: usize,
    aggregate_variants: usize,
    blocks: usize,
    switch_targets: usize,
    locals: usize,
    places: usize,
    projections: usize,
    instructions: usize,
    operands: usize,
    strings: usize,
}

#[derive(Debug, Clone)]
struct FunctionWork {
    item: HirItemId,
    instance: Option<InstanceId>,
    instance_arguments: TypeListId,
    function_type: TypeId,
    substitution: TypeSubstitution,
}
/// A closure body waiting for its enclosing function to finish lowering.
struct ClosureWork {
    expression: ExprId,
    parameters: IdRange<LocalId>,
    body: BodyId,
    enclosing_source: HirItemId,
    substitution: TypeSubstitution,
    capture_locals: Vec<MirLocalId>,
    function_type: TypeId,
    function_id: MirFunctionId,
}

pub fn build_mir(
    package: &HirPackage,
    sources: &SourceMap,
    types: &mut ShadowTypeOutput,
    options: MirBuildOptions,
) -> Result<MirProgram, MirBuildError> {
    MirBuilder::new(package, sources, types, options).build()
}

struct MirBuilder<'input> {
    package: &'input HirPackage,
    sources: &'input SourceMap,
    types: &'input mut ShadowTypeOutput,
    options: MirBuildOptions,
    type_budget: TypeWorkBudget,
    counts: BuildCounts,
    program: MirProgram,
    item_functions: Vec<Option<MirFunctionId>>,
    definition_items: Vec<Option<HirItemId>>,
    definition_functions: Vec<Option<MirFunctionId>>,
    external_items: BTreeMap<HirItemId, (Span, Option<Span>)>,
    external_function_ids: BTreeMap<HirItemId, MirExternalFunctionId>,
    instance_functions: Vec<Option<MirFunctionId>>,
    instance_type_functions: BTreeMap<(HirItemId, TypeId), Option<MirFunctionId>>,
    aggregate_types: BTreeMap<TypeId, MirAggregateId>,
    aggregate_fields: BTreeMap<(MirAggregateId, lpp_hir::FieldId), MirFieldId>,
    aggregate_variants: BTreeMap<(MirAggregateId, lpp_hir::VariantId), MirVariantId>,
    local_map: Vec<Option<MirLocalId>>,
    /// Closure bodies discovered during a lowering pass; lowered after the
    /// enclosing function finishes so function IDs keep reservation order.
    pending_closures: std::collections::VecDeque<ClosureWork>,
    string_cache: BTreeMap<String, MirStringId>,
    /// HIR locals written by a closure that captures them. Such a local is
    /// lowered as a by-reference capture cell: a single-element list shared
    /// between the enclosing frame and every capturing closure, so the
    /// closure's write is visible where the local is read.
    cell_sources: BTreeSet<LocalId>,
}

mod build_functions;
mod captures;
mod types;

fn unescape_literal(input: &str) -> Option<String> {
    let mut decoded = String::with_capacity(input.len());
    let mut characters = input.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let Some(escaped) = characters.next() else {
            return None;
        };
        match escaped {
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            '0' => decoded.push('\0'),
            '"' => decoded.push('"'),
            '\'' => decoded.push('\''),
            '\\' => decoded.push('\\'),
            // Escaped braces are legal only inside formatted strings, where the
            // lexer accepts `\{`/`\}`; decode them to literal braces.
            '{' => decoded.push('{'),
            '}' => decoded.push('}'),
            'x' => {
                let high = characters.next()?;
                let low = characters.next()?;
                let value = u32::from_str_radix(&format!("{high}{low}"), 16).ok()?;
                decoded.push(char::from_u32(value)?);
            }
            _ => return None,
        }
    }
    Some(decoded)
}

#[derive(Debug)]
struct BlockDraft {
    instructions: Vec<Instruction>,
    terminator: Option<DraftTerminator>,
    origin: OriginId,
}

#[derive(Debug)]
enum DraftTerminator {
    Goto(usize),
    Branch {
        condition: Operand,
        then_block: usize,
        else_block: usize,
    },
    SwitchEnum {
        subject: Operand,
        aggregate: MirAggregateId,
        targets: Vec<usize>,
    },
    Return(Option<Operand>),
    Unreachable,
}

struct FunctionBuilder<'builder, 'input> {
    core: &'builder mut MirBuilder<'input>,
    source: HirItemId,
    substitution: TypeSubstitution,
    type_cache: BTreeMap<TypeId, TypeId>,
    mapped_sources: Vec<LocalId>,
    return_type: TypeId,
    locals: Vec<MirLocalId>,
    parameters: Vec<MirLocalId>,
    blocks: Vec<BlockDraft>,
    current: Option<usize>,
    pending_switch_targets: usize,
    loops: Vec<(usize, usize)>,
}
