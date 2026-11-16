use super::*;

impl FunctionBuilder<'_, '_> {
    pub(in crate::builder) fn lower_expression(
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
                    Literal::RawString(span) => {
                        let string = self.core.materialize_raw_string(span, expression.origin)?;
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
                    } else if self
                        .core
                        .definition_items
                        .get(definition.index())
                        .and_then(|item| *item)
                        .is_some_and(|item| self.core.is_external_function(item))
                    {
                        Err(self.core.error(
                            expression.origin,
                            MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
                        ))
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
            ExpressionKind::Cast { value, target: _ } => {
                let lowered = self.lower_expression(value, next_depth)?;
                if matches!(
                    self.core.types.interner.kind(ty),
                    TypeKind::Ptr(_) | TypeKind::Primitive(PrimitiveType::RawPtr)
                ) {
                    self.core.program.unsafe_sites.push(UnsafeSite {
                        origin: expression.origin,
                        kind: UnsafeSiteKind::UnsafeCast,
                    });
                }
                self.assign_temporary(
                    ty,
                    Rvalue::Cast {
                        value: lowered,
                        target: ty,
                    },
                    expression.origin,
                )
            }
            ExpressionKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => self.lower_conditional(
                ty,
                condition,
                then_expr,
                else_expr,
                next_depth,
                expression.origin,
            ),
            ExpressionKind::Unary { operator, operand } => {
                let operand = self.lower_expression(operand, next_depth)?;
                self.assign_temporary(ty, Rvalue::Unary { operator, operand }, expression.origin)
            }
            ExpressionKind::Binary {
                left,
                operator,
                right,
            } => {
                if matches!(
                    operator,
                    lpp_hir::BinaryOperator::LogicalAnd | lpp_hir::BinaryOperator::LogicalOr
                ) {
                    self.lower_short_circuit(
                        ty,
                        left,
                        operator,
                        right,
                        next_depth,
                        expression.origin,
                    )
                } else {
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
                if let Some(definition) = self.core.package.expression_definition(expression_id) {
                    if let Some(value) = self.const_value(definition) {
                        self.lower_expression(value, next_depth)
                    } else {
                        self.lower_function_name(expression_id, definition, None)
                            .map(Operand::Function)
                    }
                } else if let Some(AggregateExpressionFact::UnitVariant { item, variant }) =
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

    pub(in crate::builder) fn lower_short_circuit(
        &mut self,
        ty: TypeId,
        left_expression: ExprId,
        operator: lpp_hir::BinaryOperator,
        right_expression: ExprId,
        depth: usize,
        origin: OriginId,
    ) -> Result<Operand, MirBuildError> {
        let left = self.lower_expression(left_expression, depth)?;
        let rhs_block = self.new_block(origin)?;
        let short_block = self.new_block(origin)?;
        let join_block = self.new_block(origin)?;
        let result = self.temporary(ty, origin)?;
        let (then_block, else_block, short_value) = match operator {
            lpp_hir::BinaryOperator::LogicalAnd => (rhs_block, short_block, false),
            lpp_hir::BinaryOperator::LogicalOr => (short_block, rhs_block, true),
            _ => unreachable!("only logical operators use short-circuit lowering"),
        };
        self.terminate(DraftTerminator::Branch {
            condition: left,
            then_block,
            else_block,
        });

        self.current = Some(short_block);
        self.emit(
            result,
            Rvalue::Use(Operand::Constant(Constant::Bool(short_value))),
            origin,
        )?;
        self.terminate(DraftTerminator::Goto(join_block));

        self.current = Some(rhs_block);
        let right = self.lower_expression(right_expression, depth)?;
        self.emit(result, Rvalue::Use(right), origin)?;
        self.terminate(DraftTerminator::Goto(join_block));

        self.current = Some(join_block);
        Ok(Operand::Copy(result))
    }

    pub(in crate::builder) fn lower_conditional(
        &mut self,
        ty: TypeId,
        condition: ExprId,
        then_expr: ExprId,
        else_expr: ExprId,
        depth: usize,
        origin: OriginId,
    ) -> Result<Operand, MirBuildError> {
        let cond = self.lower_expression(condition, depth)?;
        let then_block = self.new_block(origin)?;
        let else_block = self.new_block(origin)?;
        let join_block = self.new_block(origin)?;
        let result = self.temporary(ty, origin)?;
        self.terminate(DraftTerminator::Branch {
            condition: cond,
            then_block,
            else_block,
        });

        self.current = Some(then_block);
        let then_val = self.lower_expression(then_expr, depth)?;
        if self.current.is_some() {
            self.emit(result, Rvalue::Use(then_val), origin)?;
            self.terminate(DraftTerminator::Goto(join_block));
        }

        self.current = Some(else_block);
        let else_val = self.lower_expression(else_expr, depth)?;
        if self.current.is_some() {
            self.emit(result, Rvalue::Use(else_val), origin)?;
            self.terminate(DraftTerminator::Goto(join_block));
        }

        self.current = Some(join_block);
        Ok(Operand::Copy(result))
    }

    pub(in crate::builder) fn lower_try(
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
}
