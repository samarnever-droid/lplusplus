use super::*;

/// Identity and shape of one MIR function. Closures fill in a synthetic
/// descriptor: the enclosing item's identity, no definition or name, a
/// `Closure` kind, and the enclosing locals they capture.
#[derive(Debug, Clone)]
pub(super) struct FunctionDescriptor {
    pub(super) source: HirItemId,
    pub(super) instance: Option<InstanceId>,
    pub(super) instance_arguments: TypeListId,
    pub(super) definition: Option<lpp_hir::DefId>,
    pub(super) name: Option<lpp_hir::Symbol>,
    pub(super) kind: MirFunctionKind,
    pub(super) captures: Vec<MirLocalId>,
    pub(super) origin: OriginId,
}

impl FunctionBuilder<'_, '_> {
    pub(super) fn concrete_type(
        &mut self,
        ty: TypeId,
        origin: OriginId,
    ) -> Result<TypeId, MirBuildError> {
        if let Some(concrete) = self.type_cache.get(&ty) {
            return Ok(*concrete);
        }
        let concrete = self.core.materialize_type(&self.substitution, ty, origin)?;
        self.type_cache.insert(ty, concrete);
        Ok(concrete)
    }

    pub(super) fn new_block(&mut self, origin: OriginId) -> Result<usize, MirBuildError> {
        self.core.check_next(
            MirCapacity::Blocks,
            self.core.counts.blocks,
            self.core.options.max_blocks,
            origin,
        )?;
        self.core.counts.blocks += 1;
        let block = self.blocks.len();
        self.blocks.push(BlockDraft {
            instructions: Vec::new(),
            terminator: None,
            origin,
        });
        Ok(block)
    }

    pub(super) fn local(
        &mut self,
        source: LocalId,
        kind: MirLocalKind,
    ) -> Result<MirLocalId, MirBuildError> {
        if let Some(local) = self.core.local_map[source.index()] {
            return Ok(local);
        }
        let hir_local = self.core.package.locals[source];
        let ty = self.core.types.assignments.local(source).ok_or_else(|| {
            self.core.error(
                hir_local.origin,
                MirBuildErrorKind::MissingType(TypedEntity::Local(source)),
            )
        })?;
        let ty = self.concrete_type(ty, hir_local.origin)?;
        // By-reference capture cell: the local holds the single-element
        // list, and references to it read and store the head element.
        let ty = if self.core.cell_sources.contains(&source) {
            self.core.cell_type(ty, hir_local.origin)?
        } else {
            ty
        };
        self.core.check_next(
            MirCapacity::Locals,
            self.core.counts.locals,
            self.core.options.max_locals,
            hir_local.origin,
        )?;
        let local = self
            .core
            .program
            .locals
            .alloc(MirLocal {
                ty,
                source: Some(source),
                kind,
                mutable: hir_local.mutable,
                origin: hir_local.origin,
            })
            .map_err(|_| {
                self.core.error(
                    hir_local.origin,
                    MirBuildErrorKind::Capacity(MirCapacity::Storage),
                )
            })?;
        self.core.counts.locals += 1;
        self.core.local_map[source.index()] = Some(local);
        self.mapped_sources.push(source);
        self.locals.push(local);
        Ok(local)
    }

    pub(super) fn temporary(
        &mut self,
        ty: TypeId,
        origin: OriginId,
    ) -> Result<MirLocalId, MirBuildError> {
        self.temporary_with_mutability(ty, false, origin)
    }

    pub(super) fn temporary_with_mutability(
        &mut self,
        ty: TypeId,
        mutable: bool,
        origin: OriginId,
    ) -> Result<MirLocalId, MirBuildError> {
        self.core.check_next(
            MirCapacity::Locals,
            self.core.counts.locals,
            self.core.options.max_locals,
            origin,
        )?;
        let local = self
            .core
            .program
            .locals
            .alloc(MirLocal {
                ty,
                source: None,
                kind: MirLocalKind::Temporary,
                mutable,
                origin,
            })
            .map_err(|_| {
                self.core
                    .error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
            })?;
        self.core.counts.locals += 1;
        self.locals.push(local);
        Ok(local)
    }

    pub(super) fn lower_body(&mut self, body: BodyId) -> Result<(), MirBuildError> {
        let statements = self
            .core
            .package
            .statements(self.core.package.bodies[body].statements)
            .to_vec();
        for statement in statements {
            if self.current.is_none() {
                break;
            }
            self.lower_statement(statement)?;
        }
        Ok(())
    }

    pub(super) fn assign_temporary(
        &mut self,
        ty: TypeId,
        value: Rvalue,
        origin: OriginId,
    ) -> Result<Operand, MirBuildError> {
        let target = self.temporary(ty, origin)?;
        self.emit(target, value, origin)?;
        Ok(Operand::Copy(target))
    }

    pub(super) fn store_operands(
        &mut self,
        operands: &[Operand],
        origin: OriginId,
    ) -> Result<ListRange<Operand>, MirBuildError> {
        let next = self
            .core
            .counts
            .operands
            .checked_add(operands.len())
            .ok_or_else(|| {
                self.core
                    .error(origin, MirBuildErrorKind::Capacity(MirCapacity::Operands))
            })?;
        if next > self.core.options.max_operands {
            return Err(self
                .core
                .error(origin, MirBuildErrorKind::Capacity(MirCapacity::Operands)));
        }
        let range = self
            .core
            .program
            .lists
            .operands
            .extend(operands)
            .map_err(|error| self.core.storage_error(origin, error))?;
        self.core.counts.operands = next;
        Ok(range)
    }

    pub(super) fn place(
        &mut self,
        root: MirLocalId,
        projections: &[PlaceProjection],
        ty: TypeId,
        origin: OriginId,
    ) -> Result<MirPlaceId, MirBuildError> {
        self.core.check_next(
            MirCapacity::Places,
            self.core.counts.places,
            self.core.options.max_places,
            origin,
        )?;
        let projection_count = self
            .core
            .counts
            .projections
            .checked_add(projections.len())
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::Capacity(MirCapacity::Projections),
                )
            })?;
        if projection_count > self.core.options.max_projections {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::Capacity(MirCapacity::Projections),
            ));
        }
        let projections = self
            .core
            .program
            .lists
            .projections
            .extend(projections)
            .map_err(|error| self.core.storage_error(origin, error))?;
        let place = self
            .core
            .program
            .places
            .alloc(MirPlace {
                root,
                projections,
                ty,
                origin,
            })
            .map_err(|_| {
                self.core
                    .error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
            })?;
        self.core.counts.places += 1;
        self.core.counts.projections = projection_count;
        Ok(place)
    }

    pub(super) fn emit(
        &mut self,
        target: MirLocalId,
        value: Rvalue,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        self.emit_instruction(InstructionKind::Assign { target, value }, origin)
    }

    pub(super) fn emit_store(
        &mut self,
        place: MirPlaceId,
        value: Operand,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        self.emit_instruction(InstructionKind::Store { place, value }, origin)
    }

    fn emit_instruction(
        &mut self,
        kind: InstructionKind,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        self.core.check_next(
            MirCapacity::Instructions,
            self.core.counts.instructions,
            self.core.options.max_instructions,
            origin,
        )?;
        self.core.counts.instructions += 1;
        let current = self.current.expect("instructions require a current block");
        self.blocks[current]
            .instructions
            .push(Instruction { kind, origin });
        Ok(())
    }

    pub(super) fn reserve_switch_targets(
        &mut self,
        additional: usize,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        let committed_and_pending = self
            .core
            .counts
            .switch_targets
            .saturating_add(self.pending_switch_targets);
        if additional
            > self
                .core
                .options
                .max_switch_targets
                .saturating_sub(committed_and_pending)
        {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::Capacity(MirCapacity::SwitchTargets),
            ));
        }
        self.pending_switch_targets = self.pending_switch_targets.saturating_add(additional);
        Ok(())
    }

    pub(super) fn terminate(&mut self, terminator: DraftTerminator) {
        let current = self
            .current
            .take()
            .expect("terminators require a current block");
        assert!(
            self.blocks[current]
                .terminator
                .replace(terminator)
                .is_none(),
            "a MIR block receives one terminator",
        );
    }

    pub(super) fn finish(
        mut self,
        descriptor: FunctionDescriptor,
        function_id: MirFunctionId,
        function_type: TypeId,
        entry: usize,
    ) -> Result<(), MirBuildError> {
        let origin = descriptor.origin;
        if let Some(current) = self.current {
            if self.return_type == self.core.types.interner.primitive(PrimitiveType::Void) {
                self.blocks[current].terminator = Some(DraftTerminator::Return(None));
            } else {
                return Err(self.core.error(origin, MirBuildErrorKind::MissingReturn));
            }
        }

        let block_base = self.core.program.blocks.len();
        let block_ids = (0..self.blocks.len())
            .map(|offset| {
                BasicBlockId::from_index(block_base + offset).ok_or_else(|| {
                    self.core
                        .error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let entry = block_ids[entry];

        for (offset, draft) in self.blocks.drain(..).enumerate() {
            let mut instruction_ids = Vec::with_capacity(draft.instructions.len());
            for instruction in draft.instructions {
                let id = self
                    .core
                    .program
                    .instructions
                    .alloc(instruction)
                    .map_err(|_| {
                        self.core.error(
                            draft.origin,
                            MirBuildErrorKind::Capacity(MirCapacity::Storage),
                        )
                    })?;
                instruction_ids.push(id);
            }
            let instructions = self
                .core
                .program
                .lists
                .instructions
                .extend(&instruction_ids)
                .map_err(|error| self.core.storage_error(draft.origin, error))?;
            let terminator = match draft.terminator.unwrap_or(DraftTerminator::Unreachable) {
                DraftTerminator::Goto(target) => Terminator::Goto(block_ids[target]),
                DraftTerminator::Branch {
                    condition,
                    then_block,
                    else_block,
                } => Terminator::Branch {
                    condition,
                    then_block: block_ids[then_block],
                    else_block: block_ids[else_block],
                },
                DraftTerminator::SwitchEnum {
                    subject,
                    aggregate,
                    targets,
                } => {
                    if targets.len()
                        > self
                            .core
                            .options
                            .max_switch_targets
                            .saturating_sub(self.core.counts.switch_targets)
                    {
                        return Err(self.core.error(
                            draft.origin,
                            MirBuildErrorKind::Capacity(MirCapacity::SwitchTargets),
                        ));
                    }
                    let targets = targets
                        .into_iter()
                        .map(|target| block_ids[target])
                        .collect::<Vec<_>>();
                    let targets = self
                        .core
                        .program
                        .lists
                        .switch_targets
                        .extend(&targets)
                        .map_err(|error| self.core.storage_error(draft.origin, error))?;
                    self.core.counts.switch_targets += targets.len();
                    Terminator::SwitchEnum {
                        subject,
                        aggregate,
                        targets,
                    }
                }
                DraftTerminator::Return(value) => Terminator::Return(value),
                DraftTerminator::Unreachable => Terminator::Unreachable,
            };
            let id = self
                .core
                .program
                .blocks
                .alloc(BasicBlock {
                    instructions,
                    terminator,
                    origin: draft.origin,
                })
                .map_err(|_| {
                    self.core.error(
                        draft.origin,
                        MirBuildErrorKind::Capacity(MirCapacity::Storage),
                    )
                })?;
            debug_assert_eq!(id, block_ids[offset]);
        }

        let parameters = self
            .core
            .program
            .lists
            .parameters
            .extend(&self.parameters)
            .map_err(|error| self.core.storage_error(origin, error))?;
        let locals = self
            .core
            .program
            .lists
            .locals
            .extend(&self.locals)
            .map_err(|error| self.core.storage_error(origin, error))?;
        let blocks = self
            .core
            .program
            .lists
            .blocks
            .extend(&block_ids)
            .map_err(|error| self.core.storage_error(origin, error))?;
        let captures = self
            .core
            .program
            .lists
            .captures
            .extend(&descriptor.captures)
            .map_err(|error| self.core.storage_error(origin, error))?;
        let actual = self
            .core
            .program
            .functions
            .alloc(MirFunction {
                source: descriptor.source,
                instance: descriptor.instance,
                instance_arguments: descriptor.instance_arguments,
                definition: descriptor.definition,
                name: descriptor.name,
                kind: descriptor.kind,
                captures,
                ty: function_type,
                return_type: self.return_type,
                parameters,
                locals,
                blocks,
                entry,
                origin,
            })
            .map_err(|_| {
                self.core
                    .error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
            })?;
        debug_assert_eq!(actual, function_id);
        for source in self.mapped_sources {
            self.core.local_map[source.index()] = None;
        }
        Ok(())
    }
}
