use std::collections::{BTreeMap, BTreeSet};

use lpp_hir::{
    ArenaId, BinaryOperator, BindingTarget, BodyId, ExprId, ExpressionKind, FieldId, HirItem,
    HirItemId, HirItemKind, IdRange, Literal, LocalId, LocalKind, MatchArmId, ModuleId,
    NameBinding, OriginId, ScopeId, StatementKind, StmtId, Symbol, TypeParamId, UnaryOperator,
    VariantId,
};

use crate::{
    AggregateConstructor, AggregateExpressionFact, AugmentedAssignmentFact, BuiltinId,
    EnumMatchArmFact, EnumMatchFact, EnumTryFact, InferenceLevel, PlaceExpressionFact,
    PlaceStatementFact, PrimitiveType, TypeError, TypeId, TypeKind, TypeSubstitution,
};

use super::{ShadowTypeError, TypeChecker};

impl<'hir> TypeChecker<'hir> {
    pub(super) fn infer_item_expressions(
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

    fn infer_field_defaults(
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

    pub(super) fn infer_body(
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

    fn infer_statement(
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
    fn range_call_arguments(&self, iterable: ExprId) -> Option<Vec<ExprId>> {
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

    pub(super) fn infer_expression(
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
                Literal::String { .. } => self.interner.primitive(PrimitiveType::String),
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
                    Some(ty) if ty == self.interner.primitive(PrimitiveType::Int)
                        || ty == self.interner.primitive(PrimitiveType::Float)
                        || ty == self.interner.primitive(PrimitiveType::Bool)
                );
                if operator == BinaryOperator::Add && left_resolved == Some(string) && scalar_right
                {
                    // String concatenation with an implicit stringification of a
                    // scalar right operand (`"n=" + 3`, and the desugaring of
                    // `f"n={n}"`). Codegen routes this through the matching
                    // `*_to_str` builtin then `lpp_str_concat`.
                    string
                } else if is_comparison(operator) {
                    self.unify(left, right, expression.origin)?;
                    self.interner.primitive(PrimitiveType::Bool)
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

    fn record_constructor_call(&mut self, expression: ExprId, callee: ExprId) {
        let constructor = match self.package.expressions[callee].kind {
            ExpressionKind::Name {
                binding: NameBinding::Item(BindingTarget::Definition(definition)),
                ..
            } => self
                .definition_items
                .get(definition.index())
                .and_then(|item| *item)
                .and_then(|item| {
                    matches!(self.package.items[item].kind, HirItemKind::Struct(_))
                        .then_some(AggregateConstructor::Struct { item })
                }),
            _ => match self.aggregates.expression(callee) {
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
            },
        };
        if let Some(constructor) = constructor {
            self.aggregates.set_expression(
                expression,
                AggregateExpressionFact::Constructor(constructor),
            );
        }
    }

    fn infer_match_statement(
        &mut self,
        statement: StmtId,
        subject: ExprId,
        arms: IdRange<MatchArmId>,
        expected_result: TypeId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<(), ShadowTypeError> {
        let subject_type = self.infer_expression(subject, parameters)?;
        let subject_type = self
            .inference
            .resolve(&self.interner, subject_type, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.interner.kind(subject_type)
        else {
            return Err(self.at(
                origin,
                TypeError::InvalidMatchSubject {
                    actual: subject_type,
                },
            ));
        };
        let Some(item) = self.aggregates.item(definition) else {
            return Err(self.at(
                origin,
                TypeError::InvalidMatchSubject {
                    actual: subject_type,
                },
            ));
        };
        let HirItemKind::Enum(enumeration) = self.package.items[item].kind else {
            return Err(self.at(
                origin,
                TypeError::InvalidMatchSubject {
                    actual: subject_type,
                },
            ));
        };
        let variants = self.package.variants(enumeration.variants).to_vec();
        let total_variants = u32::try_from(variants.len()).map_err(|_| {
            self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: u32::MAX as usize,
                    actual: variants.len(),
                },
            )
        })?;
        let item_name = self.package.items[item]
            .name
            .expect("top-level enums retain their names");
        let mut covered = BTreeSet::new();
        let mut wildcard = None;
        let mut wildcard_seen = false;
        let arm_ids = self.package.match_arms(arms).to_vec();

        for arm_id in arm_ids {
            let arm = self.package.match_arms[arm_id];
            let path = self.package.symbols(arm.path);
            if arm.wildcard {
                if path.len() != 1 || !self.package.locals(arm.bindings).is_empty() {
                    return Err(self.at(arm.origin, TypeError::InvalidMatchPattern));
                }
                let reachable = !wildcard_seen && covered.len() < variants.len();
                if wildcard.is_none() {
                    wildcard = Some(arm_id);
                }
                wildcard_seen = true;
                self.enum_flow
                    .set_arm(arm_id, EnumMatchArmFact::Wildcard { reachable });
            } else {
                let variant_name = match path {
                    [variant] => *variant,
                    [qualifier, variant] if *qualifier == item_name => *variant,
                    _ => return Err(self.at(arm.origin, TypeError::InvalidMatchPattern)),
                };
                let Some(variant) = self.aggregates.variant(item, variant_name) else {
                    return Err(self.at(
                        arm.origin,
                        TypeError::UnknownVariant {
                            definition,
                            variant: variant_name,
                        },
                    ));
                };
                let field_range = self.package.variants[variant].fields;
                let source_fields = self.package.fields(field_range).to_vec();
                let binding_range = arm.bindings;
                let bindings = self.package.locals(binding_range).to_vec();
                if bindings.len() != source_fields.len() {
                    return Err(self.at(
                        arm.origin,
                        TypeError::MatchBindingArity {
                            variant,
                            expected: source_fields.len(),
                            actual: bindings.len(),
                        },
                    ));
                }
                for (binding, field) in bindings.into_iter().zip(source_fields) {
                    let binding_type = self.ensure_local(binding, parameters)?;
                    let field_type = self.nominal_field_type(item, field, arguments, arm.origin)?;
                    self.unify(field_type, binding_type, arm.origin)?;
                }
                let reachable = !wildcard_seen && covered.insert(variant);
                self.enum_flow.set_arm(
                    arm_id,
                    EnumMatchArmFact::Variant {
                        item,
                        variant,
                        fields: field_range,
                        bindings: binding_range,
                        reachable,
                    },
                );
            }
            self.infer_body(arm.body, expected_result, parameters)?;
        }

        let covered_variants = u32::try_from(covered.len()).map_err(|_| {
            self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: u32::MAX as usize,
                    actual: covered.len(),
                },
            )
        })?;
        self.enum_flow.set_match(
            statement,
            EnumMatchFact {
                item,
                subject_type,
                arms,
                covered_variants,
                total_variants,
                wildcard,
            },
        );
        Ok(())
    }

    fn infer_try_expression(
        &mut self,
        expression: ExprId,
        inner: ExprId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let carrier = self.infer_expression(inner, parameters)?;
        let carrier = self
            .inference
            .resolve(&self.interner, carrier, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.interner.kind(carrier)
        else {
            return Err(self.at(origin, TypeError::InvalidTryCarrier { actual: carrier }));
        };
        let Some(item) = self.aggregates.item(definition) else {
            return Err(self.at(origin, TypeError::InvalidTryCarrier { actual: carrier }));
        };
        let HirItemKind::Enum(enumeration) = self.package.items[item].kind else {
            return Err(self.at(origin, TypeError::InvalidTryCarrier { actual: carrier }));
        };
        let variants = self.package.variants(enumeration.variants).to_vec();
        let Some(success) = variants.first().copied() else {
            return Err(self.at(origin, TypeError::InvalidTryCarrier { actual: carrier }));
        };
        let success_fields = self
            .package
            .fields(self.package.variants[success].fields)
            .to_vec();
        if success_fields.len() != 1 {
            return Err(self.at(
                origin,
                TypeError::TrySuccessPayloadArity {
                    actual: success_fields.len(),
                },
            ));
        }
        let success_type = self.nominal_field_type(item, success_fields[0], arguments, origin)?;
        let Some(function_result) = self.current_function_result else {
            return Err(self.at(
                origin,
                TypeError::InvalidTryReturn {
                    carrier,
                    function_result: self.interner.primitive(PrimitiveType::Void),
                },
            ));
        };
        let function_result = self
            .inference
            .resolve(&self.interner, function_result, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        let TypeKind::Nominal {
            definition: result_definition,
            arguments: result_arguments,
        } = self.interner.kind(function_result)
        else {
            return Err(self.at(
                origin,
                TypeError::InvalidTryReturn {
                    carrier,
                    function_result,
                },
            ));
        };
        if result_definition != definition {
            return Err(self.at(
                origin,
                TypeError::InvalidTryReturn {
                    carrier,
                    function_result,
                },
            ));
        }
        let return_item = self
            .aggregates
            .item(result_definition)
            .expect("the matching nominal enum definition has an aggregate item");
        for variant in variants.iter().copied().skip(1) {
            for field in self
                .package
                .fields(self.package.variants[variant].fields)
                .iter()
                .copied()
            {
                let source = self.nominal_field_type(item, field, arguments, origin)?;
                let target = self.nominal_field_type(item, field, result_arguments, origin)?;
                self.unify(source, target, origin)
                    .map_err(|_| self.at(origin, TypeError::InvalidTryResidual { variant }))?;
            }
        }
        self.enum_flow.set_try(
            expression,
            EnumTryFact {
                item,
                carrier_type: carrier,
                success,
                success_field: success_fields[0],
                success_type,
                variants: enumeration.variants,
                return_item,
                return_type: function_result,
                direct_residual: carrier == function_result,
            },
        );
        Ok(success_type)
    }

    fn infer_index_expression(
        &mut self,
        expression: ExprId,
        base_expression: ExprId,
        index_expression: ExprId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let base = self.infer_expression(base_expression, parameters)?;
        let base = self
            .inference
            .resolve(&self.interner, base, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        let index_type = self.infer_expression(index_expression, parameters)?;
        let int = self.interner.primitive(PrimitiveType::Int);
        self.unify(int, index_type, origin)?;

        match self.interner.kind(base) {
            TypeKind::Tuple(elements) => {
                let ExpressionKind::Literal(Literal::Integer(index)) =
                    self.package.expressions[index_expression].kind
                else {
                    return Err(self.at(origin, TypeError::TupleIndexMustBeStatic));
                };
                let elements = self.interner.list(elements);
                let Ok(index_usize) = usize::try_from(index) else {
                    return Err(self.at(
                        origin,
                        TypeError::TupleIndexOutOfBounds {
                            index,
                            arity: elements.len(),
                        },
                    ));
                };
                let Some(element) = elements.get(index_usize).copied() else {
                    return Err(self.at(
                        origin,
                        TypeError::TupleIndexOutOfBounds {
                            index,
                            arity: elements.len(),
                        },
                    ));
                };
                let compact_index = u32::try_from(index_usize).map_err(|_| {
                    self.at(
                        origin,
                        TypeError::TupleIndexOutOfBounds {
                            index,
                            arity: elements.len(),
                        },
                    )
                })?;
                self.places.set_expression(
                    expression,
                    PlaceExpressionFact::TupleField {
                        index: compact_index,
                    },
                );
                Ok(element)
            }
            TypeKind::List(element) => {
                self.places
                    .set_expression(expression, PlaceExpressionFact::ListIndex);
                Ok(element)
            }
            TypeKind::Slice(element) => {
                // A slice element read `view[i]` is a value, not an assignable
                // place: it lowers to the `slice_get` runtime call. (For a
                // `StrSlice`, codegen dispatches `slice_get` to the
                // string-slice entry point by receiver type.)
                if let Some(slice_get) = self.builtin_index.lookup("slice_get") {
                    self.builtins.record(expression, slice_get);
                }
                Ok(element)
            }
            _ if base == self.interner.primitive(PrimitiveType::StrSlice) => {
                // `sview[i]` reads a character and yields a fresh one-character
                // string. It must use `slice_get` (dispatched to
                // `lpp_str_slice_get`), never `char_at`, which would misread the
                // view record as a string.
                if let Some(slice_get) = self.builtin_index.lookup("slice_get") {
                    self.builtins.record(expression, slice_get);
                }
                Ok(self.interner.primitive(PrimitiveType::String))
            }
            _ if base == self.interner.primitive(PrimitiveType::String) => {
                // String indexing is not an lvalue/place: `s[i]` produces a new
                // one-character string. Record a `char_at` builtin fact so MIR
                // lowers it to the runtime call instead of a place load.
                if let Some(char_at) = self.builtin_index.lookup("char_at") {
                    self.builtins.record(expression, char_at);
                }
                Ok(self.interner.primitive(PrimitiveType::String))
            }
            _ => {
                let element = self.fresh(InferenceLevel::ROOT.child(), origin)?;
                let list = self
                    .interner
                    .intern(TypeKind::List(element))
                    .map_err(|error| self.at(origin, error.into()))?;
                self.unify(list, base, origin)?;
                self.places
                    .set_expression(expression, PlaceExpressionFact::ListIndex);
                Ok(element)
            }
        }
    }

    fn validate_assignment_target(
        &self,
        target: ExprId,
        projected: bool,
        origin: OriginId,
    ) -> Result<(), ShadowTypeError> {
        match self.package.expressions[target].kind {
            ExpressionKind::Name {
                binding: NameBinding::Local(local),
                ..
            } => {
                let local = self.package.locals[local];
                if local.mutable || (projected && local.kind == LocalKind::Parameter) {
                    Ok(())
                } else {
                    Err(self.at(origin, TypeError::ImmutableAssignmentRoot))
                }
            }
            ExpressionKind::Field { base, .. } => {
                self.validate_assignment_target(base, true, origin)
            }
            ExpressionKind::Index { base, .. } => {
                if matches!(
                    self.places.expression(target),
                    Some(PlaceExpressionFact::TupleField { .. })
                ) {
                    return Err(self.at(origin, TypeError::TupleElementAssignment));
                }
                self.validate_assignment_target(base, true, origin)
            }
            _ => Err(self.at(origin, TypeError::InvalidAssignmentTarget)),
        }
    }

    fn infer_field_expression(
        &mut self,
        expression: ExprId,
        base: ExprId,
        name: Symbol,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let enum_item = match self.package.expressions[base].kind {
            ExpressionKind::Name {
                binding: NameBinding::Item(BindingTarget::Definition(definition)),
                ..
            } => self
                .definition_items
                .get(definition.index())
                .and_then(|item| *item)
                .filter(|item| matches!(self.package.items[*item].kind, HirItemKind::Enum(_))),
            _ => None,
        };
        let base_type = self.infer_expression(base, parameters)?;
        let resolved_base = self
            .inference
            .resolve(&self.interner, base_type, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;

        if let Some(item) = enum_item {
            let Some(variant) = self.aggregates.variant(item, name) else {
                let definition = self.package.items[item]
                    .definition
                    .expect("a top-level enum has a definition");
                return Err(self.at(
                    origin,
                    TypeError::UnknownVariant {
                        definition,
                        variant: name,
                    },
                ));
            };
            if self.package.variants[variant].fields.is_empty() {
                self.aggregates.set_expression(
                    expression,
                    AggregateExpressionFact::UnitVariant { item, variant },
                );
                return Ok(resolved_base);
            }
            let callable = self.variant_constructor_type(item, variant, resolved_base, origin)?;
            self.aggregates.set_expression(
                expression,
                AggregateExpressionFact::VariantConstructor { item, variant },
            );
            return Ok(callable);
        }

        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.interner.kind(resolved_base)
        else {
            return self.fresh(InferenceLevel::ROOT.child(), origin);
        };
        let Some(item) = self.aggregates.item(definition) else {
            return Err(self.at(
                origin,
                TypeError::InvalidAggregateBase {
                    actual: resolved_base,
                },
            ));
        };
        let HirItemKind::Struct(_) = self.package.items[item].kind else {
            return self.fresh(InferenceLevel::ROOT.child(), origin);
        };
        let Some(field) = self.aggregates.field(item, name) else {
            if self.call_callees.contains(&expression) {
                return self.fresh(InferenceLevel::ROOT.child(), origin);
            }
            return Err(self.at(
                origin,
                TypeError::UnknownField {
                    definition,
                    field: name,
                },
            ));
        };
        let field_type = self.nominal_field_type(item, field, arguments, origin)?;
        self.aggregates.set_expression(
            expression,
            AggregateExpressionFact::FieldProjection { item, field },
        );
        Ok(field_type)
    }

    fn variant_constructor_type(
        &mut self,
        item: HirItemId,
        variant: VariantId,
        nominal: TypeId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.interner.kind(nominal)
        else {
            return Err(self.at(origin, TypeError::InvalidAggregateBase { actual: nominal }));
        };
        if self.package.items[item].definition != Some(definition) {
            return Err(self.at(origin, TypeError::InvalidAggregateBase { actual: nominal }));
        }
        let fields = self
            .package
            .fields(self.package.variants[variant].fields)
            .to_vec();
        let mut field_types = Vec::with_capacity(fields.len());
        for field in fields {
            field_types.push(self.nominal_field_type(item, field, arguments, origin)?);
        }
        let parameters = self
            .interner
            .intern_list(&field_types)
            .map_err(|error| self.at(origin, error.into()))?;
        self.interner
            .intern(TypeKind::Function {
                parameters,
                result: nominal,
            })
            .map_err(|error| self.at(origin, error.into()))
    }

    fn nominal_field_type(
        &mut self,
        item: HirItemId,
        field: FieldId,
        arguments: crate::TypeListId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let item_value = self.package.items[item];
        let context = self.parameter_context(item);
        let template = self.resolve_type_ref(
            self.package.fields[field].type_ref,
            item_value.module,
            &context,
        )?;
        let parameters = self.item_parameters[item.index()].clone();
        let arguments = self.interner.list(arguments).to_vec();
        if parameters.len() != arguments.len() {
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: parameters.len(),
                    actual: arguments.len(),
                },
            ));
        }
        let mut substitution = TypeSubstitution::new();
        for (parameter, argument) in parameters.iter().zip(arguments) {
            substitution.insert(*parameter, argument);
        }
        substitution
            .apply(&mut self.interner, template, &mut self.budget)
            .map_err(|error| self.at(origin, error))
    }

    fn infer_name(
        &mut self,
        symbol: Symbol,
        binding: NameBinding,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        match binding {
            NameBinding::Local(local) => {
                if let Some(scheme) = self.assignments.local_schemes.get(&local).cloned() {
                    self.inference
                        .instantiate(
                            &mut self.interner,
                            &scheme,
                            InferenceLevel::ROOT.child(),
                            Some(origin),
                            &mut self.budget,
                        )
                        .map_err(|error| self.at(origin, error))
                } else {
                    self.ensure_local(local, parameters)
                }
            }
            NameBinding::Item(BindingTarget::Definition(definition)) => {
                self.instantiate_item(definition, origin)
            }
            NameBinding::Item(BindingTarget::Module(_)) => {
                self.fresh(InferenceLevel::ROOT.child(), origin)
            }
            NameBinding::Unresolved => {
                let spelling = self
                    .package
                    .names
                    .symbols
                    .resolve(symbol)
                    .expect("HIR symbols retain their spelling");
                let builtin_spelling = if spelling == "_getpid" {
                    "thread_id"
                } else {
                    spelling
                };
                if let Some(builtin) = self.builtin_index.lookup(builtin_spelling) {
                    return self.builtin_function_type(builtin, origin);
                }
                self.fresh(InferenceLevel::ROOT.child(), origin)
            }
        }
    }

    pub(super) fn builtin_function_type(
        &mut self,
        builtin: BuiltinId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let descriptor = builtin.descriptor();
        // `slice(list, start, len)` produces a `Slice[T]` view over a `List[T]`,
        // relating the element type of the source list to the slice result so
        // `view[i]` can recover it. (`str_slice` keeps its dedicated `StrSlice`
        // primitive result and needs no element variable.)
        if descriptor.name == "slice" {
            let element = self.fresh(InferenceLevel::ROOT.child(), origin)?;
            let list_type = self
                .interner
                .intern(TypeKind::List(element))
                .map_err(|error| self.at(origin, error.into()))?;
            let slice_type = self
                .interner
                .intern(TypeKind::Slice(element))
                .map_err(|error| self.at(origin, error.into()))?;
            let int = self.interner.primitive(PrimitiveType::Int);
            let parameters = self
                .interner
                .intern_list(&[list_type, int, int])
                .map_err(|error| self.at(origin, error.into()))?;
            return self
                .interner
                .intern(TypeKind::Function {
                    parameters,
                    result: slice_type,
                })
                .map_err(|error| self.at(origin, error.into()));
        }
        // Container builtins relate their list argument (or result, for
        // `list_new`) to one shared element variable, so lists get concrete
        // types: `list_new` yields `List(T)` for a fresh `T`, and the
        // element slots of `list_push`/`list_get`/`list_set` bind `T`
        // through the values that touch the list.
        if matches!(
            descriptor.name,
            "list_new" | "list_push" | "list_get" | "list_set" | "list_len"
        ) {
            use lpp_runtime_abi::generated::SemanticAbiType;
            let element = self.fresh(InferenceLevel::ROOT.child(), origin)?;
            let list_type = self
                .interner
                .intern(TypeKind::List(element))
                .map_err(|error| self.at(origin, error.into()))?;
            let parameter_types = if descriptor.name == "list_new" {
                vec![]
            } else {
                descriptor
                    .semantic_parameters
                    .iter()
                    .enumerate()
                    .map(|(index, kind)| -> Result<TypeId, ShadowTypeError> {
                        Ok(match (index, kind) {
                            (0, _) => list_type,
                            (_, SemanticAbiType::Any) => element,
                            _ => self.semantic_builtin_type(*kind, origin)?,
                        })
                    })
                    .collect::<Result<Vec<_>, ShadowTypeError>>()?
            };
            let parameters = self
                .interner
                .intern_list(&parameter_types)
                .map_err(|error| self.at(origin, error.into()))?;
            let result = if descriptor.name == "list_get" {
                element
            } else if descriptor.name == "list_new" {
                list_type
            } else {
                self.semantic_builtin_type(descriptor.semantic_result, origin)?
            };
            return self
                .interner
                .intern(TypeKind::Function { parameters, result })
                .map_err(|error| self.at(origin, error.into()));
        }
        let mut parameter_types = Vec::with_capacity(descriptor.semantic_parameters.len());
        for kind in descriptor.semantic_parameters {
            parameter_types.push(self.semantic_builtin_type(*kind, origin)?);
        }
        let parameters = self
            .interner
            .intern_list(&parameter_types)
            .map_err(|error| self.at(origin, error.into()))?;
        // Map membership predicates return an i64 flag in the frozen ABI but
        // are semantically boolean, so a program may use them directly as a
        // branch condition (`if map_has(m, k):`). Ground the result in `Bool`
        // so the condition type-checks and codegen accepts it.
        let result = if matches!(descriptor.name, "map_has" | "map_contains") {
            self.interner.primitive(PrimitiveType::Bool)
        } else {
            self.semantic_builtin_type(descriptor.semantic_result, origin)?
        };
        self.interner
            .intern(TypeKind::Function { parameters, result })
            .map_err(|error| self.at(origin, error.into()))
    }

    fn semantic_builtin_type(
        &mut self,
        kind: lpp_runtime_abi::generated::SemanticAbiType,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        use lpp_runtime_abi::generated::SemanticAbiType;
        // Shadow policy: only ABI kinds that map unambiguously to a language
        // primitive are constrained. The `Any` slot is opaque — a heap
        // handle, an element, or a list all share the same ABI shape — and
        // `void` results are likewise unconstrained, so container builtins
        // keep the same flexibility the shadow stage had before semantic
        // descriptors existed. A raw `I64` slot, by contrast, is a language
        // integer in every source builtin of the registry (indices, counts,
        // checksums, scalar vector lanes): grounding it in `Int` is what
        // lets `print_int(abs(5))` materialize, while the `Any` slots keep
        // the element flexibility.
        Ok(match kind {
            SemanticAbiType::Any | SemanticAbiType::I32 => {
                self.fresh(InferenceLevel::ROOT.child(), origin)?
            }
            SemanticAbiType::I64 => self.interner.primitive(PrimitiveType::Int),
            SemanticAbiType::Bool => self.interner.primitive(PrimitiveType::Bool),
            SemanticAbiType::F64 => self.interner.primitive(PrimitiveType::Float),
            SemanticAbiType::Str => self.interner.primitive(PrimitiveType::String),
            SemanticAbiType::StrSlice => self.interner.primitive(PrimitiveType::StrSlice),
            SemanticAbiType::VectorI64x2 => self.interner.primitive(PrimitiveType::VectorI64x2),
            SemanticAbiType::Void => self.interner.primitive(PrimitiveType::Void),
        })
    }

    fn record_builtin_call(&mut self, expression: ExprId, callee: ExprId) {
        let ExpressionKind::Name {
            symbol,
            binding: NameBinding::Unresolved,
        } = self.package.expressions[callee].kind
        else {
            return;
        };
        let spelling = self
            .package
            .names
            .symbols
            .resolve(symbol)
            .expect("HIR symbols retain their spelling");
        let builtin_spelling = if spelling == "_getpid" {
            "thread_id"
        } else {
            spelling
        };
        if let Some(builtin) = self.builtin_index.lookup(builtin_spelling) {
            let builtin =
                self.specialize_polymorphic_builtin(builtin_spelling, expression, builtin);
            self.builtins.record(expression, builtin);
        }
    }

    /// Some builtins are spelled generically (`len`) but bind to a single
    /// list-oriented runtime symbol in the ABI. When the argument is a string
    /// they must dispatch to the string-specific symbol instead, otherwise the
    /// string pointer is misread as a list header. This resolves that overload
    /// from the already-inferred argument type.
    pub(super) fn specialize_polymorphic_builtin(
        &mut self,
        spelling: &str,
        expression: ExprId,
        builtin: crate::BuiltinId,
    ) -> crate::BuiltinId {
        // Map key overload: the generic `map_put`/`map_get`/`map_has`/
        // `map_remove` builtins hash their key as an integer. When the key
        // argument is a string they must dispatch to the `_str` runtime symbol
        // (which hashes the string *contents* and compares with `strcmp`);
        // otherwise the string pointer is hashed as an opaque integer and two
        // equal strings at different addresses never match. The key is always
        // the second argument (index 1; the map is index 0).
        if let Some(str_variant) = match spelling {
            "map_put" => Some("lpp_map_put_str"),
            "map_get" => Some("lpp_map_get_str"),
            "map_has" => Some("lpp_map_has_str"),
            "map_remove" => Some("lpp_map_remove_str"),
            _ => None,
        } {
            if self.builtin_argument_is_string(expression, 1) {
                if let Some(variant) = self.builtin_index.lookup(str_variant) {
                    return variant;
                }
            }
            return builtin;
        }
        if spelling != "len" {
            return builtin;
        }
        let arguments = match self.package.expressions[expression].kind {
            ExpressionKind::Call { arguments, .. }
            | ExpressionKind::GenericCall { arguments, .. } => arguments,
            _ => return builtin,
        };
        let Some(&argument) = self.package.expressions(arguments).first() else {
            return builtin;
        };
        let Some(argument_type) = self.assignments.expressions[argument.index()] else {
            return builtin;
        };
        let Ok(resolved) = self
            .inference
            .resolve(&self.interner, argument_type, &mut self.budget)
        else {
            return builtin;
        };
        let kind = self.interner.kind(resolved);
        if matches!(kind, TypeKind::Primitive(PrimitiveType::String)) {
            if let Some(str_len) = self.builtin_index.lookup("str_len") {
                return str_len;
            }
        }
        // `len` on a slice view (`Slice[T]` or `StrSlice`) reads the view's
        // length field via `lpp_slice_len`, not the list header via
        // `lpp_list_len`.
        if matches!(
            kind,
            TypeKind::Slice(_) | TypeKind::Primitive(PrimitiveType::StrSlice)
        ) {
            if let Some(slice_len) = self.builtin_index.lookup("slice_len") {
                return slice_len;
            }
        }
        builtin
    }

    /// Whether the `index`-th argument of a builtin call resolves to a string.
    fn builtin_argument_is_string(&mut self, expression: ExprId, index: usize) -> bool {
        let arguments = match self.package.expressions[expression].kind {
            ExpressionKind::Call { arguments, .. }
            | ExpressionKind::GenericCall { arguments, .. } => arguments,
            _ => return false,
        };
        let Some(&argument) = self.package.expressions(arguments).get(index) else {
            return false;
        };
        let Some(argument_type) = self.assignments.expressions[argument.index()] else {
            return false;
        };
        let Ok(resolved) = self
            .inference
            .resolve(&self.interner, argument_type, &mut self.budget)
        else {
            return false;
        };
        matches!(
            self.interner.kind(resolved),
            TypeKind::Primitive(PrimitiveType::String)
        )
    }

    /// Whether a `slice_get(view, i)` call's first argument resolves to a
    /// `StrSlice`. Such a call reads a character and returns a fresh 1-char
    /// string, so its result type is `Str` and codegen must call
    /// `lpp_str_slice_get` instead of the numeric `lpp_slice_get`. The v1 ABI
    /// is frozen and has no dedicated builtin for this, so the overload lives in
    /// the checker (result type) and codegen (runtime symbol) rather than in the
    /// builtin table.
    pub(super) fn slice_get_on_str_slice(&mut self, expression: ExprId) -> bool {
        let arguments = match self.package.expressions[expression].kind {
            ExpressionKind::Call { arguments, .. }
            | ExpressionKind::GenericCall { arguments, .. } => arguments,
            _ => return false,
        };
        let Some(&argument) = self.package.expressions(arguments).first() else {
            return false;
        };
        let Some(argument_type) = self.assignments.expressions[argument.index()] else {
            return false;
        };
        let Ok(resolved) = self
            .inference
            .resolve(&self.interner, argument_type, &mut self.budget)
        else {
            return false;
        };
        matches!(
            self.interner.kind(resolved),
            TypeKind::Primitive(PrimitiveType::StrSlice)
        )
    }

    pub(super) fn validate_item_safety(
        &mut self,
        _item_id: HirItemId,
        item: HirItem,
    ) -> Result<(), ShadowTypeError> {
        let HirItemKind::Function(function) = item.kind else {
            return Ok(());
        };
        let Some(body) = function.body else {
            return Ok(());
        };
        self.validate_body_safety(body, function.is_async, None)
    }

    fn validate_body_safety(
        &mut self,
        body: BodyId,
        in_async: bool,
        closure_scope: Option<ScopeId>,
    ) -> Result<(), ShadowTypeError> {
        let mut borrowed_sources = BTreeSet::new();
        let statements = self
            .package
            .statements(self.package.bodies[body].statements)
            .to_vec();
        for statement in statements {
            self.validate_statement_safety(
                statement,
                in_async,
                closure_scope,
                &mut borrowed_sources,
            )?;
        }
        Ok(())
    }

    fn validate_statement_safety(
        &mut self,
        statement_id: StmtId,
        in_async: bool,
        closure_scope: Option<ScopeId>,
        borrowed_sources: &mut BTreeSet<LocalId>,
    ) -> Result<(), ShadowTypeError> {
        let statement = self.package.statements[statement_id];
        match statement.kind {
            StatementKind::Let { value, .. } => {
                self.validate_expression_safety(value, in_async, closure_scope)?;
                if let Some(source) = self.slice_source_local(value) {
                    borrowed_sources.insert(source);
                }
            }
            StatementKind::Assign { target, value } => {
                self.validate_expression_safety(target, in_async, closure_scope)?;
                self.validate_expression_safety(value, in_async, closure_scope)?;
                if let Some(local) = self.name_local(target) {
                    if borrowed_sources.contains(&local) {
                        return Err(self.at(
                            statement.origin,
                            TypeError::BorrowedSliceEscape {
                                reason: "source reassigned while a slice view is live",
                            },
                        ));
                    }
                }
            }
            StatementKind::Expression(expression) => {
                self.validate_expression_safety(expression, in_async, closure_scope)?;
            }
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    self.validate_expression_safety(value, in_async, closure_scope)?;
                    if self.expression_is_slice(value) {
                        return Err(self.at(
                            statement.origin,
                            TypeError::BorrowedSliceEscape {
                                reason: "slice view returned from its owner scope",
                            },
                        ));
                    }
                }
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.validate_expression_safety(condition, in_async, closure_scope)?;
                self.validate_body_safety(then_body, in_async, closure_scope)?;
                if let Some(else_body) = else_body {
                    self.validate_body_safety(else_body, in_async, closure_scope)?;
                }
            }
            StatementKind::While { condition, body } => {
                self.validate_expression_safety(condition, in_async, closure_scope)?;
                self.validate_body_safety(body, in_async, closure_scope)?;
            }
            StatementKind::For { iterable, body, .. } => {
                self.validate_expression_safety(iterable, in_async, closure_scope)?;
                self.validate_body_safety(body, in_async, closure_scope)?;
            }
            StatementKind::Match { subject, arms } => {
                self.validate_expression_safety(subject, in_async, closure_scope)?;
                for arm in self.package.match_arms(arms).to_vec() {
                    self.validate_body_safety(
                        self.package.match_arms[arm].body,
                        in_async,
                        closure_scope,
                    )?;
                }
            }
            StatementKind::Break | StatementKind::Continue => {}
        }
        Ok(())
    }

    fn validate_expression_safety(
        &mut self,
        expression_id: ExprId,
        in_async: bool,
        closure_scope: Option<ScopeId>,
    ) -> Result<(), ShadowTypeError> {
        let expression = self.package.expressions[expression_id];
        if let Some(scope) = closure_scope {
            if let Some(local) = self.name_local(expression_id) {
                if !self.scope_is_inside(self.package.locals[local].scope, scope) {
                    if self.local_is_slice(local) {
                        return Err(self.at(
                            expression.origin,
                            TypeError::BorrowedSliceEscape {
                                reason: "slice view captured by a closure",
                            },
                        ));
                    }
                    if self.local_is_task(local) {
                        return Err(self.at(expression.origin, TypeError::UnsafeAsyncTaskCapture));
                    }
                }
            }
        }

        match expression.kind {
            ExpressionKind::Unary { operand, .. } | ExpressionKind::Try(operand) => {
                self.validate_expression_safety(operand, in_async, closure_scope)?;
            }
            ExpressionKind::Binary { left, right, .. } => {
                self.validate_expression_safety(left, in_async, closure_scope)?;
                self.validate_expression_safety(right, in_async, closure_scope)?;
            }
            ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                for element in self.package.expressions(elements).to_vec() {
                    self.validate_expression_safety(element, in_async, closure_scope)?;
                }
            }
            ExpressionKind::Call { callee, arguments }
            | ExpressionKind::GenericCall {
                callee, arguments, ..
            } => {
                if in_async {
                    if let Some(name) = self.expression_name(callee) {
                        if let Some(builtin) = blocking_builtin_label(name) {
                            return Err(self.at(
                                expression.origin,
                                TypeError::UnsafeAsyncBlocking { builtin },
                            ));
                        }
                    }
                }
                self.validate_expression_safety(callee, in_async, closure_scope)?;
                for argument in self.package.expressions(arguments).to_vec() {
                    self.validate_expression_safety(argument, in_async, closure_scope)?;
                }
            }
            ExpressionKind::Field { base, .. } => {
                self.validate_expression_safety(base, in_async, closure_scope)?;
            }
            ExpressionKind::Index { base, index } => {
                self.validate_expression_safety(base, in_async, closure_scope)?;
                self.validate_expression_safety(index, in_async, closure_scope)?;
            }
            ExpressionKind::Await(inner) | ExpressionKind::Spawn(inner) => {
                self.validate_expression_safety(inner, in_async, closure_scope)?;
            }
            ExpressionKind::Closure { body, .. } => {
                let scope = self.package.bodies[body].scope;
                self.validate_body_safety(body, in_async, Some(scope))?;
            }
            ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
        }
        Ok(())
    }

    fn expression_name(&self, expression_id: ExprId) -> Option<&str> {
        let ExpressionKind::Name { symbol, .. } = self.package.expressions[expression_id].kind
        else {
            return None;
        };
        self.package.names.symbols.resolve(symbol)
    }

    fn name_local(&self, expression_id: ExprId) -> Option<LocalId> {
        let ExpressionKind::Name {
            binding: NameBinding::Local(local),
            ..
        } = self.package.expressions[expression_id].kind
        else {
            return None;
        };
        Some(local)
    }

    fn slice_source_local(&self, expression_id: ExprId) -> Option<LocalId> {
        let (ExpressionKind::Call { callee, arguments }
        | ExpressionKind::GenericCall {
            callee, arguments, ..
        }) = self.package.expressions[expression_id].kind
        else {
            return None;
        };
        match self.expression_name(callee) {
            Some("slice" | "str_slice") => {}
            _ => return None,
        }
        let source = *self.package.expressions(arguments).first()?;
        self.name_local(source)
    }

    fn expression_is_slice(&mut self, expression_id: ExprId) -> bool {
        let Some(type_id) = self.assignments.expressions[expression_id.index()] else {
            return false;
        };
        self.type_is_slice(type_id)
    }

    fn local_is_slice(&mut self, local: LocalId) -> bool {
        let Some(type_id) = self.assignments.locals[local.index()] else {
            return false;
        };
        self.type_is_slice(type_id)
    }

    fn local_is_task(&mut self, local: LocalId) -> bool {
        let Some(type_id) = self.assignments.locals[local.index()] else {
            return false;
        };
        let Ok(type_id) = self
            .inference
            .resolve(&self.interner, type_id, &mut self.budget)
        else {
            return false;
        };
        matches!(self.interner.kind(type_id), TypeKind::Task(_))
    }

    fn type_is_slice(&mut self, type_id: TypeId) -> bool {
        let Ok(type_id) = self
            .inference
            .resolve(&self.interner, type_id, &mut self.budget)
        else {
            return false;
        };
        matches!(
            self.interner.kind(type_id),
            TypeKind::Slice(_) | TypeKind::Primitive(PrimitiveType::StrSlice)
        )
    }

    fn scope_is_inside(&self, scope: ScopeId, ancestor: ScopeId) -> bool {
        let mut current = Some(scope);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.package.scopes[candidate].parent;
        }
        false
    }

    fn check_spawn_capture_target(
        &self,
        target: ExprId,
        origin: OriginId,
    ) -> Result<(), ShadowTypeError> {
        let Some(ancestor) = self.spawn_closure_scope else {
            return Ok(());
        };
        let ExpressionKind::Name {
            binding: NameBinding::Local(local),
            ..
        } = self.package.expressions[target].kind
        else {
            return Ok(());
        };
        if !self.scope_is_inside(self.package.locals[local].scope, ancestor) {
            return Err(self.at(origin, TypeError::SpawnCaptureMutation { local }));
        }
        Ok(())
    }
}

fn blocking_builtin_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "read_file" => "read_file",
        "write_file" => "write_file",
        "append_file" => "append_file",
        "delete_file" => "delete_file",
        "file_exists" => "file_exists",
        "read_line" => "read_line",
        "input" => "input",
        "file_size" => "file_size",
        "file_copy" => "file_copy",
        "file_move" => "file_move",
        "dir_create" => "dir_create",
        "dir_remove" => "dir_remove",
        "path_exists" => "path_exists",
        "command_output" => "command_output",
        "net_dial" => "net_dial",
        "net_dial_udp" => "net_dial_udp",
        "net_listen" => "net_listen",
        "net_listen_udp" => "net_listen_udp",
        "net_accept" => "net_accept",
        "net_accept_timeout" => "net_accept_timeout",
        "net_send" => "net_send",
        "net_send_all" => "net_send_all",
        "net_recv" => "net_recv",
        "net_recv_udp" => "net_recv_udp",
        "net_close" => "net_close",
        "net_set_deadline" => "net_set_deadline",
        "net_set_timeout" => "net_set_timeout",
        "net_set_nonblocking" => "net_set_nonblocking",
        "net_poll" => "net_poll",
        "net_set_keepalive" => "net_set_keepalive",
        "net_resolve" => "net_resolve",
        "net_connect" => "net_connect",
        "http_get" => "http_get",
        "http_post" => "http_post",
        "thread_join" => "thread_join",
        "sleep" => "sleep",
        "time_sleep" => "time_sleep",
        _ => return None,
    })
}

const fn is_comparison(operator: BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::Less
            | BinaryOperator::Greater
            | BinaryOperator::LessEqual
            | BinaryOperator::GreaterEqual
            | BinaryOperator::LogicalAnd
            | BinaryOperator::LogicalOr
    )
}
