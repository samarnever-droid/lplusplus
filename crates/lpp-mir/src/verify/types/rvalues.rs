use super::*;

impl Verifier<'_> {
    pub(in crate::verify) fn rvalue_type(
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
                let int = self.types.primitive(PrimitiveType::Int);
                let literal_pair_matches = (self.is_integral(left)
                    && self.integer_operand_assignable(right_operand, left))
                    || (self.is_integral(right)
                        && self.integer_operand_assignable(left_operand, right));
                let bool_int_pair =
                    (left == bool_ && right == int) || (left == int && right == bool_);
                let valid = match operator {
                    BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr => {
                        (left == bool_ && right == bool_).then_some(bool_)
                    }
                    BinaryOperator::Equal | BinaryOperator::NotEqual => {
                        (left == right || literal_pair_matches || bool_int_pair).then_some(bool_)
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
            Rvalue::ExternalCall {
                function: external_id,
                arguments,
            } => {
                let Some(external) = self.program.external_function(external_id) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::ExternalFunction(
                            external_id,
                        )),
                    );
                    return None;
                };
                let external_ty = external.ty;
                let TypeKind::Function { parameters, result } = self.types.kind(external_ty) else {
                    self.push(
                        function,
                        Some(block),
                        Some(instruction),
                        origin,
                        MirVerificationErrorKind::InvalidFunctionType(external_ty),
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
            Rvalue::Cast { value, target } => {
                self.operand_type(function, block, Some(instruction), origin, value, locals)?;
                Some(target)
            }
            Rvalue::Intrinsic {
                intrinsic: _,
                type_argument: _,
                arguments,
            } => {
                for operand in self.program.operands(arguments) {
                    self.operand_type(
                        function,
                        block,
                        Some(instruction),
                        origin,
                        *operand,
                        locals,
                    )?;
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
}
