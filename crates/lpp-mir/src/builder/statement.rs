use super::*;

impl FunctionBuilder<'_, '_> {
    pub(super) fn lower_statement(&mut self, statement_id: StmtId) -> Result<(), MirBuildError> {
        let statement = self.core.package.statements[statement_id];
        match statement.kind {
            StatementKind::Let { bindings, value } => {
                let bindings = self.core.package.locals(bindings).to_vec();
                let value_expression = value;
                let value = self.lower_expression(value_expression, 0)?;
                if bindings.len() == 1 {
                    let target = self.local(bindings[0], MirLocalKind::User)?;
                    if self.core.cell_sources.contains(&bindings[0]) {
                        // By-reference capture cell: the initializer
                        // becomes the head element of the new cell.
                        let operands = self.store_operands(&[value], statement.origin)?;
                        self.emit(target, Rvalue::List(operands), statement.origin)?;
                    } else {
                        self.emit(target, Rvalue::Use(value), statement.origin)?;
                    }
                } else {
                    let Some(lpp_types::PlaceStatementFact::TupleDestructure { arity }) =
                        self.core.types.places.statement(statement_id)
                    else {
                        return Err(self.core.error(
                            statement.origin,
                            MirBuildErrorKind::MissingStatementPlaceFact {
                                statement: statement_id,
                            },
                        ));
                    };
                    if arity as usize != bindings.len() {
                        return Err(self.core.error(
                            statement.origin,
                            MirBuildErrorKind::MissingStatementPlaceFact {
                                statement: statement_id,
                            },
                        ));
                    }
                    let value_type = self
                        .core
                        .types
                        .assignments
                        .expression(value_expression)
                        .ok_or_else(|| {
                            self.core.error(
                                statement.origin,
                                MirBuildErrorKind::MissingType(TypedEntity::Expression(
                                    value_expression,
                                )),
                            )
                        })?;
                    let value_type = self.concrete_type(value_type, statement.origin)?;
                    let root = match value {
                        Operand::Copy(local) => local,
                        value => {
                            let Operand::Copy(local) = self.assign_temporary(
                                value_type,
                                Rvalue::Use(value),
                                statement.origin,
                            )?
                            else {
                                unreachable!("temporary assignment returns a local operand")
                            };
                            local
                        }
                    };
                    for (index, binding) in bindings.into_iter().enumerate() {
                        let target = self.local(binding, MirLocalKind::User)?;
                        let ty = self.core.program.local(target).unwrap().ty;
                        let place = self.place(
                            root,
                            &[PlaceProjection::TupleField(index as u32)],
                            ty,
                            statement.origin,
                        )?;
                        self.emit(target, Rvalue::Load(place), statement.origin)?;
                    }
                }
            }
            StatementKind::Assign { target, value } => {
                let Some(lpp_types::PlaceStatementFact::Assignment { augmented }) =
                    self.core.types.places.statement(statement_id)
                else {
                    return Err(self.core.error(
                        statement.origin,
                        MirBuildErrorKind::MissingStatementPlaceFact {
                            statement: statement_id,
                        },
                    ));
                };
                let place = self.lower_store_place(target, 0)?;
                let value = if let Some(augmented) = augmented {
                    let target_type = self
                        .core
                        .program
                        .place(place)
                        .expect("newly lowered place exists")
                        .ty;
                    let left =
                        self.assign_temporary(target_type, Rvalue::Load(place), statement.origin)?;
                    let right = self.lower_expression(augmented.right, 0)?;
                    self.assign_temporary(
                        target_type,
                        Rvalue::Binary {
                            left,
                            operator: augmented.operator,
                            right,
                        },
                        statement.origin,
                    )?
                } else {
                    self.lower_expression(value, 0)?
                };
                self.emit_store(place, value, statement.origin)?;
            }
            StatementKind::Expression(expression) => {
                self.lower_expression(expression, 0)?;
            }
            StatementKind::Return(value) => {
                let value = value
                    .map(|expression| self.lower_expression(expression, 0))
                    .transpose()?;
                self.terminate(DraftTerminator::Return(value));
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => self.lower_if(condition, then_body, else_body, statement.origin)?,
            StatementKind::While { condition, body } => {
                self.lower_while(condition, body, statement.origin)?;
            }
            StatementKind::Break => {
                let Some((break_block, _)) = self.loops.last().copied() else {
                    return Err(self
                        .core
                        .error(statement.origin, MirBuildErrorKind::InvalidAssignmentTarget));
                };
                self.terminate(DraftTerminator::Goto(break_block));
            }
            StatementKind::Continue => {
                let Some((_, continue_block)) = self.loops.last().copied() else {
                    return Err(self
                        .core
                        .error(statement.origin, MirBuildErrorKind::InvalidAssignmentTarget));
                };
                self.terminate(DraftTerminator::Goto(continue_block));
            }
            StatementKind::For {
                binding,
                iterable,
                body,
            } => self.lower_for(statement_id, binding, iterable, body, statement.origin)?,
            StatementKind::Match { subject, arms } => {
                self.lower_match(statement_id, subject, arms, statement.origin)?;
            }
        }
        Ok(())
    }

    fn lower_match(
        &mut self,
        statement: StmtId,
        subject_expression: ExprId,
        arms: IdRange<MatchArmId>,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        let fact = self
            .core
            .types
            .enum_flow
            .match_statement(statement)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingEnumMatchFact { statement },
                )
            })?;
        if fact.arms != arms {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::MissingEnumMatchFact { statement },
            ));
        }
        let subject_type = self.concrete_type(fact.subject_type, origin)?;
        let aggregate = self.core.ensure_aggregate(subject_type, origin)?;
        if self
            .core
            .program
            .aggregate(aggregate)
            .is_none_or(|descriptor| descriptor.source != fact.item)
        {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::InvalidAggregateType(subject_type),
            ));
        }
        let subject = self.lower_expression(subject_expression, 0)?;
        let subject_local = self.temporary(subject_type, origin)?;
        self.emit(subject_local, Rvalue::Use(subject), origin)?;

        let join = self.new_block(origin)?;
        let variant_ids = self
            .core
            .program
            .aggregate(aggregate)
            .map(|descriptor| self.core.program.aggregate_variants(descriptor).to_vec())
            .expect("the aggregate descriptor was ensured");
        if variant_ids.len() != fact.total_variants as usize {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::InvalidAggregateType(subject_type),
            ));
        }
        self.reserve_switch_targets(variant_ids.len(), origin)?;
        let mut targets = vec![join; variant_ids.len()];
        let mut lowered_arms = Vec::new();

        for arm_id in self.core.package.match_arms(arms).iter().copied() {
            let arm_fact = self.core.types.enum_flow.match_arm(arm_id).ok_or_else(|| {
                self.core.error(
                    self.core.package.match_arms[arm_id].origin,
                    MirBuildErrorKind::MissingEnumArmFact { arm: arm_id },
                )
            })?;
            let body = self.core.package.match_arms[arm_id].body;
            let block = self.new_block(self.core.package.bodies[body].origin)?;
            match arm_fact {
                lpp_types::EnumMatchArmFact::Variant {
                    item,
                    variant,
                    reachable,
                    ..
                } => {
                    if item != fact.item {
                        return Err(self.core.error(
                            self.core.package.match_arms[arm_id].origin,
                            MirBuildErrorKind::MissingEnumArmFact { arm: arm_id },
                        ));
                    }
                    let mir_variant = *self
                        .core
                        .aggregate_variants
                        .get(&(aggregate, variant))
                        .ok_or_else(|| {
                            self.core.error(
                                self.core.package.match_arms[arm_id].origin,
                                MirBuildErrorKind::MissingEnumArmFact { arm: arm_id },
                            )
                        })?;
                    let ordinal = self
                        .core
                        .program
                        .variant(mir_variant)
                        .expect("mapped variant descriptor exists")
                        .ordinal as usize;
                    if reachable {
                        targets[ordinal] = block;
                    }
                }
                lpp_types::EnumMatchArmFact::Wildcard { reachable } => {
                    if reachable {
                        for target in &mut targets {
                            if *target == join {
                                *target = block;
                            }
                        }
                    }
                }
            }
            lowered_arms.push((arm_id, arm_fact, block));
        }

        self.terminate(DraftTerminator::SwitchEnum {
            subject: Operand::Copy(subject_local),
            aggregate,
            targets: targets.clone(),
        });

        // The join block is reachable only if some variant falls through to it
        // directly (a `SwitchEnum` target still points at `join`) or an arm body
        // completes without diverging. Otherwise every path returns/diverges and
        // leaving `join` current would be a spurious missing-return.
        let mut join_reachable = targets.iter().any(|target| *target == join);

        for (arm_id, arm_fact, block) in lowered_arms {
            let arm = self.core.package.match_arms[arm_id];
            self.current = Some(block);
            if let lpp_types::EnumMatchArmFact::Variant {
                variant,
                fields,
                bindings,
                ..
            } = arm_fact
            {
                let mir_variant = self.core.aggregate_variants[&(aggregate, variant)];
                let fields = self.core.package.fields(fields).to_vec();
                let bindings = self.core.package.locals(bindings).to_vec();
                if fields.len() != bindings.len() {
                    return Err(self.core.error(
                        arm.origin,
                        MirBuildErrorKind::MissingEnumArmFact { arm: arm_id },
                    ));
                }
                for (field, binding) in fields.into_iter().zip(bindings) {
                    let mir_field = *self
                        .core
                        .aggregate_fields
                        .get(&(aggregate, field))
                        .ok_or_else(|| {
                            self.core.error(
                                arm.origin,
                                MirBuildErrorKind::MissingEnumArmFact { arm: arm_id },
                            )
                        })?;
                    let target = self.local(binding, MirLocalKind::User)?;
                    let ty = self
                        .core
                        .program
                        .local(target)
                        .expect("payload binding local exists")
                        .ty;
                    let place = self.place(
                        subject_local,
                        &[
                            PlaceProjection::Downcast(mir_variant),
                            PlaceProjection::Field(mir_field),
                        ],
                        ty,
                        arm.origin,
                    )?;
                    self.emit(target, Rvalue::Load(place), arm.origin)?;
                }
            }
            self.lower_body(arm.body)?;
            if self.current.is_some() {
                join_reachable = true;
                self.terminate(DraftTerminator::Goto(join));
            }
        }

        self.current = if join_reachable { Some(join) } else { None };
        Ok(())
    }

    /// Recognise `for VAR in range(..)` — `range` is not a real binding, so it
    /// is desugared into a counting loop rather than list iteration. Returns the
    /// argument expressions (1..=3) when `iterable` is such a call.
    fn range_call_arguments(&self, iterable: ExprId) -> Option<Vec<ExprId>> {
        let ExpressionKind::Call { callee, arguments } =
            self.core.package.expressions[iterable].kind
        else {
            return None;
        };
        let ExpressionKind::Name {
            symbol,
            binding: NameBinding::Unresolved,
        } = self.core.package.expressions[callee].kind
        else {
            return None;
        };
        if self.core.package.names.symbols.resolve(symbol) != Some("range") {
            return None;
        }
        Some(self.core.package.expressions(arguments).to_vec())
    }

    /// Lower `for VAR in range(start, end, step)` as an integer counting loop.
    /// `range(end)` starts at 0 with step 1; `range(start, end)` uses step 1.
    fn lower_for_range(
        &mut self,
        binding: LocalId,
        arguments: &[ExprId],
        body: BodyId,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        let int = self.core.types.interner.primitive(PrimitiveType::Int);
        let bool_type = self.core.types.interner.primitive(PrimitiveType::Bool);

        let zero = || Operand::Constant(Constant::Integer(0));
        let one = || Operand::Constant(Constant::Integer(1));
        let (start, end, step) = match arguments {
            [end] => (zero(), self.lower_expression(*end, 0)?, one()),
            [start, end] => (
                self.lower_expression(*start, 0)?,
                self.lower_expression(*end, 0)?,
                one(),
            ),
            [start, end, step] => (
                self.lower_expression(*start, 0)?,
                self.lower_expression(*end, 0)?,
                self.lower_expression(*step, 0)?,
            ),
            _ => {
                return Err(self.core.error(
                    origin,
                    MirBuildErrorKind::Unsupported(UnsupportedConstruct::UnresolvedName),
                ));
            }
        };

        // The loop variable doubles as the counter. `end` and `step` are
        // snapshotted into temporaries so the bounds are evaluated once.
        let counter = self.local(binding, MirLocalKind::User)?;
        self.emit(counter, Rvalue::Use(start), origin)?;
        let end_local = self.temporary(int, origin)?;
        self.emit(end_local, Rvalue::Use(end), origin)?;
        let step_local = self.temporary(int, origin)?;
        self.emit(step_local, Rvalue::Use(step), origin)?;

        let condition_block = self.new_block(origin)?;
        let body_block = self.new_block(self.core.package.bodies[body].origin)?;
        let step_block = self.new_block(origin)?;
        let exit_block = self.new_block(origin)?;
        self.terminate(DraftTerminator::Goto(condition_block));

        self.current = Some(condition_block);
        let condition = self.assign_temporary(
            bool_type,
            Rvalue::Binary {
                left: Operand::Copy(counter),
                operator: lpp_hir::BinaryOperator::Less,
                right: Operand::Copy(end_local),
            },
            origin,
        )?;
        self.terminate(DraftTerminator::Branch {
            condition,
            then_block: body_block,
            else_block: exit_block,
        });

        self.current = Some(body_block);
        self.loops.push((exit_block, step_block));
        self.lower_body(body)?;
        self.loops.pop();
        if self.current.is_some() {
            self.terminate(DraftTerminator::Goto(step_block));
        }

        self.current = Some(step_block);
        let next = self.assign_temporary(
            int,
            Rvalue::Binary {
                left: Operand::Copy(counter),
                operator: lpp_hir::BinaryOperator::Add,
                right: Operand::Copy(step_local),
            },
            origin,
        )?;
        self.emit(counter, Rvalue::Use(next), origin)?;
        self.terminate(DraftTerminator::Goto(condition_block));

        self.current = Some(exit_block);
        Ok(())
    }

    fn lower_for(
        &mut self,
        statement: StmtId,
        binding: LocalId,
        iterable: ExprId,
        body: BodyId,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        if let Some(arguments) = self.range_call_arguments(iterable) {
            return self.lower_for_range(binding, &arguments, body, origin);
        }
        if self.core.types.places.statement(statement)
            != Some(lpp_types::PlaceStatementFact::ListIteration)
        {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::MissingStatementPlaceFact { statement },
            ));
        }

        let iterable_type = self
            .core
            .types
            .assignments
            .expression(iterable)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(iterable)),
                )
            })?;
        let iterable_type = self.concrete_type(iterable_type, origin)?;
        let iterable = self.lower_expression(iterable, 0)?;
        let list_local = self.temporary(iterable_type, origin)?;
        self.emit(list_local, Rvalue::Use(iterable), origin)?;

        let int = self.core.types.interner.primitive(PrimitiveType::Int);
        let bool_type = self.core.types.interner.primitive(PrimitiveType::Bool);
        let index_local = self.temporary(int, origin)?;
        self.emit(
            index_local,
            Rvalue::Use(Operand::Constant(Constant::Integer(0))),
            origin,
        )?;

        let condition_block = self.new_block(origin)?;
        let body_block = self.new_block(self.core.package.bodies[body].origin)?;
        let step_block = self.new_block(origin)?;
        let exit_block = self.new_block(origin)?;
        self.terminate(DraftTerminator::Goto(condition_block));

        self.current = Some(condition_block);
        let length =
            self.assign_temporary(int, Rvalue::ListLen(Operand::Copy(list_local)), origin)?;
        let condition = self.assign_temporary(
            bool_type,
            Rvalue::Binary {
                left: Operand::Copy(index_local),
                operator: lpp_hir::BinaryOperator::Less,
                right: length,
            },
            origin,
        )?;
        self.terminate(DraftTerminator::Branch {
            condition,
            then_block: body_block,
            else_block: exit_block,
        });

        self.current = Some(body_block);
        let binding_local = self.local(binding, MirLocalKind::User)?;
        let binding_type = self.core.program.local(binding_local).unwrap().ty;
        let place = self.place(
            list_local,
            &[PlaceProjection::ListIndex(Operand::Copy(index_local))],
            binding_type,
            origin,
        )?;
        self.emit(binding_local, Rvalue::Load(place), origin)?;
        self.loops.push((exit_block, step_block));
        self.lower_body(body)?;
        self.loops.pop();
        if self.current.is_some() {
            self.terminate(DraftTerminator::Goto(step_block));
        }

        self.current = Some(step_block);
        let next = self.assign_temporary(
            int,
            Rvalue::Binary {
                left: Operand::Copy(index_local),
                operator: lpp_hir::BinaryOperator::Add,
                right: Operand::Constant(Constant::Integer(1)),
            },
            origin,
        )?;
        self.emit(index_local, Rvalue::Use(next), origin)?;
        self.terminate(DraftTerminator::Goto(condition_block));

        self.current = Some(exit_block);
        Ok(())
    }

    pub(super) fn lower_if(
        &mut self,
        condition: ExprId,
        then_body: BodyId,
        else_body: Option<BodyId>,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        let condition = self.lower_expression(condition, 0)?;
        let then_block = self.new_block(self.core.package.bodies[then_body].origin)?;
        let else_origin = else_body.map_or(origin, |body| self.core.package.bodies[body].origin);
        let else_block = self.new_block(else_origin)?;
        let join_block = self.new_block(origin)?;
        self.terminate(DraftTerminator::Branch {
            condition,
            then_block,
            else_block,
        });

        self.current = Some(then_block);
        self.lower_body(then_body)?;
        let then_falls_through = self.current.is_some();
        if then_falls_through {
            self.terminate(DraftTerminator::Goto(join_block));
        }

        self.current = Some(else_block);
        if let Some(else_body) = else_body {
            self.lower_body(else_body)?;
        }
        let else_falls_through = self.current.is_some();
        if else_falls_through {
            self.terminate(DraftTerminator::Goto(join_block));
        }

        if then_falls_through || else_falls_through {
            self.current = Some(join_block);
        } else {
            self.blocks[join_block].terminator = Some(DraftTerminator::Unreachable);
            self.current = None;
        }
        Ok(())
    }

    pub(super) fn lower_while(
        &mut self,
        condition: ExprId,
        body: BodyId,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        let condition_block = self.new_block(origin)?;
        let body_block = self.new_block(self.core.package.bodies[body].origin)?;
        let exit_block = self.new_block(origin)?;
        self.terminate(DraftTerminator::Goto(condition_block));

        self.current = Some(condition_block);
        let condition = self.lower_expression(condition, 0)?;
        self.terminate(DraftTerminator::Branch {
            condition,
            then_block: body_block,
            else_block: exit_block,
        });

        self.loops.push((exit_block, condition_block));
        self.current = Some(body_block);
        self.lower_body(body)?;
        self.loops.pop();
        if self.current.is_some() {
            self.terminate(DraftTerminator::Goto(condition_block));
        }
        self.current = Some(exit_block);
        Ok(())
    }
}
