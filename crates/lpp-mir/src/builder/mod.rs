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
    BasicBlockId, MirAggregateId, MirFieldId, MirFunctionId, MirLocalId, MirPlaceId, MirStringId,
    MirVariantId,
};
use crate::ir::{
    BasicBlock, Constant, Instruction, InstructionKind, MirAggregate, MirAggregateKind, MirField,
    MirFunction, MirFunctionKind, MirLocal, MirLocalKind, MirPlace, MirProgram, MirVariant,
    Operand, PlaceProjection, Rvalue, Terminator,
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
            MirBuildErrorKind::Unsupported(_) => "E4002",
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

impl<'input> MirBuilder<'input> {
    fn new(
        package: &'input HirPackage,
        sources: &'input SourceMap,
        types: &'input mut ShadowTypeOutput,
        options: MirBuildOptions,
    ) -> Self {
        let instance_count = types.instances.records().len();
        Self {
            package,
            sources,
            types,
            options,
            type_budget: TypeWorkBudget::with_max_depth(
                options.max_type_work,
                options.max_type_depth,
            ),
            counts: BuildCounts::default(),
            program: MirProgram::with_capacity(
                package.items.len(),
                package.locals.len(),
                package.expressions.len(),
                package.statements.len(),
            ),
            item_functions: vec![None; package.items.len()],
            definition_items: vec![None; package.names.definitions.len()],
            definition_functions: vec![None; package.names.definitions.len()],
            instance_functions: vec![None; instance_count],
            instance_type_functions: BTreeMap::new(),
            aggregate_types: BTreeMap::new(),
            aggregate_fields: BTreeMap::new(),
            aggregate_variants: BTreeMap::new(),
            local_map: vec![None; package.locals.len()],
            string_cache: BTreeMap::new(),
            pending_closures: std::collections::VecDeque::new(),
            cell_sources: BTreeSet::new(),
        }
    }

    fn build(mut self) -> Result<MirProgram, MirBuildError> {
        let fallback_origin = self
            .package
            .items
            .iter()
            .next()
            .map(|item| item.origin)
            .expect("a typed HIR package has at least one item");
        if !self.types.instance_diagnostics.is_empty() || self.types.instances.finish().is_err() {
            return Err(self.error(
                fallback_origin,
                MirBuildErrorKind::InvalidInstancePlan {
                    diagnostics: self.types.instance_diagnostics.len(),
                },
            ));
        }
        for (item_id, item) in self.package.items.enumerate() {
            if let Some(definition) = item.definition {
                self.definition_items[definition.index()] = Some(item_id);
            }
        }

        let mut work = Vec::new();
        for (item_id, item) in self.package.items.enumerate() {
            let HirItemKind::Function(function) = item.kind else {
                continue;
            };
            if function.body.is_none() || !function.type_parameters.is_empty() {
                continue;
            }
            self.validate_function_shape(function, item.origin)?;
            let function_type = self.type_of_item(item_id, item.origin)?;
            let function_id = self.reserve_function(work.len(), item.origin)?;
            self.item_functions[item_id.index()] = Some(function_id);
            if let Some(definition) = item.definition {
                self.definition_functions[definition.index()] = Some(function_id);
            }
            work.push(FunctionWork {
                item: item_id,
                instance: None,
                instance_arguments: self.types.interner.empty_list(),
                function_type,
                substitution: TypeSubstitution::new(),
            });
        }

        let records = self.types.instances.records().to_vec();
        for record in records {
            if record.kind != InstanceRequestKind::Required {
                continue;
            }
            let item = self.package.items[record.key.item];
            let HirItemKind::Function(function) = item.kind else {
                continue;
            };
            if function.body.is_none() || function.type_parameters.is_empty() {
                continue;
            }
            self.validate_function_shape(function, item.origin)?;
            let parameters = self
                .package
                .type_parameters(function.type_parameters)
                .to_vec();
            let arguments = self.types.interner.list(record.key.arguments).to_vec();
            if parameters.len() != arguments.len() {
                return Err(self.error(
                    record.origin.unwrap_or(item.origin),
                    MirBuildErrorKind::InvalidInstanceArity {
                        instance: record.id,
                        expected: parameters.len(),
                        actual: arguments.len(),
                    },
                ));
            }
            let mut substitution = TypeSubstitution::new();
            for (parameter, argument) in parameters.iter().zip(arguments) {
                substitution.insert(*parameter, argument);
            }
            let template_type = self.type_of_item(record.key.item, item.origin)?;
            let function_type = self.materialize_type(&substitution, template_type, item.origin)?;
            let function_id = self.reserve_function(work.len(), item.origin)?;
            self.instance_functions[record.id.index()] = Some(function_id);
            self.instance_type_functions
                .entry((record.key.item, function_type))
                .and_modify(|entry| *entry = None)
                .or_insert(Some(function_id));
            work.push(FunctionWork {
                item: record.key.item,
                instance: Some(record.id),
                instance_arguments: record.key.arguments,
                function_type,
                substitution,
            });
        }

        for function in work {
            self.lower_function(function)?;
        }
        // Drain the closure queue once, after every item is lowered:
        // items are reserved first, then closures in encounter order, so
        // a single FIFO drain allocates every function in reservation
        // order. Draining per item would interleave closure allocations
        // with later item allocations and break the ID numbering.
        self.lower_pending_closures()?;
        self.ensure_program_aggregate_types(fallback_origin)?;
        Ok(self.program)
    }

    fn lower_function(&mut self, work: FunctionWork) -> Result<(), MirBuildError> {
        let item = self.package.items[work.item];
        let HirItemKind::Function(function) = item.kind else {
            unreachable!("the function worklist contains only functions")
        };
        let TypeKind::Function {
            parameters: _,
            result,
        } = self.types.interner.kind(work.function_type)
        else {
            return Err(self.error(
                item.origin,
                MirBuildErrorKind::InvalidFunctionType(work.function_type),
            ));
        };
        // Async functions expose `fn(...) -> Task<R>`; the body itself
        // produces `R`, so the MIR function's return type is `R`.
        let return_type = if function.is_async {
            let TypeKind::Task(return_type) = self.types.interner.kind(result) else {
                return Err(self.error(
                    item.origin,
                    MirBuildErrorKind::InvalidFunctionType(work.function_type),
                ));
            };
            return_type
        } else {
            result
        };
        let body = function.body.ok_or_else(|| {
            self.error(
                item.origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
            )
        })?;
        let entry_origin = self.package.bodies[body].origin;
        let parameters = self.package.locals(function.parameters).to_vec();
        // Default parameters are supported: the body sees every parameter as a
        // normal local, and call sites fill omitted trailing arguments with the
        // parameter's default expression (see `lower_defaulted_call`).
        let function_id = match work.instance {
            Some(instance) => self.instance_functions[instance.index()],
            None => self.item_functions[work.item.index()],
        }
        .expect("the function was assigned an identity before lowering");
        // Mark by-reference capture cells before any local of this body
        // is materialized: the mark changes how each local is typed and
        // referenced throughout the frame.
        let parameter_sources: BTreeSet<LocalId> = parameters.iter().copied().collect();
        self.mark_write_captures(body, &parameter_sources);
        let mut function_builder = FunctionBuilder {
            core: self,
            source: work.item,
            substitution: work.substitution,
            type_cache: BTreeMap::new(),
            mapped_sources: Vec::new(),
            return_type,
            locals: Vec::new(),
            parameters: Vec::new(),
            blocks: Vec::new(),
            current: None,
            pending_switch_targets: 0,
            loops: Vec::new(),
        };
        let entry = function_builder.new_block(entry_origin)?;
        function_builder.current = Some(entry);
        for parameter in parameters {
            let local = function_builder.local(parameter, MirLocalKind::Parameter)?;
            function_builder.parameters.push(local);
        }
        function_builder.lower_body(body)?;
        let kind = if function.is_async {
            MirFunctionKind::Async
        } else {
            MirFunctionKind::Function
        };
        function_builder.finish(
            FunctionDescriptor {
                source: work.item,
                instance: work.instance,
                instance_arguments: work.instance_arguments,
                definition: item.definition,
                name: item.name,
                kind,
                captures: Vec::new(),
                origin: item.origin,
            },
            function_id,
            work.function_type,
            entry,
        )
    }

    fn validate_function_shape(
        &self,
        _function: lpp_hir::Function,
        _origin: OriginId,
    ) -> Result<(), MirBuildError> {
        // Variadic functions are supported: the type checker rewrites the rest
        // parameter to `List[element]`, so the definition lowers like any other
        // function with a trailing list parameter, and call sites collect the
        // trailing arguments into that list (see `lower_variadic_call`).
        Ok(())
    }

    /// Reserve the closure's MIR function ID and queue its body for
    /// lowering. The body finishes after the enclosing function so function
    /// IDs keep reservation order while allocation order follows body
    /// completion. The closure frame begins with one capture local per
    /// enclosing local in `capture_locals` (in HIR declaration order),
    /// followed by the closure parameters.
    fn lower_closure_function(
        &mut self,
        expression: ExprId,
        parameters: IdRange<LocalId>,
        body: BodyId,
        enclosing_source: HirItemId,
        substitution: TypeSubstitution,
        capture_locals: Vec<MirLocalId>,
        function_type: TypeId,
    ) -> Result<MirFunctionId, MirBuildError> {
        let ExpressionKind::Closure { .. } = self.package.expressions[expression].kind else {
            unreachable!("closure lowering receives closure expressions only");
        };
        let TypeKind::Function {
            parameters: _,
            result: _,
        } = self.types.interner.kind(function_type)
        else {
            return Err(self.error(
                self.package.expressions[expression].origin,
                MirBuildErrorKind::InvalidFunctionType(function_type),
            ));
        };
        let origin = self.package.expressions[expression].origin;
        let function_id = self.reserve_function(self.counts.functions, origin)?;
        self.pending_closures.push_back(ClosureWork {
            expression,
            parameters,
            body,
            enclosing_source,
            substitution,
            capture_locals,
            function_type,
            function_id,
        });
        Ok(function_id)
    }

    /// Lower every closure body queued during the enclosing pass, in
    /// reservation order, so function IDs keep their reserved numbering.
    fn lower_pending_closures(&mut self) -> Result<(), MirBuildError> {
        while let Some(work) = self.pending_closures.pop_front() {
            self.lower_queued_closure(work)?;
        }
        Ok(())
    }

    fn lower_queued_closure(&mut self, work: ClosureWork) -> Result<(), MirBuildError> {
        let ClosureWork {
            expression,
            parameters,
            body,
            enclosing_source,
            substitution,
            capture_locals,
            function_type,
            function_id,
        } = work;
        let TypeKind::Function {
            parameters: _,
            result,
        } = self.types.interner.kind(function_type)
        else {
            return Err(self.error(
                self.package.expressions[expression].origin,
                MirBuildErrorKind::InvalidFunctionType(function_type),
            ));
        };
        let origin = self.package.expressions[expression].origin;

        // Pair each enclosing MIR local with the source local it mirrors so
        // the closure body's name references (still keyed by the original
        // LocalId) map into this frame.
        let mut capture_sources: Vec<LocalId> = Vec::with_capacity(capture_locals.len());
        for enclosing in &capture_locals {
            let source = self
                .program
                .local(*enclosing)
                .and_then(|local| local.source)
                .expect("enclosing captures reference source locals");
            capture_sources.push(source);
        }

        // The closure frame owns one writable slot per captured source local,
        // initialized from the enclosing frame at creation time. Capture
        // slots precede the closure parameters in frame order.
        let mut capture_frame_locals = Vec::with_capacity(capture_sources.len());
        for source in &capture_sources {
            let hir_local = self.package.locals[*source];
            let ty = self.types.assignments.local(*source).ok_or_else(|| {
                self.error(
                    hir_local.origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Local(*source)),
                )
            })?;
            let ty = self.materialize_type(&substitution, ty, hir_local.origin)?;
            // A by-reference capture cell continues into this frame: the
            // capture local is the cell (list) itself, so reads and
            // stores in the body go through the shared element.
            let ty = if self.cell_sources.contains(source) {
                self.cell_type(ty, hir_local.origin)?
            } else {
                ty
            };
            self.check_next(
                MirCapacity::Locals,
                self.counts.locals,
                self.options.max_locals,
                hir_local.origin,
            )?;
            let capture = self
                .program
                .locals
                .alloc(MirLocal {
                    ty,
                    source: Some(*source),
                    kind: MirLocalKind::Capture,
                    mutable: true,
                    origin: hir_local.origin,
                })
                .map_err(|_| {
                    self.error(
                        hir_local.origin,
                        MirBuildErrorKind::Capacity(MirCapacity::Storage),
                    )
                })?;
            self.counts.locals += 1;
            capture_frame_locals.push(capture);
        }

        let closure_parameters = self.package.locals(parameters).to_vec();
        for parameter in &closure_parameters {
            if self.package.locals[*parameter].default.is_some() {
                return Err(self.error(
                    self.package.locals[*parameter].origin,
                    MirBuildErrorKind::Unsupported(UnsupportedConstruct::ParameterDefault),
                ));
            }
        }
        let entry_origin = self.package.bodies[body].origin;
        let empty_list = self.types.interner.empty_list();

        // Map the captured source locals into this frame before the body
        // lowers; restore the enclosing mapping afterwards so the enclosing
        // function keeps its own locals.
        let mut remapped: Vec<(LocalId, Option<MirLocalId>)> = Vec::new();
        for (source, capture) in capture_sources.iter().zip(capture_frame_locals.iter()) {
            remapped.push((*source, self.local_map[source.index()]));
            self.local_map[source.index()] = Some(*capture);
        }
        let mut builder = FunctionBuilder {
            core: self,
            source: enclosing_source,
            substitution,
            type_cache: BTreeMap::new(),
            mapped_sources: Vec::new(),
            return_type: result,
            locals: capture_frame_locals.clone(),
            parameters: capture_frame_locals.clone(),
            blocks: Vec::new(),
            current: None,
            pending_switch_targets: 0,
            loops: Vec::new(),
        };

        for parameter in closure_parameters {
            let local = builder.local(parameter, MirLocalKind::Parameter)?;
            builder.parameters.push(local);
        }

        let entry = builder.new_block(entry_origin)?;
        builder.current = Some(entry);
        builder.lower_body(body)?;
        let descriptor = FunctionDescriptor {
            source: enclosing_source,
            instance: None,
            instance_arguments: empty_list,
            definition: None,
            name: None,
            kind: MirFunctionKind::Closure,
            captures: capture_locals,
            origin,
        };
        builder.finish(descriptor, function_id, function_type, entry)?;
        for (source, previous) in remapped {
            self.local_map[source.index()] = previous;
        }
        Ok(())
    }

    fn compute_captures(&self, body: BodyId, scope: ScopeId) -> Vec<LocalId> {
        let mut referenced: BTreeSet<LocalId> = BTreeSet::new();
        let mut visit = |expression: ExprId| {
            if let ExpressionKind::Name {
                binding: NameBinding::Local(local),
                ..
            } = self.package.expressions[expression].kind
            {
                referenced.insert(local);
            }
        };
        self.walk_body(body, &mut visit);
        let mut captures = Vec::new();
        for local in referenced {
            if !Self::scope_is_inside(&self.package, self.package.locals[local].scope, scope) {
                captures.push(local);
            }
        }
        captures
    }

    fn walk_body(&self, body: BodyId, visit: &mut dyn FnMut(ExprId)) {
        for statement in self
            .package
            .statements(self.package.bodies[body].statements)
        {
            self.walk_statement(*statement, visit);
        }
    }

    fn walk_statement(&self, statement: StmtId, visit: &mut dyn FnMut(ExprId)) {
        match self.package.statements[statement].kind {
            StatementKind::Let { bindings: _, value } => self.walk_expression(value, visit),
            StatementKind::Assign { target, value } => {
                self.walk_expression(target, visit);
                self.walk_expression(value, visit);
            }
            StatementKind::Expression(expression) => self.walk_expression(expression, visit),
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    self.walk_expression(value, visit);
                }
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.walk_expression(condition, visit);
                self.walk_body(then_body, visit);
                if let Some(else_body) = else_body {
                    self.walk_body(else_body, visit);
                }
            }
            StatementKind::While { condition, body } => {
                self.walk_expression(condition, visit);
                self.walk_body(body, visit);
            }
            StatementKind::For {
                binding: _,
                iterable,
                body,
            } => {
                self.walk_expression(iterable, visit);
                self.walk_body(body, visit);
            }
            StatementKind::Match { subject, arms } => {
                self.walk_expression(subject, visit);
                for arm in self.package.match_arms(arms) {
                    self.walk_body(self.package.match_arms[*arm].body, visit);
                }
            }
            StatementKind::Break | StatementKind::Continue => {}
        }
    }

    fn walk_expression(&self, expression: ExprId, visit: &mut dyn FnMut(ExprId)) {
        visit(expression);
        match self.package.expressions[expression].kind {
            ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
            ExpressionKind::Unary { operand, .. } => self.walk_expression(operand, visit),
            ExpressionKind::Binary { left, right, .. } => {
                self.walk_expression(left, visit);
                self.walk_expression(right, visit);
            }
            ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                for element in self.package.expressions(elements) {
                    self.walk_expression(*element, visit);
                }
            }
            ExpressionKind::Call { callee, arguments } => {
                self.walk_expression(callee, visit);
                for argument in self.package.expressions(arguments) {
                    self.walk_expression(*argument, visit);
                }
            }
            ExpressionKind::GenericCall {
                callee,
                type_arguments: _,
                arguments,
            } => {
                self.walk_expression(callee, visit);
                for argument in self.package.expressions(arguments) {
                    self.walk_expression(*argument, visit);
                }
            }
            ExpressionKind::Field { base, .. } => self.walk_expression(base, visit),
            ExpressionKind::Index { base, index } => {
                self.walk_expression(base, visit);
                self.walk_expression(index, visit);
            }
            ExpressionKind::Try(inner)
            | ExpressionKind::Await(inner)
            | ExpressionKind::Spawn(inner) => self.walk_expression(inner, visit),
            ExpressionKind::Closure {
                parameters: _,
                return_type: _,
                body,
            } => self.walk_body(body, visit),
        }
    }

    /// By-reference capture cells: before any local of `body` is
    /// materialized, mark every local that a closure contained in `body`
    /// (at any nesting depth) assigns to. The mark makes the local lower
    /// as a single-element list shared with the closure. A local that is
    /// a parameter of any enclosing function is never marked: the call
    /// ABI passes parameters by value, so that capture stays the
    /// one-way value capture.
    fn mark_write_captures(&mut self, body: BodyId, function_parameters: &BTreeSet<LocalId>) {
        self.mark_write_captures_inner(body, function_parameters);
    }

    /// The same walk restricted to a function body: `enclosing_params`
    /// holds the parameter locals of every function whose body contains
    /// this one (the item function and each enclosing closure).
    fn mark_write_captures_inner(&mut self, body: BodyId, enclosing_params: &BTreeSet<LocalId>) {
        for &statement in self
            .package
            .statements(self.package.bodies[body].statements)
        {
            self.mark_write_captures_statement(statement, enclosing_params);
        }
    }

    fn mark_write_captures_statement(
        &mut self,
        statement: StmtId,
        enclosing_params: &BTreeSet<LocalId>,
    ) {
        match self.package.statements[statement].kind {
            StatementKind::Let { value, .. } => {
                self.mark_write_captures_expression(value, enclosing_params)
            }
            StatementKind::Assign { target, value } => {
                self.mark_write_captures_expression(target, enclosing_params);
                self.mark_write_captures_expression(value, enclosing_params);
            }
            StatementKind::Expression(expression) => {
                self.mark_write_captures_expression(expression, enclosing_params)
            }
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    self.mark_write_captures_expression(value, enclosing_params);
                }
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.mark_write_captures_expression(condition, enclosing_params);
                self.mark_write_captures_inner(then_body, enclosing_params);
                if let Some(else_body) = else_body {
                    self.mark_write_captures_inner(else_body, enclosing_params);
                }
            }
            StatementKind::While { condition, body } => {
                self.mark_write_captures_expression(condition, enclosing_params);
                self.mark_write_captures_inner(body, enclosing_params);
            }
            StatementKind::For { iterable, body, .. } => {
                self.mark_write_captures_expression(iterable, enclosing_params);
                self.mark_write_captures_inner(body, enclosing_params);
            }
            StatementKind::Match { subject, arms } => {
                self.mark_write_captures_expression(subject, enclosing_params);
                for &arm in self.package.match_arms(arms) {
                    self.mark_write_captures_inner(
                        self.package.match_arms[arm].body,
                        enclosing_params,
                    );
                }
            }
            StatementKind::Break | StatementKind::Continue => {}
        }
    }

    fn mark_write_captures_expression(
        &mut self,
        expression: ExprId,
        enclosing_params: &BTreeSet<LocalId>,
    ) {
        match self.package.expressions[expression].kind {
            ExpressionKind::Closure {
                parameters, body, ..
            } => {
                // This closure's own assignments to locals visible above
                // it become cell captures.
                let mut extended = enclosing_params.clone();
                extended.extend(self.package.locals(parameters).iter().copied());
                let scope = self.package.bodies[body].scope;
                Self::collect_capture_writes(
                    &self.package,
                    body,
                    scope,
                    &extended,
                    &mut self.cell_sources,
                );
                // Nested closures live inside this body.
                self.mark_write_captures_inner(body, &extended);
            }
            ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
            ExpressionKind::Unary { operand, .. } => {
                self.mark_write_captures_expression(operand, enclosing_params)
            }
            ExpressionKind::Binary { left, right, .. } => {
                self.mark_write_captures_expression(left, enclosing_params);
                self.mark_write_captures_expression(right, enclosing_params);
            }
            ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                for &element in self.package.expressions(elements) {
                    self.mark_write_captures_expression(element, enclosing_params);
                }
            }
            ExpressionKind::Call { callee, arguments } => {
                self.mark_write_captures_expression(callee, enclosing_params);
                for &argument in self.package.expressions(arguments) {
                    self.mark_write_captures_expression(argument, enclosing_params);
                }
            }
            ExpressionKind::GenericCall {
                callee,
                type_arguments: _,
                arguments,
            } => {
                self.mark_write_captures_expression(callee, enclosing_params);
                for &argument in self.package.expressions(arguments) {
                    self.mark_write_captures_expression(argument, enclosing_params);
                }
            }
            ExpressionKind::Field { base, .. } => {
                self.mark_write_captures_expression(base, enclosing_params)
            }
            ExpressionKind::Index { base, index } => {
                self.mark_write_captures_expression(base, enclosing_params);
                self.mark_write_captures_expression(index, enclosing_params);
            }
            ExpressionKind::Try(inner)
            | ExpressionKind::Await(inner)
            | ExpressionKind::Spawn(inner) => {
                self.mark_write_captures_expression(inner, enclosing_params)
            }
        }
    }

    /// The closure's direct assignments (through its own control flow,
    /// excluding nested closures — each of those is marked by its own
    /// visit) that target a local outside the closure scope.
    fn collect_capture_writes(
        package: &HirPackage,
        body: BodyId,
        scope: ScopeId,
        enclosing_params: &BTreeSet<LocalId>,
        cell_sources: &mut BTreeSet<LocalId>,
    ) {
        Self::collect_writes_statements(package, body, scope, enclosing_params, cell_sources);
    }

    fn collect_writes_statements(
        package: &HirPackage,
        body: BodyId,
        scope: ScopeId,
        enclosing_params: &BTreeSet<LocalId>,
        cell_sources: &mut BTreeSet<LocalId>,
    ) {
        for &statement in package.statements(package.bodies[body].statements) {
            match package.statements[statement].kind {
                StatementKind::Assign { target, .. } => {
                    if let Some(local) = Self::assign_target_root_local(package, target)
                        && !Self::scope_is_inside_local(package, local, scope)
                        && !enclosing_params.contains(&local)
                    {
                        cell_sources.insert(local);
                    }
                }
                StatementKind::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    Self::collect_writes_statements(
                        package,
                        then_body,
                        scope,
                        enclosing_params,
                        cell_sources,
                    );
                    if let Some(else_body) = else_body {
                        Self::collect_writes_statements(
                            package,
                            else_body,
                            scope,
                            enclosing_params,
                            cell_sources,
                        );
                    }
                }
                StatementKind::While { body, .. } | StatementKind::For { body, .. } => {
                    Self::collect_writes_statements(
                        package,
                        body,
                        scope,
                        enclosing_params,
                        cell_sources,
                    );
                }
                StatementKind::Match { arms, .. } => {
                    for &arm in package.match_arms(arms) {
                        Self::collect_writes_statements(
                            package,
                            package.match_arms[arm].body,
                            scope,
                            enclosing_params,
                            cell_sources,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// The local an assignment target roots at, walking field and index
    /// projections to the base name; `None` for non-place targets.
    fn assign_target_root_local(package: &HirPackage, expression: ExprId) -> Option<LocalId> {
        match package.expressions[expression].kind {
            ExpressionKind::Name {
                binding: NameBinding::Local(local),
                ..
            } => Some(local),
            ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => {
                Self::assign_target_root_local(package, base)
            }
            _ => None,
        }
    }

    fn scope_is_inside_local(package: &HirPackage, local: LocalId, ancestor: ScopeId) -> bool {
        Self::scope_is_inside(package, package.locals[local].scope, ancestor)
    }

    fn scope_is_inside(package: &HirPackage, scope: ScopeId, ancestor: ScopeId) -> bool {
        let mut current = Some(scope);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = package.scopes[candidate].parent;
        }
        false
    }

    fn reserve_function(
        &mut self,
        current: usize,
        origin: OriginId,
    ) -> Result<MirFunctionId, MirBuildError> {
        self.check_next(
            MirCapacity::Functions,
            current,
            self.options.max_functions,
            origin,
        )?;
        let id = MirFunctionId::from_index(current).ok_or_else(|| {
            self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Functions))
        })?;
        // The ID is owned from reservation time: bodies may finish in any
        // order (closures finish before their enclosing function), so the
        // count must not wait for the allocation.
        self.counts.functions = current + 1;
        Ok(id)
    }

    fn type_of_item(&self, item: HirItemId, origin: OriginId) -> Result<TypeId, MirBuildError> {
        self.types.assignments.item(item).ok_or_else(|| {
            self.error(
                origin,
                MirBuildErrorKind::MissingType(TypedEntity::Function(item)),
            )
        })
    }

    fn materialize_type(
        &mut self,
        substitution: &TypeSubstitution,
        ty: TypeId,
        origin: OriginId,
    ) -> Result<TypeId, MirBuildError> {
        // Resolve inference variables bound during shadow inference. The
        // shadow stage normalizes assignments, but a type can still carry a
        // variable at the point it is read (opaque slots are bound by the
        // first use, which may occur after the let). MIR stores concrete
        // types only: a variable that remains unbound is a checker bug and
        // fails the build with a materialization error.
        let ty = self
            .types
            .inference
            .normalize(&mut self.types.interner, ty, &mut self.type_budget)
            .map_err(|error| {
                self.error(
                    origin,
                    match error {
                        TypeError::WorkLimitExceeded { .. } => {
                            MirBuildErrorKind::Capacity(MirCapacity::TypeWork)
                        }
                        TypeError::DepthLimitExceeded { .. } => {
                            MirBuildErrorKind::Capacity(MirCapacity::TypeDepth)
                        }
                        _ => MirBuildErrorKind::GenericTypeMaterialization,
                    },
                )
            })?;
        if matches!(self.types.interner.kind(ty), TypeKind::InferenceVariable(_)) {
            return Err(self.error(origin, MirBuildErrorKind::GenericTypeMaterialization));
        }
        if substitution.is_empty() {
            return Ok(ty);
        }
        substitution
            .apply(&mut self.types.interner, ty, &mut self.type_budget)
            .map_err(|error| {
                self.error(
                    origin,
                    match error {
                        TypeError::WorkLimitExceeded { .. } => {
                            MirBuildErrorKind::Capacity(MirCapacity::TypeWork)
                        }
                        TypeError::DepthLimitExceeded { .. } => {
                            MirBuildErrorKind::Capacity(MirCapacity::TypeDepth)
                        }
                        _ => MirBuildErrorKind::GenericTypeMaterialization,
                    },
                )
            })
    }

    /// The by-reference capture cell type: the single-element list that
    /// holds the captured value.
    fn cell_type(&mut self, element: TypeId, origin: OriginId) -> Result<TypeId, MirBuildError> {
        self.types
            .interner
            .intern(TypeKind::List(element))
            .map_err(|_| self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage)))
    }

    /// The raw source text covered by `span`, when the span is well-formed.
    fn source_text(&self, span: Span) -> Option<&str> {
        let file = self.sources.get(span.file)?;
        let text = file.text();
        let Some(start) = usize::try_from(span.start).ok() else {
            return None;
        };
        let Some(end) = usize::try_from(span.end).ok() else {
            return None;
        };
        if start >= end || end > text.len() {
            return None;
        }
        Some(&text[start..end])
    }

    fn intern_string(
        &mut self,
        text: String,
        origin: OriginId,
    ) -> Result<MirStringId, MirBuildError> {
        if let Some(&id) = self.string_cache.get(&text) {
            return Ok(id);
        }
        self.check_next(
            MirCapacity::Strings,
            self.counts.strings,
            self.options.max_strings,
            origin,
        )?;
        let id =
            self.program.strings.alloc(text.clone()).map_err(|_| {
                self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
            })?;
        self.counts.strings += 1;
        self.string_cache.insert(text, id);
        Ok(id)
    }

    fn materialize_string(
        &mut self,
        span: Span,
        formatted: bool,
        origin: OriginId,
    ) -> Result<MirStringId, MirBuildError> {
        if formatted {
            // A formatted-string *segment*: the HIR desugaring already stripped
            // the `f"..."` wrapper and split out interpolations, so `span`
            // covers raw inner text with no surrounding quotes. An empty span is
            // the empty string (the synthetic leading segment of the desugaring).
            // Unescape the raw text and intern it as an ordinary string constant.
            let text = if span.start >= span.end {
                ""
            } else {
                self.source_text(span)
                    .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?
            };
            let decoded = unescape_literal(text)
                .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
            return self.intern_string(decoded, origin);
        }
        let text = self
            .source_text(span)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let triple = text.starts_with("\"\"\"");
        let inner = if triple {
            text.strip_prefix("\"\"\"")
                .and_then(|inner| inner.strip_suffix("\"\"\""))
        } else {
            text.strip_prefix('\"')
                .and_then(|inner| inner.strip_suffix('\"'))
        };
        let Some(inner) = inner else {
            return Err(self.error(origin, MirBuildErrorKind::InvalidLiteral));
        };
        let decoded = unescape_literal(inner)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        self.intern_string(decoded, origin)
    }

    fn materialize_character(&self, span: Span, origin: OriginId) -> Result<char, MirBuildError> {
        let text = self
            .source_text(span)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let inner = text
            .strip_prefix('\'')
            .and_then(|inner| inner.strip_suffix('\''))
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let decoded = unescape_literal(inner)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let mut characters = decoded.chars();
        let Some(character) = characters.next() else {
            return Err(self.error(origin, MirBuildErrorKind::InvalidLiteral));
        };
        if characters.next().is_some() {
            return Err(self.error(origin, MirBuildErrorKind::InvalidLiteral));
        }
        Ok(character)
    }

    fn check_next(
        &self,
        capacity: MirCapacity,
        current: usize,
        limit: usize,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        if current >= limit {
            Err(self.error(origin, MirBuildErrorKind::Capacity(capacity)))
        } else {
            Ok(())
        }
    }

    const fn error(&self, origin: OriginId, kind: MirBuildErrorKind) -> MirBuildError {
        MirBuildError { origin, kind }
    }

    fn storage_error(&self, origin: OriginId, _: StorageExhausted) -> MirBuildError {
        self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
    }
}

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
