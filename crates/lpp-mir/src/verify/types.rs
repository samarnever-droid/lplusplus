use super::*;

impl Verifier<'_> {
    pub(super) fn rvalue_type(
        &mut self,
        site: InstructionSite,
        value: Rvalue,
        locals: &BTreeSet<MirLocalId>,
    ) -> Option<TypeId> {
        let InstructionSite {
            function,
            block,
            instruction,
            origin,
            target_type,
        } = site;
        match value {
            Rvalue::Use(operand) => {
                let actual =
                    self.operand_type(function, block, Some(instruction), origin, operand, locals)?;
                // Source integer literals are context-sensitive. The builder
                // materializes a non-`Int` literal through `Use` so that the
                // resulting local carries its inferred fixed-width type (or a
                // nominal type for the legacy zero/null sentinel). Keep this
                // exception narrow: it applies only to literal constants and
                // still range-checks every fixed-width representation.
                if let Operand::Constant(crate::Constant::Integer(value)) = operand
                    && self.integer_literal_assignable(value, target_type)
                {
                    Some(target_type)
                } else {
                    Some(actual)
                }
            }
            Rvalue::Unary { operator, operand } => {
                let operand =
                    self.operand_type(function, block, Some(instruction), origin, operand, locals)?;
                let bool_ = self.types.primitive(PrimitiveType::Bool);
                match operator {
                    UnaryOperator::Not if operand == bool_ => Some(bool_),
                    UnaryOperator::Negate if self.is_numeric(operand) => Some(operand),
                    _ => {
                        self.push(
                            function,
                            Some(block),
                            Some(instruction),
                            origin,
                            MirVerificationErrorKind::InvalidUnaryOperand(operator),
                        );
                        None
                    }
                }
            }
            Rvalue::Binary {
                left,
                operator,
                right,
            } => {
                let left_operand = left;
                let right_operand = right;
                let left = self.operand_type(
                    function,
                    block,
                    Some(instruction),
                    origin,
                    left_operand,
                    locals,
                )?;
                let right = self.operand_type(
                    function,
                    block,
                    Some(instruction),
                    origin,
                    right_operand,
                    locals,
                )?;
                let bool_ = self.types.primitive(PrimitiveType::Bool);
                let literal_pair_matches = (self.is_integral(left)
                    && self.integer_operand_assignable(right_operand, left))
                    || (self.is_integral(right)
                        && self.integer_operand_assignable(left_operand, right));
                let valid = match operator {
                    BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr => {
                        (left == bool_ && right == bool_).then_some(bool_)
                    }
                    BinaryOperator::Equal | BinaryOperator::NotEqual => {
                        (left == right || literal_pair_matches).then_some(bool_)
                    }
                    BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEqual
                    | BinaryOperator::GreaterEqual => ((left == right && self.is_ordered(left))
                        || (literal_pair_matches
                            && (self.is_ordered(left) || self.is_ordered(right))))
                    .then_some(bool_),
                    BinaryOperator::Add => {
                        let string = self.types.primitive(PrimitiveType::String);
                        let int = self.types.primitive(PrimitiveType::Int);
                        let float = self.types.primitive(PrimitiveType::Float);
                        if left == right && self.is_addable(left) {
                            Some(left)
                        } else if left == string
                            && (right == int || right == float || right == bool_)
                        {
                            // String concatenation with an implicit stringified
                            // scalar right operand (`"n=" + 3`, f-string interp).
                            Some(string)
                        } else {
                            None
                        }
                    }
                    BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Modulo => {
                        (left == right && self.is_numeric(left)).then_some(left)
                    }
                    BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::BitXor
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight => {
                        (left == right && self.is_integral(left)).then_some(left)
                    }
                };
                if valid.is_none() {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidBinaryOperands(operator),
                    );
                }
                valid
            }
            Rvalue::Tuple(elements) => {
                let TypeKind::Tuple(element_types) = self.types.kind(target_type) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidTupleType(target_type),
                    );
                    return None;
                };
                let expected = self.types.list(element_types);
                let actual = self.program.operands(elements);
                if expected.len() != actual.len() {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::ArityMismatch {
                            expected: expected.len(),
                            actual: actual.len(),
                        },
                    );
                    return None;
                }
                for (expected, operand) in expected.iter().zip(actual) {
                    let actual = self.operand_type(
                        function,
                        block,
                        Some(instruction),
                        origin,
                        *operand,
                        locals,
                    )?;
                    if actual != *expected {
                        self.push(
                            function,
                            Some(block),
                            Some(instruction),
                            origin,
                            MirVerificationErrorKind::TypeMismatch {
                                expected: *expected,
                                actual,
                            },
                        );
                        return None;
                    }
                }
                Some(target_type)
            }
            Rvalue::List(elements) => {
                let TypeKind::List(expected) = self.types.kind(target_type) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidListType(target_type),
                    );
                    return None;
                };
                for operand in self.program.operands(elements) {
                    let actual = self.operand_type(
                        function,
                        block,
                        Some(instruction),
                        origin,
                        *operand,
                        locals,
                    )?;
                    if actual != expected {
                        self.push(
                            function,
                            Some(block),
                            Some(instruction),
                            origin,
                            MirVerificationErrorKind::TypeMismatch { expected, actual },
                        );
                        return None;
                    }
                }
                Some(target_type)
            }
            Rvalue::ConstructStruct { aggregate, fields } => {
                let Some(descriptor) = self.program.aggregate(aggregate) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Aggregate(aggregate)),
                    );
                    return None;
                };
                if descriptor.kind != crate::MirAggregateKind::Struct
                    || descriptor.ty != target_type
                {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidAggregateType(target_type),
                    );
                    return None;
                }
                let expected = self.program.aggregate_fields(descriptor);
                self.verify_aggregate_operands(site, expected, fields, locals)?;
                Some(descriptor.ty)
            }
            Rvalue::ConstructVariant {
                aggregate,
                variant,
                fields,
            } => {
                let Some(descriptor) = self.program.aggregate(aggregate) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Aggregate(aggregate)),
                    );
                    return None;
                };
                let Some(variant_value) = self.program.variant(variant) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Variant(variant)),
                    );
                    return None;
                };
                if descriptor.kind != crate::MirAggregateKind::Enum
                    || descriptor.ty != target_type
                    || variant_value.aggregate != aggregate
                    || !self
                        .program
                        .aggregate_variants(descriptor)
                        .contains(&variant)
                {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidAggregateType(target_type),
                    );
                    return None;
                }
                let expected = self.program.variant_fields(variant_value);
                self.verify_aggregate_operands(site, expected, fields, locals)?;
                Some(descriptor.ty)
            }
            Rvalue::Load(place) => self.place_type(site, place, locals, false),
            Rvalue::ListLen(list) => {
                let list =
                    self.operand_type(function, block, Some(instruction), origin, list, locals)?;
                if !matches!(self.types.kind(list), TypeKind::List(_)) {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidListType(list),
                    );
                    return None;
                }
                Some(self.types.primitive(PrimitiveType::Int))
            }
            Rvalue::Call { callee, arguments } => {
                let callee =
                    self.operand_type(function, block, Some(instruction), origin, callee, locals)?;
                let TypeKind::Function { parameters, result } = self.types.kind(callee) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidFunctionType(callee),
                    );
                    return None;
                };
                let expected = self.types.list(parameters);
                let actual = self.program.operands(arguments);
                if expected.len() != actual.len() {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::ArityMismatch {
                            expected: expected.len(),
                            actual: actual.len(),
                        },
                    );
                    return None;
                }
                for (expected, operand) in expected.iter().zip(actual) {
                    let actual = self.operand_type(
                        function,
                        block,
                        Some(instruction),
                        origin,
                        *operand,
                        locals,
                    )?;
                    if actual != *expected {
                        self.push(
                            function,
                            Some(block),
                            Some(instruction),
                            origin,
                            MirVerificationErrorKind::TypeMismatch {
                                expected: *expected,
                                actual,
                            },
                        );
                        return None;
                    }
                }
                Some(result)
            }
            Rvalue::Builtin { builtin, arguments } => {
                let descriptor = builtin.descriptor();
                let expected = descriptor.semantic_parameters;
                let actual = self.program.operands(arguments);
                if expected.len() != actual.len() {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::ArityMismatch {
                            expected: expected.len(),
                            actual: actual.len(),
                        },
                    );
                    return None;
                }
                for (argument, (semantic, operand)) in expected.iter().zip(actual).enumerate() {
                    let actual = self.operand_type(
                        function,
                        block,
                        Some(instruction),
                        origin,
                        *operand,
                        locals,
                    )?;
                    let contextual_literal = lpp_types::semantic_result_type(*semantic, self.types)
                        .is_some_and(|expected| {
                            self.integer_operand_assignable(*operand, expected)
                        });
                    if !lpp_types::type_matches_semantic(actual, *semantic, self.types)
                        && !contextual_literal
                    {
                        self.push(
                            function,
                            Some(block),
                            Some(instruction),
                            origin,
                            MirVerificationErrorKind::InvalidBuiltinArgument {
                                argument,
                                expected: lpp_types::semantic_spelling(*semantic),
                                actual,
                            },
                        );
                        return None;
                    }
                }
                let expected_result =
                    lpp_types::semantic_result_type(descriptor.semantic_result, self.types)
                        .unwrap_or(target_type);
                if expected_result != target_type {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::TypeMismatch {
                            expected: expected_result,
                            actual: target_type,
                        },
                    );
                    return None;
                }
                Some(target_type)
            }
            Rvalue::MakeClosure {
                function: target,
                captures,
            } => {
                let Some(target_function) = self.program.function(target) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Function(target)),
                    );
                    return None;
                };
                if target_function.kind != crate::MirFunctionKind::Closure {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidClosureFunction(target),
                    );
                    return None;
                }
                let expected = self.program.function_captures(target_function);
                let actual = self.program.operands(captures);
                if expected.len() != actual.len() {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::ArityMismatch {
                            expected: expected.len(),
                            actual: actual.len(),
                        },
                    );
                    return None;
                }
                for (expected_local, operand) in expected.iter().zip(actual) {
                    let Some(capture) = self.program.local(*expected_local) else {
                        self.push(
                            function,
                            Some(block),
                            Some(instruction),
                            origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Local(
                                *expected_local,
                            )),
                        );
                        return None;
                    };
                    let actual = self.operand_type(
                        function,
                        block,
                        Some(instruction),
                        origin,
                        *operand,
                        locals,
                    )?;
                    if actual != capture.ty {
                        self.push(
                            function,
                            Some(block),
                            Some(instruction),
                            origin,
                            MirVerificationErrorKind::TypeMismatch {
                                expected: capture.ty,
                                actual,
                            },
                        );
                        return None;
                    }
                }
                if target_function.ty != target_type {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::TypeMismatch {
                            expected: target_function.ty,
                            actual: target_type,
                        },
                    );
                    return None;
                }
                Some(target_type)
            }
            Rvalue::Await(operand) => {
                let ty =
                    self.operand_type(function, block, Some(instruction), origin, operand, locals)?;
                let TypeKind::Task(inner) = self.types.kind(ty) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidAwaitType(ty),
                    );
                    return None;
                };
                if inner != target_type {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::TypeMismatch {
                            expected: target_type,
                            actual: inner,
                        },
                    );
                    return None;
                }
                Some(target_type)
            }
            Rvalue::Spawn(operand) => {
                let ty =
                    self.operand_type(function, block, Some(instruction), origin, operand, locals)?;
                let TypeKind::Function {
                    parameters: _,
                    result,
                } = self.types.kind(ty)
                else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidSpawnTarget(ty),
                    );
                    return None;
                };
                let void = self.types.primitive(PrimitiveType::Void);
                if result != void || target_type != void {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidSpawnTarget(ty),
                    );
                    return None;
                }
                Some(target_type)
            }
        }
    }

    pub(super) fn place_type(
        &mut self,
        site: InstructionSite,
        place_id: MirPlaceId,
        locals: &BTreeSet<MirLocalId>,
        for_store: bool,
    ) -> Option<TypeId> {
        let Some(place) = self.program.place(place_id).copied() else {
            self.push(
                site.function,
                Some(site.block),
                Some(site.instruction),
                site.origin,
                MirVerificationErrorKind::MissingReference(MirEntity::Place(place_id)),
            );
            return None;
        };
        let Some(root) = self.program.local(place.root).copied() else {
            self.reference_error(
                site.function,
                site.block,
                site.instruction,
                site.origin,
                MirEntity::Local(place.root),
                false,
            );
            return None;
        };
        if !locals.contains(&place.root) {
            self.reference_error(
                site.function,
                site.block,
                site.instruction,
                site.origin,
                MirEntity::Local(place.root),
                true,
            );
            return None;
        }
        let projections = self.program.place_projections(&place).to_vec();
        if for_store {
            if projections
                .iter()
                .any(|projection| matches!(projection, PlaceProjection::Downcast(_)))
            {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::DowncastStore,
                );
                return None;
            }
            let parameter_projection = !projections.is_empty()
                && self
                    .program
                    .function(site.function)
                    .is_some_and(|function| {
                        self.program
                            .function_parameters(function)
                            .contains(&place.root)
                    });
            if !root.mutable && !parameter_projection {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::ImmutableStore(place.root),
                );
                return None;
            }
            if projections
                .iter()
                .any(|projection| matches!(projection, PlaceProjection::TupleField(_)))
            {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::TupleElementStore,
                );
                return None;
            }
        }

        let mut current = root.ty;
        let mut downcast = None;
        for projection in projections {
            current = match projection {
                PlaceProjection::Downcast(variant_id) => {
                    let Some(variant) = self.program.variant(variant_id) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Variant(
                                variant_id,
                            )),
                        );
                        return None;
                    };
                    let Some(aggregate) = self.program.aggregate(variant.aggregate) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Aggregate(
                                variant.aggregate,
                            )),
                        );
                        return None;
                    };
                    if downcast.is_some()
                        || current != aggregate.ty
                        || aggregate.kind != crate::MirAggregateKind::Enum
                        || !self
                            .program
                            .aggregate_variants(aggregate)
                            .contains(&variant_id)
                    {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidAggregateType(current),
                        );
                        return None;
                    }
                    downcast = Some(variant_id);
                    current
                }
                PlaceProjection::Field(field_id) => {
                    let Some(field) = self.program.field(field_id) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Field(field_id)),
                        );
                        return None;
                    };
                    let Some(aggregate) = self.program.aggregate(field.aggregate) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Aggregate(
                                field.aggregate,
                            )),
                        );
                        return None;
                    };
                    let valid = if let Some(variant) = downcast.take() {
                        aggregate.kind == crate::MirAggregateKind::Enum
                            && field.variant == Some(variant)
                            && self.program.variant(variant).is_some_and(|variant| {
                                self.program.variant_fields(variant).contains(&field_id)
                            })
                    } else {
                        aggregate.kind == crate::MirAggregateKind::Struct
                            && field.variant.is_none()
                            && self.program.aggregate_fields(aggregate).contains(&field_id)
                    };
                    if current != aggregate.ty || !valid {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidAggregateType(current),
                        );
                        return None;
                    }
                    field.ty
                }
                PlaceProjection::TupleField(index) => {
                    let TypeKind::Tuple(elements) = self.types.kind(current) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidTupleType(current),
                        );
                        return None;
                    };
                    let Some(element) = self.types.list(elements).get(index as usize).copied()
                    else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidTupleType(current),
                        );
                        return None;
                    };
                    element
                }
                PlaceProjection::ListIndex(index) => {
                    let actual = self.operand_type(
                        site.function,
                        site.block,
                        Some(site.instruction),
                        site.origin,
                        index,
                        locals,
                    )?;
                    let int = self.types.primitive(PrimitiveType::Int);
                    if actual != int {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::TypeMismatch {
                                expected: int,
                                actual,
                            },
                        );
                        return None;
                    }
                    let TypeKind::List(element) = self.types.kind(current) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidListType(current),
                        );
                        return None;
                    };
                    element
                }
            };
        }
        if downcast.is_some() {
            self.push(
                site.function,
                Some(site.block),
                Some(site.instruction),
                site.origin,
                MirVerificationErrorKind::InvalidAggregateType(current),
            );
            return None;
        }
        if current != place.ty {
            self.push(
                site.function,
                Some(site.block),
                Some(site.instruction),
                site.origin,
                MirVerificationErrorKind::TypeMismatch {
                    expected: current,
                    actual: place.ty,
                },
            );
            return None;
        }
        Some(current)
    }

    fn verify_aggregate_operands(
        &mut self,
        site: InstructionSite,
        expected: &[MirFieldId],
        actual: crate::ListRange<Operand>,
        locals: &BTreeSet<MirLocalId>,
    ) -> Option<()> {
        let actual = self.program.operands(actual);
        if expected.len() != actual.len() {
            self.push(
                site.function,
                Some(site.block),
                Some(site.instruction),
                site.origin,
                MirVerificationErrorKind::ArityMismatch {
                    expected: expected.len(),
                    actual: actual.len(),
                },
            );
            return None;
        }
        for (field, operand) in expected.iter().zip(actual) {
            let Some(field) = self.program.field(*field) else {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::MissingReference(MirEntity::Field(*field)),
                );
                return None;
            };
            let actual = self.operand_type(
                site.function,
                site.block,
                Some(site.instruction),
                site.origin,
                *operand,
                locals,
            )?;
            if actual != field.ty && !self.integer_operand_assignable(*operand, field.ty) {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::TypeMismatch {
                        expected: field.ty,
                        actual,
                    },
                );
                return None;
            }
        }
        Some(())
    }

    pub(super) fn operand_type(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        instruction: Option<InstructionId>,
        origin: OriginId,
        operand: Operand,
        locals: &BTreeSet<MirLocalId>,
    ) -> Option<TypeId> {
        match operand {
            Operand::Copy(local) => match self.program.local(local) {
                Some(value) if locals.contains(&local) => Some(value.ty),
                Some(_) => {
                    self.push(
                        function,
                        Some(block),
                        instruction,
                        origin,
                        MirVerificationErrorKind::CrossFunctionReference(MirEntity::Local(local)),
                    );
                    None
                }
                None => {
                    self.push(
                        function,
                        Some(block),
                        instruction,
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Local(local)),
                    );
                    None
                }
            },
            Operand::Function(target) => match self.program.function(target) {
                Some(target) => Some(target.ty),
                None => {
                    self.push(
                        function,
                        Some(block),
                        instruction,
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Function(target)),
                    );
                    None
                }
            },
            Operand::Constant(constant) => Some(match constant {
                crate::Constant::Integer(_) => self.types.primitive(PrimitiveType::Int),
                crate::Constant::FloatBits(_) => self.types.primitive(PrimitiveType::Float),
                crate::Constant::String { .. } => self.types.primitive(PrimitiveType::String),
                crate::Constant::Character { .. } => self.types.primitive(PrimitiveType::Char),
                crate::Constant::Bool(_) => self.types.primitive(PrimitiveType::Bool),
            }),
        }
    }

    pub(super) fn reference_error(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        instruction: InstructionId,
        origin: OriginId,
        entity: MirEntity,
        exists: bool,
    ) {
        self.push(
            function,
            Some(block),
            Some(instruction),
            origin,
            if exists {
                MirVerificationErrorKind::CrossFunctionReference(entity)
            } else {
                MirVerificationErrorKind::MissingReference(entity)
            },
        );
    }

    pub(super) fn integer_operand_assignable(&self, operand: Operand, target: TypeId) -> bool {
        match operand {
            Operand::Constant(crate::Constant::Integer(value)) => {
                self.integer_literal_assignable(value, target)
            }
            Operand::Copy(_) | Operand::Function(_) | Operand::Constant(_) => false,
        }
    }

    fn integer_literal_assignable(&self, value: i64, target: TypeId) -> bool {
        match self.types.kind(target) {
            TypeKind::Primitive(PrimitiveType::Int) => true,
            TypeKind::Primitive(PrimitiveType::U8) => u8::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::U16) => u16::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::U32) => u32::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::I8) => i8::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::I16) => i16::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::I32) => i32::try_from(value).is_ok(),
            // Legacy L++ source uses integer zero for a null aggregate edge,
            // most notably as the terminal link of recursive structures.
            TypeKind::Nominal { .. } => value == 0,
            _ => false,
        }
    }

    fn is_numeric(&self, ty: TypeId) -> bool {
        self.is_integral(ty) || self.types.kind(ty) == TypeKind::Primitive(PrimitiveType::Float)
    }

    pub(super) fn is_integral(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::Primitive(
                PrimitiveType::Int
                    | PrimitiveType::U8
                    | PrimitiveType::U16
                    | PrimitiveType::U32
                    | PrimitiveType::I8
                    | PrimitiveType::I16
                    | PrimitiveType::I32
            )
        )
    }

    fn is_addable(&self, ty: TypeId) -> bool {
        self.is_numeric(ty) || self.types.kind(ty) == TypeKind::Primitive(PrimitiveType::String)
    }

    fn is_ordered(&self, ty: TypeId) -> bool {
        self.is_addable(ty) || self.types.kind(ty) == TypeKind::Primitive(PrimitiveType::Char)
    }
}
