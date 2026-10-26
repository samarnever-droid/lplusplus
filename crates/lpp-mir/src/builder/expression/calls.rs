use super::*;

impl FunctionBuilder<'_, '_> {
    pub(in crate::builder) fn lower_call(
        &mut self,
        expression: ExprId,
        callee: ExprId,
        explicit_arguments: Option<IdRange<TypeRefId>>,
        arguments: IdRange<ExprId>,
        result_type: TypeId,
        depth: usize,
    ) -> Result<Operand, MirBuildError> {
        let origin = self.core.package.expressions[expression].origin;
        if let Some(fact) = self.core.types.builtins.expression(expression) {
            let descriptor = fact.builtin.descriptor();
            // Arity is the number of *semantic* parameters: machine-level
            // lowering may fold arguments into registers or omit them.
            let expected = descriptor.semantic_parameters.len();
            let actual = self.core.package.expressions(arguments).len();
            if actual != expected {
                return Err(self.core.error(
                    origin,
                    MirBuildErrorKind::InvalidBuiltinArity { expected, actual },
                ));
            }
            let arguments = self.lower_arguments(arguments, origin, depth)?;
            return self.assign_temporary(
                result_type,
                Rvalue::Builtin {
                    builtin: fact.builtin,
                    arguments,
                },
                origin,
            );
        }
        if let Some(AggregateExpressionFact::MethodCall { method }) =
            self.core.types.aggregates.expression(expression)
        {
            return self.lower_method_call(callee, method, arguments, result_type, origin, depth);
        }
        if let Some(AggregateExpressionFact::Constructor(constructor)) =
            self.core.types.aggregates.expression(expression)
        {
            let arguments = self.lower_arguments(arguments, origin, depth)?;
            let aggregate = self.core.ensure_aggregate(result_type, origin)?;
            let value = match constructor {
                AggregateConstructor::Struct { item } => {
                    if self.core.program.aggregate(aggregate).unwrap().source != item {
                        return Err(self
                            .core
                            .error(origin, MirBuildErrorKind::InvalidAggregateType(result_type)));
                    }
                    // v1 zero-argument struct construction (`Box()`): the type
                    // checker accepted the empty argument list, so synthesize
                    // one zero constant per concrete field here. A full
                    // positional argument list is lowered as-is.
                    let fields = if self.core.program.operands(arguments).is_empty() {
                        self.zero_init_struct_fields(aggregate, origin)?
                    } else {
                        arguments
                    };
                    Rvalue::ConstructStruct { aggregate, fields }
                }
                AggregateConstructor::EnumVariant { item, variant } => {
                    if self.core.program.aggregate(aggregate).unwrap().source != item {
                        return Err(self
                            .core
                            .error(origin, MirBuildErrorKind::InvalidAggregateType(result_type)));
                    }
                    let variant = self
                        .core
                        .aggregate_variants
                        .get(&(aggregate, variant))
                        .copied()
                        .ok_or_else(|| {
                            self.core.error(
                                origin,
                                MirBuildErrorKind::MissingAggregateFact { expression },
                            )
                        })?;
                    Rvalue::ConstructVariant {
                        aggregate,
                        variant,
                        fields: arguments,
                    }
                }
            };
            return self.assign_temporary(result_type, value, origin);
        }

        if let Some(definition) = self.core.package.expression_definition(callee)
            && let Some(item) = self
                .core
                .definition_items
                .get(definition.index())
                .and_then(|item| *item)
            && self.core.is_external_function(item)
        {
            if explicit_arguments.is_some() {
                return Err(self
                    .core
                    .error(origin, MirBuildErrorKind::UnsupportedExternalSignature));
            }
            let function = self.core.register_external_function(item, origin)?;
            let arguments = self.lower_arguments(arguments, origin, depth)?;
            return self.assign_temporary(
                result_type,
                Rvalue::ExternalCall {
                    function,
                    arguments,
                },
                origin,
            );
        }

        if let Some(fixed) = self.variadic_fixed_arity(callee) {
            return self.lower_variadic_call(callee, fixed, arguments, result_type, origin, depth);
        }
        if let Some(param_locals) = self.default_param_locals(callee) {
            let provided = self.core.package.expressions(arguments).len();
            if provided < param_locals.len() {
                return self.lower_defaulted_call(
                    callee,
                    &param_locals,
                    arguments,
                    result_type,
                    origin,
                    depth,
                );
            }
        }

        let callee = match (
            explicit_arguments,
            self.core.package.expression_definition(callee),
        ) {
            (Some(type_arguments), Some(definition)) => Operand::Function(
                self.lower_function_name(callee, definition, Some(type_arguments))?,
            ),
            _ => self.lower_expression(callee, depth)?,
        };
        let arguments = self.lower_arguments(arguments, origin, depth)?;
        self.assign_temporary(result_type, Rvalue::Call { callee, arguments }, origin)
    }

    /// Lower a UFCS method call `receiver.method(args..)` resolved by the type
    /// checker into a direct call `method(receiver, args..)`.
    pub(in crate::builder) fn lower_method_call(
        &mut self,
        callee: ExprId,
        method: HirItemId,
        arguments: IdRange<ExprId>,
        result_type: TypeId,
        origin: OriginId,
        depth: usize,
    ) -> Result<Operand, MirBuildError> {
        let ExpressionKind::Field { base, name } = self.core.package.expressions[callee].kind
        else {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::MissingAggregateFact { expression: callee },
            ));
        };
        // Re-resolve the method against the *concrete* receiver type for this
        // monomorphized instance. The type checker may have recorded a trait's
        // abstract method (for a trait-bounded generic receiver) or a base
        // candidate; once the instance substitution makes the receiver concrete
        // (e.g. `Box` or `Buffer`), the real `impl` method is known by name.
        let method = self.resolve_concrete_method(base, name, method, origin);
        let function_id = self.lower_method_function(callee, method, origin)?;
        let receiver = self.lower_expression(base, depth)?;
        let argument_ids = self.core.package.expressions(arguments).to_vec();
        let mut operands = Vec::with_capacity(argument_ids.len() + 1);
        operands.push(receiver);
        for argument in argument_ids {
            operands.push(self.lower_expression(argument, depth)?);
        }
        let arguments = self.store_operands(&operands, origin)?;
        self.assign_temporary(
            result_type,
            Rvalue::Call {
                callee: Operand::Function(function_id),
                arguments,
            },
            origin,
        )
    }

    pub(in crate::builder) fn lower_method_function(
        &mut self,
        callee: ExprId,
        method: HirItemId,
        origin: OriginId,
    ) -> Result<MirFunctionId, MirBuildError> {
        let HirItemKind::Function(function) = self.core.package.items[method].kind else {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
            ));
        };
        if function.type_parameters.is_empty() {
            return self
                .core
                .item_functions
                .get(method.index())
                .and_then(|function| *function)
                .ok_or_else(|| {
                    self.core.error(
                        origin,
                        MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
                    )
                });
        }
        let method_type = self
            .core
            .types
            .assignments
            .expression(callee)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(callee)),
                )
            })?;
        let method_type = self.concrete_type(method_type, origin)?;
        self.core
            .instance_type_functions
            .get(&(method, method_type))
            .and_then(|function| *function)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingFunctionInstance {
                        item: method,
                        ty: method_type,
                    },
                )
            })
    }

    /// Re-resolve a method call to the concrete `impl` method for the receiver's
    /// monomorphized type. Falls back to `fallback` (the item the type checker
    /// recorded) when the receiver type is not a known nominal or no concrete
    /// method with a lowered body exists for `(receiver definition, name)`.
    pub(in crate::builder) fn resolve_concrete_method(
        &mut self,
        base: ExprId,
        name: lpp_hir::Symbol,
        fallback: HirItemId,
        origin: OriginId,
    ) -> HirItemId {
        let Some(base_type) = self.core.types.assignments.expression(base) else {
            return fallback;
        };
        let Ok(concrete) = self.concrete_type(base_type, origin) else {
            return fallback;
        };
        let TypeKind::Nominal { definition, .. } = self.core.types.interner.kind(concrete) else {
            return fallback;
        };
        let Some(candidates) = self.core.types.method_candidates.get(&(definition, name)) else {
            return fallback;
        };
        let mut candidates = candidates.clone();
        candidates.sort_by_key(|method| match self.core.package.items[*method].kind {
            HirItemKind::Function(function) => self
                .core
                .package
                .type_parameters(function.type_parameters)
                .len(),
            _ => usize::MAX,
        });
        for candidate in candidates {
            if self.method_receiver_matches(candidate, concrete, origin) {
                return candidate;
            }
        }
        fallback
    }

    pub(in crate::builder) fn method_receiver_matches(
        &mut self,
        method: HirItemId,
        concrete_receiver: TypeId,
        origin: OriginId,
    ) -> bool {
        let Some(method_type) = self.core.types.assignments.item(method) else {
            return false;
        };
        let TypeKind::Function { parameters, .. } = self.core.types.interner.kind(method_type)
        else {
            return false;
        };
        let Some(receiver) = self.core.types.interner.list(parameters).first().copied() else {
            return false;
        };
        let Ok(receiver) = self.concrete_type(receiver, origin) else {
            return false;
        };
        if receiver == concrete_receiver {
            return true;
        }
        match (
            self.core.types.interner.kind(receiver),
            self.core.types.interner.kind(concrete_receiver),
        ) {
            (
                TypeKind::Nominal {
                    definition: left,
                    arguments,
                },
                TypeKind::Nominal {
                    definition: right, ..
                },
            ) if left == right => self
                .core
                .types
                .interner
                .list(arguments)
                .iter()
                .any(|argument| {
                    matches!(
                        self.core.types.interner.kind(*argument),
                        TypeKind::GenericParameter(_)
                    )
                }),
            _ => false,
        }
    }

    /// When `callee` names a variadic function, the count of leading fixed
    /// parameters. Mirrors the type checker's `variadic_fixed_arity`.
    pub(in crate::builder) fn variadic_fixed_arity(&self, callee: ExprId) -> Option<usize> {
        let definition = self.core.package.expression_definition(callee)?;
        let item = self
            .core
            .definition_items
            .get(definition.index())
            .and_then(|item| *item)?;
        let HirItemKind::Function(function) = self.core.package.items[item].kind else {
            return None;
        };
        if !function.variadic {
            return None;
        }
        Some(
            self.core
                .package
                .locals(function.parameters)
                .len()
                .saturating_sub(1),
        )
    }

    /// When `callee` names a function that has at least one default parameter,
    /// returns its parameter locals (so omitted trailing arguments can be
    /// filled from each parameter's default expression).
    pub(in crate::builder) fn default_param_locals(&self, callee: ExprId) -> Option<Vec<LocalId>> {
        let definition = self.core.package.expression_definition(callee)?;
        let item = self
            .core
            .definition_items
            .get(definition.index())
            .and_then(|item| *item)?;
        let HirItemKind::Function(function) = self.core.package.items[item].kind else {
            return None;
        };
        let locals = self.core.package.locals(function.parameters).to_vec();
        if locals
            .iter()
            .any(|local| self.core.package.locals[*local].default.is_some())
        {
            Some(locals)
        } else {
            None
        }
    }

    /// Lower a call that omits trailing defaulted arguments: supplied arguments
    /// are lowered positionally, and each omitted parameter contributes its
    /// default expression.
    pub(in crate::builder) fn lower_defaulted_call(
        &mut self,
        callee: ExprId,
        param_locals: &[LocalId],
        arguments: IdRange<ExprId>,
        result_type: TypeId,
        origin: OriginId,
        depth: usize,
    ) -> Result<Operand, MirBuildError> {
        let provided = self.core.package.expressions(arguments).to_vec();
        let callee_operand = self.lower_expression(callee, depth)?;
        let mut operands = Vec::with_capacity(param_locals.len());
        for (index, param) in param_locals.iter().enumerate() {
            if let Some(argument) = provided.get(index) {
                operands.push(self.lower_expression(*argument, depth)?);
            } else {
                let default = self.core.package.locals[*param].default.ok_or_else(|| {
                    self.core.error(
                        origin,
                        MirBuildErrorKind::Unsupported(UnsupportedConstruct::ParameterDefault),
                    )
                })?;
                operands.push(self.lower_expression(default, depth)?);
            }
        }
        let call_arguments = self.store_operands(&operands, origin)?;
        self.assign_temporary(
            result_type,
            Rvalue::Call {
                callee: callee_operand,
                arguments: call_arguments,
            },
            origin,
        )
    }

    /// Lower a call to a variadic function: pass the leading `fixed` arguments
    /// positionally and gather every trailing argument into the rest
    /// parameter's `List[element]` (empty list when none are supplied).
    pub(in crate::builder) fn lower_variadic_call(
        &mut self,
        callee: ExprId,
        fixed: usize,
        arguments: IdRange<ExprId>,
        result_type: TypeId,
        origin: OriginId,
        depth: usize,
    ) -> Result<Operand, MirBuildError> {
        let argument_ids = self.core.package.expressions(arguments).to_vec();
        if argument_ids.len() < fixed {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::InvalidBuiltinArity {
                    expected: fixed,
                    actual: argument_ids.len(),
                },
            ));
        }
        // The rest-parameter list type is the last parameter of the callee's
        // function type.
        let callee_type = self
            .core
            .types
            .assignments
            .expression(callee)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(callee)),
                )
            })?;
        let TypeKind::Function { parameters, .. } = self.core.types.interner.kind(callee_type)
        else {
            return Err(self
                .core
                .error(origin, MirBuildErrorKind::InvalidFunctionType(callee_type)));
        };
        let parameter_types = self.core.types.interner.list(parameters).to_vec();
        let list_type = *parameter_types.last().ok_or_else(|| {
            self.core
                .error(origin, MirBuildErrorKind::InvalidFunctionType(callee_type))
        })?;

        let callee_operand = self.lower_expression(callee, depth)?;
        let mut operands = Vec::with_capacity(fixed + 1);
        for argument in &argument_ids[..fixed] {
            operands.push(self.lower_expression(*argument, depth)?);
        }
        let mut rest = Vec::with_capacity(argument_ids.len() - fixed);
        for argument in &argument_ids[fixed..] {
            rest.push(self.lower_expression(*argument, depth)?);
        }
        let rest_operands = self.store_operands(&rest, origin)?;
        let list = self.assign_temporary(list_type, Rvalue::List(rest_operands), origin)?;
        operands.push(list);
        let call_arguments = self.store_operands(&operands, origin)?;
        self.assign_temporary(
            result_type,
            Rvalue::Call {
                callee: callee_operand,
                arguments: call_arguments,
            },
            origin,
        )
    }

    pub(in crate::builder) fn lower_arguments(
        &mut self,
        arguments: IdRange<ExprId>,
        origin: OriginId,
        depth: usize,
    ) -> Result<ListRange<Operand>, MirBuildError> {
        let argument_ids = self.core.package.expressions(arguments).to_vec();
        let mut operands = Vec::with_capacity(argument_ids.len());
        for argument in argument_ids {
            operands.push(self.lower_expression(argument, depth)?);
        }
        self.store_operands(&operands, origin)
    }
}
