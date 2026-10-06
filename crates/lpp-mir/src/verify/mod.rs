use std::collections::BTreeSet;

use lpp_hir::{BinaryOperator, InstanceId, OriginId, UnaryOperator};
use lpp_types::{PrimitiveType, TypeId, TypeInterner, TypeKind};

use crate::{
    BasicBlockId, InstructionId, InstructionKind, MirAggregateId, MirFieldId, MirFunctionId,
    MirLocalId, MirPlaceId, MirProgram, MirVariantId, Operand, PlaceProjection, Rvalue, Terminator,
};

mod definite;
mod types;

pub use definite::DefiniteInitializationLimits;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MirInvariant {
    References,
    ControlFlow,
    Types,
    DefiniteInitialization,
    Ownership,
    NoOwningCycles,
    OwnershipBalance,
    BackendLegal,
}

pub const CORE_MIR_INVARIANTS: &[MirInvariant] = &[
    MirInvariant::References,
    MirInvariant::ControlFlow,
    MirInvariant::Types,
    MirInvariant::DefiniteInitialization,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirEntity {
    Function(MirFunctionId),
    Aggregate(MirAggregateId),
    Field(MirFieldId),
    Variant(MirVariantId),
    Block(BasicBlockId),
    Local(MirLocalId),
    Place(MirPlaceId),
    Instruction(InstructionId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefiniteInitializationResource {
    StateWords,
    Iterations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirVerificationErrorKind {
    MissingReference(MirEntity),
    CrossFunctionReference(MirEntity),
    DuplicateInstance(InstanceId),
    InvalidInstanceArguments,
    EntryOutsideFunction(BasicBlockId),
    InvalidFunctionType(TypeId),
    InvalidAggregateType(TypeId),
    InvalidAggregateDescriptor,
    InvalidTupleType(TypeId),
    InvalidListType(TypeId),
    TypeMismatch {
        expected: TypeId,
        actual: TypeId,
    },
    ArityMismatch {
        expected: usize,
        actual: usize,
    },
    InvalidUnaryOperand(UnaryOperator),
    InvalidBinaryOperands(BinaryOperator),
    InvalidBuiltinArgument {
        argument: usize,
        expected: &'static str,
        actual: TypeId,
    },
    InvalidClosureFunction(MirFunctionId),
    InvalidAwaitType(TypeId),
    InvalidSpawnTarget(TypeId),
    ImmutableStore(MirLocalId),
    TupleElementStore,
    DowncastStore,
    UninitializedRead(MirLocalId),
    DefiniteInitializationLimit {
        resource: DefiniteInitializationResource,
        required: usize,
        limit: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirVerificationError {
    pub function: Option<MirFunctionId>,
    pub block: Option<BasicBlockId>,
    pub instruction: Option<InstructionId>,
    pub origin: OriginId,
    pub kind: MirVerificationErrorKind,
}

impl MirVerificationError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            MirVerificationErrorKind::MissingReference(_)
            | MirVerificationErrorKind::CrossFunctionReference(_)
            | MirVerificationErrorKind::DuplicateInstance(_)
            | MirVerificationErrorKind::InvalidInstanceArguments => "E4101",
            MirVerificationErrorKind::EntryOutsideFunction(_) => "E4102",
            MirVerificationErrorKind::InvalidFunctionType(_)
            | MirVerificationErrorKind::InvalidAggregateType(_)
            | MirVerificationErrorKind::InvalidAggregateDescriptor
            | MirVerificationErrorKind::InvalidTupleType(_)
            | MirVerificationErrorKind::InvalidListType(_)
            | MirVerificationErrorKind::TypeMismatch { .. }
            | MirVerificationErrorKind::ArityMismatch { .. }
            | MirVerificationErrorKind::InvalidUnaryOperand(_)
            | MirVerificationErrorKind::InvalidBinaryOperands(_)
            | MirVerificationErrorKind::InvalidBuiltinArgument { .. }
            | MirVerificationErrorKind::InvalidClosureFunction(_)
            | MirVerificationErrorKind::InvalidAwaitType(_)
            | MirVerificationErrorKind::InvalidSpawnTarget(_)
            | MirVerificationErrorKind::ImmutableStore(_)
            | MirVerificationErrorKind::TupleElementStore
            | MirVerificationErrorKind::DowncastStore => "E4103",
            MirVerificationErrorKind::UninitializedRead(_) => "E4104",
            MirVerificationErrorKind::DefiniteInitializationLimit { .. } => "E4105",
        }
    }
}

#[must_use]
pub fn verify_mir(program: &MirProgram, types: &TypeInterner) -> Vec<MirVerificationError> {
    verify_mir_with_limits(program, types, DefiniteInitializationLimits::default())
}

#[must_use]
pub fn verify_mir_with_limits(
    program: &MirProgram,
    types: &TypeInterner,
    definite_limits: DefiniteInitializationLimits,
) -> Vec<MirVerificationError> {
    let mut verifier = Verifier {
        program,
        types,
        errors: Vec::new(),
    };
    verifier.verify_aggregates();
    let mut instances = BTreeSet::new();
    for (function_id, function) in program.functions() {
        let has_instance_arguments = !types.list(function.instance_arguments).is_empty();
        if matches!(
            (function.instance, has_instance_arguments),
            (None, true) | (Some(_), false)
        ) {
            verifier.push(
                function_id,
                None,
                None,
                function.origin,
                MirVerificationErrorKind::InvalidInstanceArguments,
            );
        }
        if let Some(instance) = function.instance
            && !instances.insert(instance)
        {
            verifier.push(
                function_id,
                None,
                None,
                function.origin,
                MirVerificationErrorKind::DuplicateInstance(instance),
            );
        }
        verifier.verify_function(function_id, function);
    }
    if verifier.errors.is_empty() {
        verifier
            .errors
            .extend(definite::verify_definite_initialization(
                program,
                definite_limits,
            ));
    }
    verifier.errors
}

struct Verifier<'input> {
    program: &'input MirProgram,
    types: &'input TypeInterner,
    errors: Vec<MirVerificationError>,
}

#[derive(Clone, Copy)]
struct InstructionSite {
    function: MirFunctionId,
    block: BasicBlockId,
    instruction: InstructionId,
    origin: OriginId,
    target_type: TypeId,
}

impl Verifier<'_> {
    fn verify_aggregates(&mut self) {
        let mut types = BTreeSet::new();
        let mut instances = BTreeSet::new();
        let mut listed_fields = BTreeSet::new();
        let mut listed_variants = BTreeSet::new();
        for (aggregate_id, aggregate) in self.program.aggregates() {
            let valid_type = matches!(
                self.types.kind(aggregate.ty),
                TypeKind::Nominal {
                    definition,
                    arguments,
                } if definition == aggregate.definition && arguments == aggregate.arguments
            );
            if !valid_type || !self.is_concrete_type(aggregate.ty) || !types.insert(aggregate.ty) {
                self.push_global(
                    aggregate.origin,
                    MirVerificationErrorKind::InvalidAggregateDescriptor,
                );
            }
            if let Some(instance) = aggregate.instance
                && (!instances.insert(instance) || self.types.list(aggregate.arguments).is_empty())
            {
                self.push_global(
                    aggregate.origin,
                    MirVerificationErrorKind::InvalidAggregateDescriptor,
                );
            }
            let fields = self.program.aggregate_fields(aggregate);
            let variants = self.program.aggregate_variants(aggregate);
            match aggregate.kind {
                crate::MirAggregateKind::Struct if !variants.is_empty() => self.push_global(
                    aggregate.origin,
                    MirVerificationErrorKind::InvalidAggregateDescriptor,
                ),
                crate::MirAggregateKind::Enum if !fields.is_empty() => self.push_global(
                    aggregate.origin,
                    MirVerificationErrorKind::InvalidAggregateDescriptor,
                ),
                crate::MirAggregateKind::Struct | crate::MirAggregateKind::Enum => {}
            }
            for field_id in fields {
                if !listed_fields.insert(*field_id) {
                    self.push_global(
                        aggregate.origin,
                        MirVerificationErrorKind::InvalidAggregateDescriptor,
                    );
                }
                match self.program.field(*field_id) {
                    Some(field)
                        if field.aggregate == aggregate_id
                            && field.variant.is_none()
                            && self.is_concrete_type(field.ty) => {}
                    Some(field) => self.push_global(
                        field.origin,
                        MirVerificationErrorKind::InvalidAggregateDescriptor,
                    ),
                    None => self.push_global(
                        aggregate.origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Field(*field_id)),
                    ),
                }
            }
            for (ordinal, variant_id) in variants.iter().enumerate() {
                if !listed_variants.insert(*variant_id) {
                    self.push_global(
                        aggregate.origin,
                        MirVerificationErrorKind::InvalidAggregateDescriptor,
                    );
                }
                let Some(variant) = self.program.variant(*variant_id) else {
                    self.push_global(
                        aggregate.origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Variant(*variant_id)),
                    );
                    continue;
                };
                if variant.aggregate != aggregate_id || variant.ordinal as usize != ordinal {
                    self.push_global(
                        variant.origin,
                        MirVerificationErrorKind::InvalidAggregateDescriptor,
                    );
                }
                for field_id in self.program.variant_fields(variant) {
                    if !listed_fields.insert(*field_id) {
                        self.push_global(
                            variant.origin,
                            MirVerificationErrorKind::InvalidAggregateDescriptor,
                        );
                    }
                    match self.program.field(*field_id) {
                        Some(field)
                            if field.aggregate == aggregate_id
                                && field.variant == Some(*variant_id)
                                && self.is_concrete_type(field.ty) => {}
                        Some(field) => self.push_global(
                            field.origin,
                            MirVerificationErrorKind::InvalidAggregateDescriptor,
                        ),
                        None => self.push_global(
                            variant.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Field(*field_id)),
                        ),
                    }
                }
            }
        }
        for (field_id, field) in self.program.fields() {
            if !listed_fields.contains(&field_id) {
                self.push_global(
                    field.origin,
                    MirVerificationErrorKind::InvalidAggregateDescriptor,
                );
            }
        }
        for (variant_id, variant) in self.program.variants() {
            if !listed_variants.contains(&variant_id) {
                self.push_global(
                    variant.origin,
                    MirVerificationErrorKind::InvalidAggregateDescriptor,
                );
            }
        }
    }

    fn is_concrete_type(&self, root: TypeId) -> bool {
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            if !visited.insert(ty) {
                continue;
            }
            match self.types.kind(ty) {
                TypeKind::Tuple(elements) => {
                    pending.extend(self.types.list(elements).iter().copied());
                }
                TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
                    pending.push(element);
                }
                TypeKind::Map { key, value } => {
                    pending.push(key);
                    pending.push(value);
                }
                TypeKind::Function { parameters, result } => {
                    pending.extend(self.types.list(parameters).iter().copied());
                    pending.push(result);
                }
                TypeKind::Nominal { arguments, .. } => {
                    pending.extend(self.types.list(arguments).iter().copied());
                }
                TypeKind::Error
                | TypeKind::GenericParameter(_)
                | TypeKind::BoundVariable(_)
                | TypeKind::InferenceVariable(_)
                | TypeKind::UnresolvedName { .. } => return false,
                TypeKind::Never | TypeKind::Primitive(_) => {}
            }
        }
        true
    }

    fn verify_function(&mut self, function_id: MirFunctionId, function: &crate::MirFunction) {
        let local_ids = self
            .program
            .function_locals(function)
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let block_ids = self
            .program
            .function_blocks(function)
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let TypeKind::Function { parameters, result } = self.types.kind(function.ty) else {
            self.push(
                function_id,
                None,
                None,
                function.origin,
                MirVerificationErrorKind::InvalidFunctionType(function.ty),
            );
            return;
        };
        // Async functions expose `fn(...) -> Task<R>` while the body and the
        // MIR return type are `R`.
        let expected_result = match (function.kind, self.types.kind(result)) {
            (crate::MirFunctionKind::Async, TypeKind::Task(inner)) => inner,
            _ => result,
        };
        if expected_result != function.return_type {
            self.push(
                function_id,
                None,
                None,
                function.origin,
                MirVerificationErrorKind::TypeMismatch {
                    expected: expected_result,
                    actual: function.return_type,
                },
            );
        }
        let parameter_types = self.types.list(parameters);
        let mir_parameters = self.program.function_parameters(function);
        // Closure frames begin with one slot per captured enclosing local,
        // followed by the user parameters.
        let capture_count = self.program.function_captures(function).len();
        let user_parameters = mir_parameters.len().saturating_sub(capture_count);
        if capture_count > mir_parameters.len() || parameter_types.len() != user_parameters {
            self.push(
                function_id,
                None,
                None,
                function.origin,
                MirVerificationErrorKind::ArityMismatch {
                    expected: parameter_types.len(),
                    actual: user_parameters,
                },
            );
        }
        for (expected, parameter) in parameter_types
            .iter()
            .zip(mir_parameters.iter().skip(capture_count))
        {
            match self.program.local(*parameter) {
                Some(local) if local_ids.contains(parameter) => {
                    if local.ty != *expected {
                        self.push(
                            function_id,
                            None,
                            None,
                            local.origin,
                            MirVerificationErrorKind::TypeMismatch {
                                expected: *expected,
                                actual: local.ty,
                            },
                        );
                    }
                }
                Some(local) => self.push(
                    function_id,
                    None,
                    None,
                    local.origin,
                    MirVerificationErrorKind::CrossFunctionReference(MirEntity::Local(*parameter)),
                ),
                None => self.push(
                    function_id,
                    None,
                    None,
                    function.origin,
                    MirVerificationErrorKind::MissingReference(MirEntity::Local(*parameter)),
                ),
            }
        }

        if !block_ids.contains(&function.entry) {
            self.push(
                function_id,
                None,
                None,
                function.origin,
                MirVerificationErrorKind::EntryOutsideFunction(function.entry),
            );
        }
        for block_id in self.program.function_blocks(function) {
            let Some(block) = self.program.block(*block_id) else {
                self.push(
                    function_id,
                    Some(*block_id),
                    None,
                    function.origin,
                    MirVerificationErrorKind::MissingReference(MirEntity::Block(*block_id)),
                );
                continue;
            };
            for instruction_id in self.program.block_instructions(block) {
                let Some(instruction) = self.program.instruction(*instruction_id) else {
                    self.push(
                        function_id,
                        Some(*block_id),
                        Some(*instruction_id),
                        block.origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Instruction(
                            *instruction_id,
                        )),
                    );
                    continue;
                };
                match instruction.kind {
                    InstructionKind::Assign { target, value } => {
                        let Some(target_value) = self.program.local(target) else {
                            self.reference_error(
                                function_id,
                                *block_id,
                                *instruction_id,
                                instruction.origin,
                                MirEntity::Local(target),
                                false,
                            );
                            continue;
                        };
                        if !local_ids.contains(&target) {
                            self.reference_error(
                                function_id,
                                *block_id,
                                *instruction_id,
                                instruction.origin,
                                MirEntity::Local(target),
                                true,
                            );
                            continue;
                        }
                        let expected = target_value.ty;
                        let actual = self.rvalue_type(
                            InstructionSite {
                                function: function_id,
                                block: *block_id,
                                instruction: *instruction_id,
                                origin: instruction.origin,
                                target_type: expected,
                            },
                            value,
                            &local_ids,
                        );
                        if let Some(actual) = actual
                            && expected != actual
                        {
                            self.push(
                                function_id,
                                Some(*block_id),
                                Some(*instruction_id),
                                instruction.origin,
                                MirVerificationErrorKind::TypeMismatch { expected, actual },
                            );
                        }
                    }
                    InstructionKind::Store { place, value } => {
                        let site = InstructionSite {
                            function: function_id,
                            block: *block_id,
                            instruction: *instruction_id,
                            origin: instruction.origin,
                            target_type: self
                                .program
                                .place(place)
                                .map_or(function.return_type, |place| place.ty),
                        };
                        let place_type = self.place_type(site, place, &local_ids, true);
                        let value_type = self.operand_type(
                            function_id,
                            *block_id,
                            Some(*instruction_id),
                            instruction.origin,
                            value,
                            &local_ids,
                        );
                        if let (Some(expected), Some(actual)) = (place_type, value_type)
                            && expected != actual
                            && !(self.is_integral(expected)
                                && self.integer_operand_assignable(value, expected))
                        {
                            self.push(
                                function_id,
                                Some(*block_id),
                                Some(*instruction_id),
                                instruction.origin,
                                MirVerificationErrorKind::TypeMismatch { expected, actual },
                            );
                        }
                    }
                }
            }
            self.verify_terminator(
                function_id,
                function,
                *block_id,
                block,
                &local_ids,
                &block_ids,
            );
        }
    }

    fn verify_terminator(
        &mut self,
        function_id: MirFunctionId,
        function: &crate::MirFunction,
        block_id: BasicBlockId,
        block: &crate::BasicBlock,
        locals: &BTreeSet<MirLocalId>,
        blocks: &BTreeSet<BasicBlockId>,
    ) {
        match block.terminator {
            Terminator::Goto(target) => {
                self.verify_edge(function_id, block_id, block.origin, target, blocks);
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                self.verify_edge(function_id, block_id, block.origin, then_block, blocks);
                self.verify_edge(function_id, block_id, block.origin, else_block, blocks);
                if let Some(actual) =
                    self.operand_type(function_id, block_id, None, block.origin, condition, locals)
                {
                    let expected = self.types.primitive(PrimitiveType::Bool);
                    // Source conditions support integer truthiness. Both native
                    // and Wasm lowering normalize an integral condition to
                    // `value != 0`, so MIR must preserve the same language rule
                    // rather than rejecting a type-checked branch here.
                    if actual != expected && !self.is_integral(actual) {
                        self.push(
                            function_id,
                            Some(block_id),
                            None,
                            block.origin,
                            MirVerificationErrorKind::TypeMismatch { expected, actual },
                        );
                    }
                }
            }
            Terminator::SwitchEnum {
                subject,
                aggregate,
                targets,
            } => {
                let Some(descriptor) = self.program.aggregate(aggregate) else {
                    self.push(
                        function_id,
                        Some(block_id),
                        None,
                        block.origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Aggregate(aggregate)),
                    );
                    return;
                };
                let target_ids = self.program.switch_targets(targets);
                let expected_targets = self.program.aggregate_variants(descriptor).len();
                if descriptor.kind != crate::MirAggregateKind::Enum
                    || target_ids.len() != expected_targets
                {
                    self.push(
                        function_id,
                        Some(block_id),
                        None,
                        block.origin,
                        MirVerificationErrorKind::InvalidAggregateDescriptor,
                    );
                }
                if let Some(actual) =
                    self.operand_type(function_id, block_id, None, block.origin, subject, locals)
                    && actual != descriptor.ty
                {
                    self.push(
                        function_id,
                        Some(block_id),
                        None,
                        block.origin,
                        MirVerificationErrorKind::TypeMismatch {
                            expected: descriptor.ty,
                            actual,
                        },
                    );
                }
                for target in target_ids {
                    self.verify_edge(function_id, block_id, block.origin, *target, blocks);
                }
            }
            Terminator::Return(value) => {
                let actual = value.map_or(self.types.primitive(PrimitiveType::Void), |value| {
                    self.operand_type(function_id, block_id, None, block.origin, value, locals)
                        .unwrap_or(function.return_type)
                });
                if actual != function.return_type {
                    self.push(
                        function_id,
                        Some(block_id),
                        None,
                        block.origin,
                        MirVerificationErrorKind::TypeMismatch {
                            expected: function.return_type,
                            actual,
                        },
                    );
                }
            }
            Terminator::Unreachable => {}
        }
    }

    fn verify_edge(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        target: BasicBlockId,
        blocks: &BTreeSet<BasicBlockId>,
    ) {
        if blocks.contains(&target) {
            return;
        }
        self.push(
            function,
            Some(block),
            None,
            origin,
            if self.program.block(target).is_some() {
                MirVerificationErrorKind::CrossFunctionReference(MirEntity::Block(target))
            } else {
                MirVerificationErrorKind::MissingReference(MirEntity::Block(target))
            },
        );
    }

    fn push_global(&mut self, origin: OriginId, kind: MirVerificationErrorKind) {
        self.errors.push(MirVerificationError {
            function: None,
            block: None,
            instruction: None,
            origin,
            kind,
        });
    }

    fn push(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        instruction: Option<InstructionId>,
        origin: OriginId,
        kind: MirVerificationErrorKind,
    ) {
        self.errors.push(MirVerificationError {
            function: Some(function),
            block,
            instruction,
            origin,
            kind,
        });
    }
}
