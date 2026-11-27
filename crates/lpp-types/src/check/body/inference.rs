use super::*;

impl<'hir> TypeChecker<'hir> {
    pub(in crate::check) fn infer_item_expressions(
        &mut self,
        item_id: HirItemId,
        item: HirItem,
    ) -> Result<(), ShadowTypeError> {
        let parameters = self.parameter_context(item_id);
        match item.kind {
            HirItemKind::Function(function) => {
                let locals = self.package.locals(function.parameters).to_vec();
                for local in locals {
                    if let Some(default) = self.package.locals[local].default {
                        let expected = self.ensure_local(local, &parameters)?;
                        let actual = self.infer_expression(default, &parameters)?;
                        self.unify(expected, actual, item.origin)?;
                    }
                }
                if let Some(body) = function.body {
                    let result = self.function_results[item_id.index()]
                        .expect("function signatures record their body result");
                    self.current_function_result = Some(result);
                    let checked = self.infer_body(body, result, &parameters);
                    self.current_function_result = None;
                    checked?;
                }
            }
            HirItemKind::Struct(struct_) => {
                self.infer_field_defaults(
                    self.package.fields(struct_.fields).to_vec(),
                    item.module,
                    &parameters,
                )?;
            }
            HirItemKind::Enum(enum_) => {
                let variants = self.package.variants(enum_.variants).to_vec();
                for variant in variants {
                    self.infer_field_defaults(
                        self.package
                            .fields(self.package.variants[variant].fields)
                            .to_vec(),
                        item.module,
                        &parameters,
                    )?;
                }
            }
            HirItemKind::Const { value } => {
                let expected = self.assignments.items[item_id.index()]
                    .expect("constant registration allocates its type");
                let actual = self.infer_expression(value, &parameters)?;
                self.unify(expected, actual, item.origin)?;
            }
            HirItemKind::Trait(_)
            | HirItemKind::Impl(_)
            | HirItemKind::Extern(_)
            | HirItemKind::TypeAlias { .. } => {}
        }
        Ok(())
    }

    pub(in crate::check) fn infer_field_defaults(
        &mut self,
        fields: Vec<lpp_hir::FieldId>,
        module: ModuleId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
    ) -> Result<(), ShadowTypeError> {
        for field_id in fields {
            let field = self.package.fields[field_id];
            let expected = self.resolve_type_ref(field.type_ref, module, parameters)?;
            if let Some(default) = field.default {
                let actual = self.infer_expression(default, parameters)?;
                self.unify(expected, actual, field.origin)?;
            }
        }
        Ok(())
    }

    pub(in crate::check) fn infer_body(
        &mut self,
        body: BodyId,
        expected_result: TypeId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
    ) -> Result<(), ShadowTypeError> {
        let statement_ids = self
            .package
            .statements(self.package.bodies[body].statements)
            .to_vec();
        for statement in statement_ids {
            self.infer_statement(statement, expected_result, parameters)?;
        }
        Ok(())
    }

    pub(in crate::check) fn infer_statement(
        &mut self,
        statement_id: StmtId,
        expected_result: TypeId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
    ) -> Result<(), ShadowTypeError> {
        let statement = self.package.statements[statement_id];
        match statement.kind {
            StatementKind::Let { bindings, value } => {
                let value_type = self.infer_expression(value, parameters)?;
                let bindings = self.package.locals(bindings).to_vec();
                if bindings.len() == 1 {
                    let binding = bindings[0];
                    let local_type = self.ensure_local(binding, parameters)?;
                    self.unify(local_type, value_type, statement.origin)?;
                    // Generalization quantifies exactly the inference
                    // variables that are still unbound at the let. Those
                    // variables name opaque slots (unconstrained builtin
                    // results such as `list_get`), not parametric types:
                    // recording them as a scheme would give every later use
                    // a fresh instantiation, so the first use never binds
                    // the stored local type and the builder is left with a
                    // bare inference variable. Keep such locals monomorphic
                    // instead: the first use pins the slot's concrete type.
                } else {
                    let mut local_types = Vec::with_capacity(bindings.len());
                    for binding in &bindings {
                        local_types.push(self.ensure_local(*binding, parameters)?);
                    }
                    let elements = self
                        .interner
                        .intern_list(&local_types)
                        .map_err(|error| self.at(statement.origin, error.into()))?;
                    let tuple = self
                        .interner
                        .intern(TypeKind::Tuple(elements))
                        .map_err(|error| self.at(statement.origin, error.into()))?;
                    self.unify(tuple, value_type, statement.origin)?;
                    let arity = u32::try_from(bindings.len()).map_err(|_| {
                        self.at(
                            statement.origin,
                            TypeError::ArityMismatch {
                                expected: u32::MAX as usize,
                                actual: bindings.len(),
                            },
                        )
                    })?;
                    self.places.set_statement(
                        statement_id,
                        PlaceStatementFact::TupleDestructure { arity },
                    );
                }
            }
            StatementKind::Assign { target, value } => {
                let target_type = self.infer_expression(target, parameters)?;
                self.validate_assignment_target(target, false, statement.origin)?;
                self.check_spawn_capture_target(target, statement.origin)?;
                let value_type = self.infer_expression(value, parameters)?;
                self.unify(target_type, value_type, statement.origin)?;
                let augmented = match self.package.expressions[value].kind {
                    ExpressionKind::Binary {
                        left,
                        operator,
                        right,
                    } if left == target => Some(AugmentedAssignmentFact { operator, right }),
                    _ => None,
                };
                self.places
                    .set_statement(statement_id, PlaceStatementFact::Assignment { augmented });
            }
            StatementKind::Expression(expression) => {
                self.infer_expression(expression, parameters)?;
            }
            StatementKind::Return(value) => {
                let actual = if let Some(value) = value {
                    self.infer_expression(value, parameters)?
                } else {
                    self.interner.primitive(PrimitiveType::Void)
                };
                self.unify(expected_result, actual, statement.origin)?;
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.infer_expression(condition, parameters)?;
                self.infer_body(then_body, expected_result, parameters)?;
                if let Some(else_body) = else_body {
                    self.infer_body(else_body, expected_result, parameters)?;
                }
            }
            StatementKind::While { condition, body } => {
                self.infer_expression(condition, parameters)?;
                self.infer_body(body, expected_result, parameters)?;
            }
            StatementKind::Unsafe { body } => {
                self.infer_body(body, expected_result, parameters)?;
            }
            StatementKind::For {
                binding,
                iterable,
                body,
            } => {
                if let Some(arguments) = self.range_call_arguments(iterable) {
                    // `for VAR in range(..)` is an integer counting loop: pin the
                    // loop variable and every bound to `Int` so downstream type
                    // variables materialize even when the body only prints `VAR`.
                    // The range call itself is still inferred so every HIR
                    // expression (the call and its `range` callee) carries a type
                    // assignment.
                    let int = self.interner.primitive(PrimitiveType::Int);
                    self.infer_expression(iterable, parameters)?;
                    let element = self.ensure_local(binding, parameters)?;
                    self.unify(element, int, statement.origin)?;
                    for argument in arguments {
                        let argument_type = self.infer_expression(argument, parameters)?;
                        self.unify(argument_type, int, statement.origin)?;
                    }
                    self.places
                        .set_statement(statement_id, PlaceStatementFact::ListIteration);
                    self.infer_body(body, expected_result, parameters)?;
                } else {
                    let iterable = self.infer_expression(iterable, parameters)?;
                    let element = self.ensure_local(binding, parameters)?;
                    let list = self
                        .interner
                        .intern(TypeKind::List(element))
                        .map_err(|error| self.at(statement.origin, error.into()))?;
                    self.unify(list, iterable, statement.origin)?;
                    self.places
                        .set_statement(statement_id, PlaceStatementFact::ListIteration);
                    self.infer_body(body, expected_result, parameters)?;
                }
            }
            StatementKind::Match { subject, arms } => self.infer_match_statement(
                statement_id,
                subject,
                arms,
                expected_result,
                parameters,
                statement.origin,
            )?,
            StatementKind::Break | StatementKind::Continue => {}
        }
        Ok(())
    }

    /// Detect `range(..)` used as a `for` iterable: `range` resolves to no
    /// binding, so it is a counting-loop marker rather than a real value.
    /// Returns the 1..=3 argument expressions when `iterable` is such a call.
    pub(in crate::check) fn range_call_arguments(&self, iterable: ExprId) -> Option<Vec<ExprId>> {
        let ExpressionKind::Call { callee, arguments } = self.package.expressions[iterable].kind
        else {
            return None;
        };
        let ExpressionKind::Name {
            symbol,
            binding: NameBinding::Unresolved,
        } = self.package.expressions[callee].kind
        else {
            return None;
        };
        if self.package.names.symbols.resolve(symbol) != Some("range") {
            return None;
        }
        Some(self.package.expressions(arguments).to_vec())
    }

    pub(in crate::check) fn infer_expression(
        &mut self,
        expression_id: ExprId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
    ) -> Result<TypeId, ShadowTypeError> {
        if let Some(type_id) = self.assignments.expressions[expression_id.index()] {
            return Ok(type_id);
        }
        let expression = self.package.expressions[expression_id];
        let type_id = match expression.kind {
            ExpressionKind::Literal(literal) => match literal {
                Literal::Integer(_) => self.interner.primitive(PrimitiveType::Int),
                Literal::FloatBits(_) => self.interner.primitive(PrimitiveType::Float),
                Literal::String { .. } | Literal::RawString(_) => {
                    self.interner.primitive(PrimitiveType::String)
                }
                Literal::Character(_) => self.interner.primitive(PrimitiveType::Char),
                Literal::Bool(_) => self.interner.primitive(PrimitiveType::Bool),
            },
            ExpressionKind::Name { symbol, binding } => {
                self.infer_name(symbol, binding, parameters, expression.origin)?
            }
            ExpressionKind::Unary { operator, operand } => {
                let operand = self.infer_expression(operand, parameters)?;
                match operator {
                    UnaryOperator::Negate => operand,
                    UnaryOperator::Plus => {
                        let resolved = self
                            .inference
                            .resolve(&self.interner, operand, &mut self.budget)
                            .ok();
                        if let Some(ty) = resolved {
                            match self.interner.kind(ty) {
                                TypeKind::Primitive(p)
                                    if is_numeric_cast_primitive(p)
                                        && !matches!(
                                            p,
                                            PrimitiveType::Bool | PrimitiveType::Char
                                        ) => {}
                                TypeKind::InferenceVariable(_)
                                | TypeKind::GenericParameter(_)
                                | TypeKind::Error => {}
                                _ => {
                                    let int = self.interner.primitive(PrimitiveType::Int);
                                    self.unify(int, ty, expression.origin)?;
                                }
                            }
                        }
                        operand
                    }
                    UnaryOperator::BitNot => {
                        let resolved = self
                            .inference
                            .resolve(&self.interner, operand, &mut self.budget)
                            .ok();
                        if let Some(ty) = resolved {
                            match self.interner.kind(ty) {
                                TypeKind::Primitive(p)
                                    if is_integer_cast_primitive(p)
                                        && !matches!(
                                            p,
                                            PrimitiveType::Bool | PrimitiveType::Char
                                        ) => {}
                                TypeKind::InferenceVariable(_)
                                | TypeKind::GenericParameter(_)
                                | TypeKind::Error => {}
                                _ => {
                                    let int = self.interner.primitive(PrimitiveType::Int);
                                    self.unify(int, ty, expression.origin)?;
                                }
                            }
                        }
                        operand
                    }
                    UnaryOperator::Not => self.interner.primitive(PrimitiveType::Bool),
                }
            }
            ExpressionKind::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.infer_expression(left, parameters)?;
                let right = self.infer_expression(right, parameters)?;
                let string = self.interner.primitive(PrimitiveType::String);
                let int = self.interner.primitive(PrimitiveType::Int);
                let bool_ = self.interner.primitive(PrimitiveType::Bool);
                // A local's type is an inference variable bound to a primitive,
                // not the primitive TypeId itself, so resolve before comparing.
                // Resolution is best-effort: if it fails (e.g. a still-generic
                // operand) we fall through to the ordinary unify path.
                let left_resolved = self
                    .inference
                    .resolve(&self.interner, left, &mut self.budget)
                    .ok();
                let right_resolved = self
                    .inference
                    .resolve(&self.interner, right, &mut self.budget)
                    .ok();
                let scalar_right = matches!(
                    right_resolved,
                    Some(ty) if ty == int
                        || ty == self.interner.primitive(PrimitiveType::Float)
                        || ty == bool_
                        || ty == self.interner.primitive(PrimitiveType::Char)
                );
                let bool_int_eq = matches!(
                    operator,
                    BinaryOperator::Equal | BinaryOperator::NotEqual
                ) && matches!(
                    (left_resolved, right_resolved),
                    (Some(l), Some(r)) if (l == bool_ && r == int) || (l == int && r == bool_)
                );
                if operator == BinaryOperator::Add && left_resolved == Some(string) && scalar_right
                {
                    // String concatenation with an implicit stringification of a
                    // scalar right operand (`"n=" + 3`, and the desugaring of
                    // `f"n={n}"`). Codegen routes this through the matching
                    // `*_to_str` builtin then `lpp_str_concat`.
                    string
                } else if bool_int_eq {
                    bool_
                } else if is_comparison(operator) {
                    self.unify(left, right, expression.origin)?;
                    bool_
                } else {
                    self.unify(left, right, expression.origin)?;
                    left
                }
            }
            ExpressionKind::Tuple(elements) => {
                let elements = self.package.expressions(elements).to_vec();
                let mut types = Vec::with_capacity(elements.len());
                for element in elements {
                    types.push(self.infer_expression(element, parameters)?);
                }
                let types = self
                    .interner
                    .intern_list(&types)
                    .map_err(|error| self.at(expression.origin, error.into()))?;
                self.interner
                    .intern(TypeKind::Tuple(types))
                    .map_err(|error| self.at(expression.origin, error.into()))?
            }
            ExpressionKind::List(elements) => {
                let elements = self.package.expressions(elements).to_vec();
                let element_type = if let Some(first) = elements.first() {
                    self.infer_expression(*first, parameters)?
                } else {
                    self.fresh(InferenceLevel::ROOT.child(), expression.origin)?
                };
                for element in elements.iter().skip(1) {
                    let actual = self.infer_expression(*element, parameters)?;
                    self.unify(element_type, actual, expression.origin)?;
                }
                self.interner
                    .intern(TypeKind::List(element_type))
                    .map_err(|error| self.at(expression.origin, error.into()))?
            }
            ExpressionKind::Call { callee, arguments } => {
                let result = self.infer_call(
                    expression_id,
                    callee,
                    arguments,
                    None,
                    parameters,
                    expression.origin,
                )?;
                self.record_constructor_call(expression_id, callee);
                self.record_builtin_call(expression_id, callee);
                result
            }
            ExpressionKind::GenericCall {
                callee,
                type_arguments,
                arguments,
            } => {
                let type_arguments = self.package.type_refs(type_arguments).to_vec();
                let module = self.module_for_origin(expression.origin);
                let mut explicit = Vec::with_capacity(type_arguments.len());
                for type_argument in type_arguments {
                    explicit.push(self.resolve_type_ref(type_argument, module, parameters)?);
                }
                let result = self.infer_call(
                    expression_id,
                    callee,
                    arguments,
                    Some(&explicit),
                    parameters,
                    expression.origin,
                )?;
                self.record_constructor_call(expression_id, callee);
                self.record_builtin_call(expression_id, callee);
                result
            }
            ExpressionKind::Field { base, name } => self.infer_field_expression(
                expression_id,
                base,
                name,
                parameters,
                expression.origin,
            )?,
            ExpressionKind::Index { base, index } => self.infer_index_expression(
                expression_id,
                base,
                index,
                parameters,
                expression.origin,
            )?,
            ExpressionKind::Cast { value, target } => {
                let from = self.infer_expression(value, parameters)?;
                let module = self.module_for_origin(expression.origin);
                let to = self.resolve_type_ref(target, module, parameters)?;
                let from_resolved = self
                    .inference
                    .resolve(&self.interner, from, &mut self.budget)
                    .map_err(|error| self.at(expression.origin, error))?;
                let to_resolved = self
                    .inference
                    .resolve(&self.interner, to, &mut self.budget)
                    .map_err(|error| self.at(expression.origin, error))?;
                if !matches!(
                    self.interner.kind(from_resolved),
                    TypeKind::InferenceVariable(_)
                        | TypeKind::GenericParameter(_)
                        | TypeKind::Error
                ) && !matches!(
                    self.interner.kind(to_resolved),
                    TypeKind::InferenceVariable(_)
                        | TypeKind::GenericParameter(_)
                        | TypeKind::Error
                ) && !is_cast_compatible(&self.interner, from_resolved, to_resolved)
                {
                    return Err(self.at(
                        expression.origin,
                        TypeError::InvalidCast {
                            from: from_resolved,
                            to: to_resolved,
                        },
                    ));
                }
                to
            }
            ExpressionKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let condition_ty = self.infer_expression(condition, parameters)?;
                let bool_ = self.interner.primitive(PrimitiveType::Bool);
                self.unify(bool_, condition_ty, expression.origin)?;
                let then_ty = self.infer_expression(then_expr, parameters)?;
                let else_ty = self.infer_expression(else_expr, parameters)?;
                self.unify(then_ty, else_ty, expression.origin)?;
                then_ty
            }
            ExpressionKind::Try(inner) => {
                self.infer_try_expression(expression_id, inner, parameters, expression.origin)?
            }
            ExpressionKind::Await(inner) => {
                let task = self.infer_expression(inner, parameters)?;
                let result = self.fresh(InferenceLevel::ROOT.child(), expression.origin)?;
                let expected = self
                    .interner
                    .intern(TypeKind::Task(result))
                    .map_err(|error| self.at(expression.origin, error.into()))?;
                self.unify(expected, task, expression.origin)?;
                result
            }
            ExpressionKind::Spawn(inner) => {
                let closure_scope = match self.package.expressions[inner].kind {
                    ExpressionKind::Closure { body, .. } => Some(self.package.bodies[body].scope),
                    _ => None,
                };
                let previous = std::mem::replace(&mut self.spawn_closure_scope, closure_scope);
                let inferred = self.infer_expression(inner, parameters);
                self.spawn_closure_scope = previous;
                inferred?;
                self.interner.primitive(PrimitiveType::Void)
            }
            ExpressionKind::Closure {
                parameters: closure_parameters,
                return_type,
                body,
            } => {
                let locals = self.package.locals(closure_parameters).to_vec();
                let mut local_types = Vec::with_capacity(locals.len());
                for local in locals {
                    local_types.push(self.ensure_local(local, parameters)?);
                }
                let local_types = self
                    .interner
                    .intern_list(&local_types)
                    .map_err(|error| self.at(expression.origin, error.into()))?;
                let result = if let Some(return_type) = return_type {
                    self.resolve_type_ref(
                        return_type,
                        self.module_for_origin(expression.origin),
                        parameters,
                    )?
                } else {
                    self.fresh(InferenceLevel::ROOT.child(), expression.origin)?
                };
                let enclosing_result = self.current_function_result.replace(result);
                let checked = self.infer_body(body, result, parameters);
                self.current_function_result = enclosing_result;
                checked?;
                // Resolve the result: a closure with an implicit result type
                // whose body never returns a value leaves the fresh variable
                // unbound, which means the closure yields `Void`.
                let resolved = self
                    .inference
                    .resolve(&self.interner, result, &mut self.budget)
                    .map_err(|error| self.at(expression.origin, error))?;
                let result =
                    if matches!(self.interner.kind(resolved), TypeKind::InferenceVariable(_)) {
                        self.interner.primitive(PrimitiveType::Void)
                    } else {
                        resolved
                    };
                self.interner
                    .intern(TypeKind::Function {
                        parameters: local_types,
                        result,
                    })
                    .map_err(|error| self.at(expression.origin, error.into()))?
            }
        };
        self.assignments.expressions[expression_id.index()] = Some(type_id);
        Ok(type_id)
    }

    pub(in crate::check) fn record_constructor_call(&mut self, expression: ExprId, callee: ExprId) {
        let constructor = if let Some(definition) = self.package.expression_definition(callee) {
            self.definition_items
                .get(definition.index())
                .and_then(|item| *item)
                .and_then(|item| {
                    matches!(self.package.items[item].kind, HirItemKind::Struct(_))
                        .then_some(AggregateConstructor::Struct { item })
                })
        } else {
            match self.aggregates.expression(callee) {
                Some(AggregateExpressionFact::VariantConstructor { item, variant }) => {
                    Some(AggregateConstructor::EnumVariant { item, variant })
                }
                Some(AggregateExpressionFact::UnitVariant { item, variant }) => {
                    Some(AggregateConstructor::EnumVariant { item, variant })
                }
                Some(
                    AggregateExpressionFact::Constructor(_)
                    | AggregateExpressionFact::FieldProjection { .. }
                    | AggregateExpressionFact::MethodCall { .. },
                )
                | None => None,
            }
        };
        if let Some(constructor) = constructor {
            self.aggregates.set_expression(
                expression,
                AggregateExpressionFact::Constructor(constructor),
            );
        }
    }
}

fn is_integer_cast_primitive(p: PrimitiveType) -> bool {
    matches!(
        p,
        PrimitiveType::Int
            | PrimitiveType::Bool
            | PrimitiveType::Char
            | PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::USize
            | PrimitiveType::ISize
    )
}

fn is_numeric_cast_primitive(p: PrimitiveType) -> bool {
    is_integer_cast_primitive(p) || matches!(p, PrimitiveType::Float)
}

fn is_pointer_kind(interner: &crate::TypeInterner, ty: TypeId) -> bool {
    matches!(
        interner.kind(ty),
        TypeKind::Ptr(_) | TypeKind::Primitive(PrimitiveType::RawPtr)
    )
}

fn is_cast_compatible(interner: &crate::TypeInterner, from: TypeId, to: TypeId) -> bool {
    if from == to {
        return true;
    }
    match (interner.kind(from), interner.kind(to)) {
        (TypeKind::Primitive(a), TypeKind::Primitive(b))
            if is_numeric_cast_primitive(a) && is_numeric_cast_primitive(b) =>
        {
            true
        }
        (TypeKind::Primitive(a), _)
            if is_integer_cast_primitive(a) && is_pointer_kind(interner, to) =>
        {
            true
        }
        (_, TypeKind::Primitive(b))
            if is_pointer_kind(interner, from) && is_integer_cast_primitive(b) =>
        {
            true
        }
        _ if is_pointer_kind(interner, from) && is_pointer_kind(interner, to) => true,
        (TypeKind::Function { .. }, _) if is_pointer_kind(interner, to) => true,
        _ => false,
    }
}
