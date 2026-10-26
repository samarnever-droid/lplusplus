use super::*;

impl FunctionBuilder<'_, '_> {
    pub(in crate::builder) fn lower_place(
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

    pub(in crate::builder) fn lower_store_place(
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

    pub(in crate::builder) fn lower_place_parts(
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

    pub(in crate::builder) fn resolve_field_projection(
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
}
