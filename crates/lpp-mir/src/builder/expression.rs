use super::*;

impl FunctionBuilder<'_, '_> {
    pub(super) fn lower_expression(
        &mut self,
        expression_id: ExprId,
        depth: usize,
    ) -> Result<Operand, MirBuildError> {
        let expression = self.core.package.expressions[expression_id];
        if depth >= self.core.options.max_expression_depth {
            return Err(self.core.error(
                expression.origin,
                MirBuildErrorKind::Capacity(MirCapacity::ExpressionDepth),
            ));
        }
        let ty = self
            .core
            .types
            .assignments
            .expression(expression_id)
            .ok_or_else(|| {
                self.core.error(
                    expression.origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(expression_id)),
                )
            })?;
        let ty = self.concrete_type(ty, expression.origin)?;
        let next_depth = depth + 1;
        match expression.kind {
            ExpressionKind::Literal(literal) => {
                let constant = match literal {
                    Literal::Integer(value) => Constant::Integer(value),
                    Literal::FloatBits(value) => Constant::FloatBits(value),
                    Literal::String { span, formatted } => {
                        let string =
                            self.core
                                .materialize_string(span, formatted, expression.origin)?;
                        Constant::String {
                            origin: expression.origin,
                            string,
                        }
                    }
                    Literal::Character(span) => {
                        let character = self.core.materialize_character(span, expression.origin)?;
                        Constant::Character {
                            origin: expression.origin,
                            character,
                        }
                    }
                    Literal::Bool(value) => Constant::Bool(value),
                };
                // Integer literals are context-sensitive in source: the type
                // checker may assign one to a fixed-width integer, or accept
                // literal zero as the null sentinel for a nominal reference.
                // MIR constants deliberately retain the canonical `Int` type,
                // so materialize a typed temporary whenever inference selected
                // another type. This keeps every later use (constructor fields,
                // comparisons, stores) consistently typed instead of relying on
                // each consumer to reinterpret a bare integer constant.
                if matches!(constant, Constant::Integer(_))
                    && ty != self.core.types.interner.primitive(PrimitiveType::Int)
                {
                    self.assign_temporary(
                        ty,
                        Rvalue::Use(Operand::Constant(constant)),
                        expression.origin,
                    )
                } else {
                    Ok(Operand::Constant(constant))
                }
            }
            ExpressionKind::Name { binding, .. } => match binding {
                NameBinding::Local(source) => {
                    let local = self.local(source, MirLocalKind::User)?;
                    if self.core.cell_sources.contains(&source) {
                        // By-reference capture cell: reading the local
                        // yields the head element of its list.
                        let place = self.place(
                            local,
                            &[PlaceProjection::ListIndex(Operand::Constant(
                                Constant::Integer(0),
                            ))],
                            ty,
                            expression.origin,
                        )?;
                        self.assign_temporary(ty, Rvalue::Load(place), expression.origin)
                    } else {
                        Ok(Operand::Copy(local))
                    }
                }
                NameBinding::Item(lpp_hir::BindingTarget::Definition(definition)) => {
                    // A reference to a `const` inlines the constant's value
                    // expression; any other definition is a function value.
                    if let Some(value) = self.const_value(definition) {
                        self.lower_expression(value, next_depth)
                    } else {
                        self.lower_function_name(expression_id, definition, None)
                            .map(Operand::Function)
                    }
                }
                NameBinding::Item(lpp_hir::BindingTarget::Module(_)) => Err(self.core.error(
                    expression.origin,
                    MirBuildErrorKind::Unsupported(UnsupportedConstruct::ModuleValue),
                )),
                NameBinding::Unresolved => Err(self.core.error(
                    expression.origin,
                    MirBuildErrorKind::Unsupported(UnsupportedConstruct::UnresolvedName),
                )),
            },
            ExpressionKind::Unary { operator, operand } => {
                let operand = self.lower_expression(operand, next_depth)?;
                self.assign_temporary(ty, Rvalue::Unary { operator, operand }, expression.origin)
            }
            ExpressionKind::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.lower_expression(left, next_depth)?;
                let right = self.lower_expression(right, next_depth)?;
                self.assign_temporary(
                    ty,
                    Rvalue::Binary {
                        left,
                        operator,
                        right,
                    },
                    expression.origin,
                )
            }
            ExpressionKind::Tuple(elements) => {
                let elements = self.core.package.expressions(elements).to_vec();
                let mut operands = Vec::with_capacity(elements.len());
                for element in elements {
                    operands.push(self.lower_expression(element, next_depth)?);
                }
                let operands = self.store_operands(&operands, expression.origin)?;
                self.assign_temporary(ty, Rvalue::Tuple(operands), expression.origin)
            }
            ExpressionKind::List(elements) => {
                let elements = self.core.package.expressions(elements).to_vec();
                let mut operands = Vec::with_capacity(elements.len());
                for element in elements {
                    operands.push(self.lower_expression(element, next_depth)?);
                }
                let operands = self.store_operands(&operands, expression.origin)?;
                self.assign_temporary(ty, Rvalue::List(operands), expression.origin)
            }
            ExpressionKind::Call { callee, arguments } => {
                self.lower_call(expression_id, callee, None, arguments, ty, next_depth)
            }
            ExpressionKind::GenericCall {
                callee,
                type_arguments,
                arguments,
            } => self.lower_call(
                expression_id,
                callee,
                Some(type_arguments),
                arguments,
                ty,
                next_depth,
            ),
            ExpressionKind::Field { .. } => {
                if let Some(AggregateExpressionFact::UnitVariant { item, variant }) =
                    self.core.types.aggregates.expression(expression_id)
                {
                    let aggregate = self.core.ensure_aggregate(ty, expression.origin)?;
                    if self.core.program.aggregate(aggregate).unwrap().source != item {
                        return Err(self.core.error(
                            expression.origin,
                            MirBuildErrorKind::InvalidAggregateType(ty),
                        ));
                    }
                    let variant = self.core.aggregate_variants[&(aggregate, variant)];
                    let fields = self.store_operands(&[], expression.origin)?;
                    self.assign_temporary(
                        ty,
                        Rvalue::ConstructVariant {
                            aggregate,
                            variant,
                            fields,
                        },
                        expression.origin,
                    )
                } else {
                    let place = self.lower_place(expression_id, next_depth)?;
                    self.assign_temporary(ty, Rvalue::Load(place), expression.origin)
                }
            }
            ExpressionKind::Index { base, index } => {
                // String indexing carries a builtin fact (`char_at`) and lowers
                // to a runtime call rather than a place load — a string index is
                // a value, not an assignable place.
                if let Some(fact) = self.core.types.builtins.expression(expression_id) {
                    let base_operand = self.lower_expression(base, next_depth)?;
                    let index_operand = self.lower_expression(index, next_depth)?;
                    let arguments =
                        self.store_operands(&[base_operand, index_operand], expression.origin)?;
                    self.assign_temporary(
                        ty,
                        Rvalue::Builtin {
                            builtin: fact.builtin,
                            arguments,
                        },
                        expression.origin,
                    )
                } else {
                    let place = self.lower_place(expression_id, next_depth)?;
                    self.assign_temporary(ty, Rvalue::Load(place), expression.origin)
                }
            }
            ExpressionKind::Try(inner) => {
                self.lower_try(expression_id, inner, next_depth, expression.origin)
            }
            ExpressionKind::Await(inner) => {
                let inner_type =
                    self.core
                        .types
                        .assignments
                        .expression(inner)
                        .ok_or_else(|| {
                            self.core.error(
                                self.core.package.expressions[inner].origin,
                                MirBuildErrorKind::MissingType(TypedEntity::Expression(inner)),
                            )
                        })?;
                let TypeKind::Task(_) = self.core.types.interner.kind(inner_type) else {
                    return Err(self.core.error(
                        expression.origin,
                        MirBuildErrorKind::InvalidAwaitTarget(inner_type),
                    ));
                };
                let operand = self.lower_expression(inner, next_depth)?;
                self.assign_temporary(ty, Rvalue::Await(operand), expression.origin)
            }
            ExpressionKind::Spawn(inner) => {
                let inner_type =
                    self.core
                        .types
                        .assignments
                        .expression(inner)
                        .ok_or_else(|| {
                            self.core.error(
                                self.core.package.expressions[inner].origin,
                                MirBuildErrorKind::MissingType(TypedEntity::Expression(inner)),
                            )
                        })?;
                let TypeKind::Function {
                    parameters: _,
                    result,
                } = self.core.types.interner.kind(inner_type)
                else {
                    return Err(self.core.error(
                        expression.origin,
                        MirBuildErrorKind::InvalidSpawnTarget(inner_type),
                    ));
                };
                let void = self.core.types.interner.primitive(PrimitiveType::Void);
                if result != void {
                    return Err(self.core.error(
                        expression.origin,
                        MirBuildErrorKind::InvalidSpawnTarget(inner_type),
                    ));
                }
                let operand = self.lower_expression(inner, next_depth)?;
                self.assign_temporary(ty, Rvalue::Spawn(operand), expression.origin)
            }
            ExpressionKind::Closure {
                parameters,
                return_type: _,
                body,
            } => {
                let closure_scope = self.core.package.bodies[body].scope;
                let capture_sources = self.core.compute_captures(body, closure_scope);
                let mut capture_locals = Vec::with_capacity(capture_sources.len());
                for source in &capture_sources {
                    capture_locals.push(self.local(*source, MirLocalKind::User)?);
                }
                let capture_operands: Vec<Operand> = capture_locals
                    .iter()
                    .map(|local| Operand::Copy(*local))
                    .collect();
                let function = self.core.lower_closure_function(
                    expression_id,
                    parameters,
                    body,
                    self.source,
                    self.substitution.clone(),
                    capture_locals,
                    ty,
                )?;
                let captures = self.store_operands(&capture_operands, expression.origin)?;
                self.assign_temporary(
                    ty,
                    Rvalue::MakeClosure { function, captures },
                    expression.origin,
                )
            }
        }
    }

    fn lower_try(
        &mut self,
        expression: ExprId,
        inner: ExprId,
        depth: usize,
        origin: OriginId,
    ) -> Result<Operand, MirBuildError> {
        let fact = self
            .core
            .types
            .enum_flow
            .try_expression(expression)
            .ok_or_else(|| {
                self.core
                    .error(origin, MirBuildErrorKind::MissingEnumTryFact { expression })
            })?;
        let carrier_type = self.concrete_type(fact.carrier_type, origin)?;
        let success_type = self.concrete_type(fact.success_type, origin)?;
        let return_type = self.concrete_type(fact.return_type, origin)?;
        if return_type != self.return_type {
            return Err(self
                .core
                .error(origin, MirBuildErrorKind::MissingEnumTryFact { expression }));
        }
        let carrier_aggregate = self.core.ensure_aggregate(carrier_type, origin)?;
        let return_aggregate = self.core.ensure_aggregate(return_type, origin)?;
        if self
            .core
            .program
            .aggregate(carrier_aggregate)
            .is_none_or(|aggregate| aggregate.source != fact.item)
            || self
                .core
                .program
                .aggregate(return_aggregate)
                .is_none_or(|aggregate| aggregate.source != fact.return_item)
        {
            return Err(self
                .core
                .error(origin, MirBuildErrorKind::MissingEnumTryFact { expression }));
        }

        let operand = self.lower_expression(inner, depth)?;
        let carrier = self.temporary(carrier_type, origin)?;
        self.emit(carrier, Rvalue::Use(operand), origin)?;
        let result = self.temporary(success_type, origin)?;
        let success_block = self.new_block(origin)?;
        let join_block = self.new_block(origin)?;
        let source_variants = self.core.package.variants(fact.variants).to_vec();
        self.reserve_switch_targets(source_variants.len(), origin)?;
        let mut targets = Vec::with_capacity(source_variants.len());
        let mut residual_blocks = Vec::new();
        let mut direct_residual_block = None;

        for variant in source_variants.iter().copied() {
            if variant == fact.success {
                targets.push(success_block);
            } else if fact.direct_residual {
                let block = match direct_residual_block {
                    Some(block) => block,
                    None => {
                        let block = self.new_block(origin)?;
                        direct_residual_block = Some(block);
                        block
                    }
                };
                targets.push(block);
            } else {
                let block = self.new_block(self.core.package.variants[variant].origin)?;
                targets.push(block);
                residual_blocks.push((variant, block));
            }
        }
        self.terminate(DraftTerminator::SwitchEnum {
            subject: Operand::Copy(carrier),
            aggregate: carrier_aggregate,
            targets,
        });

        self.current = Some(success_block);
        let success_variant = *self
            .core
            .aggregate_variants
            .get(&(carrier_aggregate, fact.success))
            .ok_or_else(|| {
                self.core
                    .error(origin, MirBuildErrorKind::MissingEnumTryFact { expression })
            })?;
        let success_field = *self
            .core
            .aggregate_fields
            .get(&(carrier_aggregate, fact.success_field))
            .ok_or_else(|| {
                self.core
                    .error(origin, MirBuildErrorKind::MissingEnumTryFact { expression })
            })?;
        let place = self.place(
            carrier,
            &[
                PlaceProjection::Downcast(success_variant),
                PlaceProjection::Field(success_field),
            ],
            success_type,
            origin,
        )?;
        self.emit(result, Rvalue::Load(place), origin)?;
        self.terminate(DraftTerminator::Goto(join_block));

        if let Some(block) = direct_residual_block {
            self.current = Some(block);
            self.terminate(DraftTerminator::Return(Some(Operand::Copy(carrier))));
        }

        for (source_variant, block) in residual_blocks {
            self.current = Some(block);
            let input_variant = self.core.aggregate_variants[&(carrier_aggregate, source_variant)];
            let output_variant = *self
                .core
                .aggregate_variants
                .get(&(return_aggregate, source_variant))
                .ok_or_else(|| {
                    self.core
                        .error(origin, MirBuildErrorKind::MissingEnumTryFact { expression })
                })?;
            let source_fields = self
                .core
                .package
                .fields(self.core.package.variants[source_variant].fields)
                .to_vec();
            let mut values = Vec::with_capacity(source_fields.len());
            for source_field in source_fields {
                let input_field = self.core.aggregate_fields[&(carrier_aggregate, source_field)];
                let field_type = self
                    .core
                    .program
                    .field(input_field)
                    .expect("mapped residual input field exists")
                    .ty;
                let place = self.place(
                    carrier,
                    &[
                        PlaceProjection::Downcast(input_variant),
                        PlaceProjection::Field(input_field),
                    ],
                    field_type,
                    origin,
                )?;
                values.push(self.assign_temporary(field_type, Rvalue::Load(place), origin)?);
            }
            let values = self.store_operands(&values, origin)?;
            let residual = self.assign_temporary(
                return_type,
                Rvalue::ConstructVariant {
                    aggregate: return_aggregate,
                    variant: output_variant,
                    fields: values,
                },
                origin,
            )?;
            self.terminate(DraftTerminator::Return(Some(residual)));
        }

        self.current = Some(join_block);
        Ok(Operand::Copy(result))
    }

    fn lower_call(
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

        let callee_expression = self.core.package.expressions[callee];
        let callee = match (explicit_arguments, callee_expression.kind) {
            (
                Some(type_arguments),
                ExpressionKind::Name {
                    binding: NameBinding::Item(lpp_hir::BindingTarget::Definition(definition)),
                    ..
                },
            ) => Operand::Function(self.lower_function_name(
                callee,
                definition,
                Some(type_arguments),
            )?),
            _ => self.lower_expression(callee, depth)?,
        };
        let arguments = self.lower_arguments(arguments, origin, depth)?;
        self.assign_temporary(result_type, Rvalue::Call { callee, arguments }, origin)
    }

    /// Lower a UFCS method call `receiver.method(args..)` resolved by the type
    /// checker into a direct call `method(receiver, args..)`.
    fn lower_method_call(
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

    fn lower_method_function(
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
    fn resolve_concrete_method(
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

    fn method_receiver_matches(
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
    fn variadic_fixed_arity(&self, callee: ExprId) -> Option<usize> {
        let ExpressionKind::Name {
            binding: NameBinding::Item(lpp_hir::BindingTarget::Definition(definition)),
            ..
        } = self.core.package.expressions[callee].kind
        else {
            return None;
        };
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
    fn default_param_locals(&self, callee: ExprId) -> Option<Vec<LocalId>> {
        let ExpressionKind::Name {
            binding: NameBinding::Item(lpp_hir::BindingTarget::Definition(definition)),
            ..
        } = self.core.package.expressions[callee].kind
        else {
            return None;
        };
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
    fn lower_defaulted_call(
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
    fn lower_variadic_call(
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

    fn lower_arguments(
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

    /// Synthesize the zero value for every field of a struct aggregate — the
    /// MIR side of v1's zero-argument constructor (`Box()` zero-initializes
    /// each field, verified against the v1 oracle: Int->0, Float->0.0,
    /// Bool->false, Str->""). Primitive fields get their zero constant; a
    /// non-primitive field (nested struct, list, map, …) is a structured
    /// `Unsupported` failure for now, keeping this slice bounded.
    fn zero_init_struct_fields(
        &mut self,
        aggregate: MirAggregateId,
        origin: OriginId,
    ) -> Result<ListRange<Operand>, MirBuildError> {
        // Phase 1: read the concrete field types (immutable borrows only).
        let field_types: Vec<TypeId> = {
            let descriptor = self
                .core
                .program
                .aggregate(aggregate)
                .expect("the constructor's aggregate descriptor was just ensured");
            let fields = self.core.program.aggregate_fields(descriptor);
            let mut types = Vec::with_capacity(fields.len());
            for &field in fields {
                let field = self
                    .core
                    .program
                    .field(field)
                    .expect("aggregate fields reference live field descriptors");
                types.push(field.ty);
            }
            types
        };
        // Phase 2: build one zero value per field (recursively, so a nested
        // struct / list / tuple field gets a real default rather than a
        // rejection).
        let mut zeros = Vec::with_capacity(field_types.len());
        for ty in field_types {
            zeros.push(self.zero_value(ty, origin)?);
        }
        self.store_operands(&zeros, origin)
    }

    /// The default ("zero") value for a type, matching v1's zero-argument
    /// constructor: Int→0, Float→0.0, Bool→false, Char→'\0', Str→"",
    /// List→empty, a nested struct→its own zero-initialized value, and a
    /// tuple→a tuple of its elements' zero values. Non-defaultable types
    /// (enums, maps, …) remain a structured `Unsupported(Field)` failure.
    fn zero_value(&mut self, ty: TypeId, origin: OriginId) -> Result<Operand, MirBuildError> {
        let ty = self.concrete_type(ty, origin)?;
        match self.core.types.interner.kind(ty) {
            TypeKind::Primitive(PrimitiveType::Int) => Ok(Operand::Constant(Constant::Integer(0))),
            TypeKind::Primitive(PrimitiveType::Float) => {
                Ok(Operand::Constant(Constant::FloatBits(0)))
            }
            TypeKind::Primitive(PrimitiveType::Bool) => {
                Ok(Operand::Constant(Constant::Bool(false)))
            }
            TypeKind::Primitive(PrimitiveType::Char) => {
                Ok(Operand::Constant(Constant::Character {
                    origin,
                    character: '\0',
                }))
            }
            TypeKind::Primitive(PrimitiveType::String) => {
                let string = self.core.intern_string(String::new(), origin)?;
                Ok(Operand::Constant(Constant::String { origin, string }))
            }
            TypeKind::List(_) => {
                let operands = self.store_operands(&[], origin)?;
                self.assign_temporary(ty, Rvalue::List(operands), origin)
            }
            TypeKind::Nominal { .. } => {
                let aggregate = self.core.ensure_aggregate(ty, origin)?;
                let kind = self
                    .core
                    .program
                    .aggregate(aggregate)
                    .expect("ensure_aggregate yields a live aggregate")
                    .kind;
                if kind != MirAggregateKind::Struct {
                    return Err(self.core.error(
                        origin,
                        MirBuildErrorKind::Unsupported(UnsupportedConstruct::Field),
                    ));
                }
                let fields = self.zero_init_struct_fields(aggregate, origin)?;
                self.assign_temporary(ty, Rvalue::ConstructStruct { aggregate, fields }, origin)
            }
            TypeKind::Tuple(elements) => {
                let element_types = self.core.types.interner.list(elements).to_vec();
                let mut operands = Vec::with_capacity(element_types.len());
                for element in element_types {
                    operands.push(self.zero_value(element, origin)?);
                }
                let operands = self.store_operands(&operands, origin)?;
                self.assign_temporary(ty, Rvalue::Tuple(operands), origin)
            }
            _ => Err(self.core.error(
                origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::Field),
            )),
        }
    }

    pub(super) fn lower_place(
        &mut self,
        expression: ExprId,
        depth: usize,
    ) -> Result<MirPlaceId, MirBuildError> {
        let origin = self.core.package.expressions[expression].origin;
        let (root, projections) = self.lower_place_parts(expression, depth)?;
        let ty = self
            .core
            .types
            .assignments
            .expression(expression)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(expression)),
                )
            })?;
        let ty = self.concrete_type(ty, origin)?;
        self.place(root, &projections, ty, origin)
    }

    pub(super) fn lower_store_place(
        &mut self,
        expression: ExprId,
        depth: usize,
    ) -> Result<MirPlaceId, MirBuildError> {
        let node = self.core.package.expressions[expression];
        let (root, mut projections) = self.lower_place_parts(expression, depth)?;
        let terminal_type = self
            .core
            .types
            .assignments
            .expression(expression)
            .ok_or_else(|| {
                self.core.error(
                    node.origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(expression)),
                )
            })?;
        let terminal_type = self.concrete_type(terminal_type, node.origin)?;
        if projections.len() <= 1 {
            return self.place(root, &projections, terminal_type, node.origin);
        }

        let base = match node.kind {
            ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => base,
            _ => unreachable!("multi-projection places end in a projection expression"),
        };
        let base_node = self.core.package.expressions[base];
        let base_type = self
            .core
            .types
            .assignments
            .expression(base)
            .ok_or_else(|| {
                self.core.error(
                    base_node.origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(base)),
                )
            })?;
        let base_type = self.concrete_type(base_type, base_node.origin)?;
        let final_projection = projections
            .pop()
            .expect("checked non-empty projection path");
        let prefix = self.place(root, &projections, base_type, base_node.origin)?;
        let materialized = self.temporary_with_mutability(base_type, true, base_node.origin)?;
        self.emit(materialized, Rvalue::Load(prefix), base_node.origin)?;
        self.place(
            materialized,
            &[final_projection],
            terminal_type,
            node.origin,
        )
    }

    fn lower_place_parts(
        &mut self,
        expression: ExprId,
        depth: usize,
    ) -> Result<(MirLocalId, Vec<PlaceProjection>), MirBuildError> {
        let node = self.core.package.expressions[expression];
        if depth >= self.core.options.max_expression_depth {
            return Err(self.core.error(
                node.origin,
                MirBuildErrorKind::Capacity(MirCapacity::ExpressionDepth),
            ));
        }
        match node.kind {
            ExpressionKind::Name {
                binding: NameBinding::Local(source),
                ..
            } => {
                let local = self.local(source, MirLocalKind::User)?;
                // By-reference capture cell: a store to (or load from)
                // the local targets the head element of its list.
                let projections = if self.core.cell_sources.contains(&source) {
                    vec![PlaceProjection::ListIndex(Operand::Constant(
                        Constant::Integer(0),
                    ))]
                } else {
                    Vec::new()
                };
                Ok((local, projections))
            }
            ExpressionKind::Field { base, .. } => {
                let (root, mut projections) = self.lower_place_parts(base, depth + 1)?;
                projections.push(PlaceProjection::Field(self.resolve_field_projection(
                    expression,
                    base,
                    node.origin,
                )?));
                Ok((root, projections))
            }
            ExpressionKind::Index { base, index } => {
                let (root, mut projections) = self.lower_place_parts(base, depth + 1)?;
                let fact = self
                    .core
                    .types
                    .places
                    .expression(expression)
                    .ok_or_else(|| {
                        self.core.error(
                            node.origin,
                            MirBuildErrorKind::MissingPlaceFact { expression },
                        )
                    })?;
                match fact {
                    lpp_types::PlaceExpressionFact::TupleField { index } => {
                        projections.push(PlaceProjection::TupleField(index));
                    }
                    lpp_types::PlaceExpressionFact::ListIndex => {
                        let index = self.lower_expression(index, depth + 1)?;
                        let int = self.core.types.interner.primitive(PrimitiveType::Int);
                        let index = self.assign_temporary(
                            int,
                            Rvalue::Use(index),
                            self.core.package.expressions[expression].origin,
                        )?;
                        projections.push(PlaceProjection::ListIndex(index));
                    }
                }
                Ok((root, projections))
            }
            _ => {
                let ty = self
                    .core
                    .types
                    .assignments
                    .expression(expression)
                    .ok_or_else(|| {
                        self.core.error(
                            node.origin,
                            MirBuildErrorKind::MissingType(TypedEntity::Expression(expression)),
                        )
                    })?;
                let ty = self.concrete_type(ty, node.origin)?;
                let value = self.lower_expression(expression, depth + 1)?;
                let root = match value {
                    Operand::Copy(local) => local,
                    value => {
                        let Operand::Copy(local) =
                            self.assign_temporary(ty, Rvalue::Use(value), node.origin)?
                        else {
                            unreachable!("temporary assignment returns a local operand")
                        };
                        local
                    }
                };
                Ok((root, Vec::new()))
            }
        }
    }

    fn resolve_field_projection(
        &mut self,
        expression: ExprId,
        base: ExprId,
        origin: OriginId,
    ) -> Result<MirFieldId, MirBuildError> {
        let base_type = self
            .core
            .types
            .assignments
            .expression(base)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(base)),
                )
            })?;
        let base_type = self.concrete_type(base_type, origin)?;
        let (item, field) = match self.core.types.aggregates.expression(expression) {
            Some(AggregateExpressionFact::FieldProjection { item, field }) => (item, field),
            _ => {
                let ExpressionKind::Field { name, .. } =
                    self.core.package.expressions[expression].kind
                else {
                    return Err(self.core.error(
                        origin,
                        MirBuildErrorKind::MissingAggregateFact { expression },
                    ));
                };
                let TypeKind::Nominal { definition, .. } = self.core.types.interner.kind(base_type)
                else {
                    return Err(self.core.error(
                        origin,
                        MirBuildErrorKind::MissingAggregateFact { expression },
                    ));
                };
                let item = self.core.types.aggregates.item(definition).ok_or_else(|| {
                    self.core.error(
                        origin,
                        MirBuildErrorKind::MissingAggregateFact { expression },
                    )
                })?;
                let field = self
                    .core
                    .types
                    .aggregates
                    .field(item, name)
                    .ok_or_else(|| {
                        self.core.error(
                            origin,
                            MirBuildErrorKind::MissingAggregateFact { expression },
                        )
                    })?;
                (item, field)
            }
        };
        let aggregate = self.core.ensure_aggregate(base_type, origin)?;
        if self.core.program.aggregate(aggregate).unwrap().source != item {
            return Err(self
                .core
                .error(origin, MirBuildErrorKind::InvalidAggregateType(base_type)));
        }
        self.core
            .aggregate_fields
            .get(&(aggregate, field))
            .copied()
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingAggregateFact { expression },
                )
            })
    }

    /// If `definition` names a `const`, returns its value expression so a
    /// reference can be inlined at the use site.
    fn const_value(&self, definition: lpp_hir::DefId) -> Option<ExprId> {
        let item = self
            .core
            .definition_items
            .get(definition.index())
            .and_then(|item| *item)?;
        match self.core.package.items[item].kind {
            HirItemKind::Const { value } => Some(value),
            _ => None,
        }
    }

    fn lower_function_name(
        &mut self,
        expression: ExprId,
        definition: lpp_hir::DefId,
        explicit_arguments: Option<IdRange<TypeRefId>>,
    ) -> Result<MirFunctionId, MirBuildError> {
        let origin = self.core.package.expressions[expression].origin;
        let Some(item_id) = self
            .core
            .definition_items
            .get(definition.index())
            .and_then(|item| *item)
        else {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
            ));
        };
        let HirItemKind::Function(function) = self.core.package.items[item_id].kind else {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
            ));
        };
        if function.type_parameters.is_empty() {
            return self
                .core
                .definition_functions
                .get(definition.index())
                .and_then(|function| *function)
                .ok_or_else(|| {
                    self.core.error(
                        origin,
                        MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
                    )
                });
        }

        if let Some(type_arguments) = explicit_arguments {
            let type_arguments = self.core.package.type_refs(type_arguments).to_vec();
            let mut concrete_arguments = Vec::with_capacity(type_arguments.len());
            for type_reference in type_arguments {
                let ty = self
                    .core
                    .types
                    .assignments
                    .type_ref(type_reference)
                    .ok_or_else(|| {
                        self.core
                            .error(origin, MirBuildErrorKind::GenericTypeMaterialization)
                    })?;
                concrete_arguments.push(self.concrete_type(ty, origin)?);
            }
            let arguments = self
                .core
                .types
                .interner
                .intern_list(&concrete_arguments)
                .map_err(|_| {
                    self.core
                        .error(origin, MirBuildErrorKind::GenericTypeMaterialization)
                })?;
            let instance = self
                .core
                .types
                .instances
                .instance(InstanceKey::new(item_id, arguments))
                .ok_or_else(|| {
                    self.core.error(
                        origin,
                        MirBuildErrorKind::MissingExplicitInstance { item: item_id },
                    )
                })?;
            return self.core.instance_functions[instance.index()].ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingExplicitInstance { item: item_id },
                )
            });
        }

        let ty = self
            .core
            .types
            .assignments
            .expression(expression)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(expression)),
                )
            })?;
        let ty = self.concrete_type(ty, origin)?;
        self.core
            .instance_type_functions
            .get(&(item_id, ty))
            .and_then(|function| *function)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingFunctionInstance { item: item_id, ty },
                )
            })
    }
}
