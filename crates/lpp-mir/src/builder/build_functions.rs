use super::*;

pub(super) struct ClosureLowerSpec {
    pub(super) expression: ExprId,
    pub(super) parameters: IdRange<LocalId>,
    pub(super) body: BodyId,
    pub(super) enclosing_source: HirItemId,
    pub(super) substitution: TypeSubstitution,
    pub(super) capture_locals: Vec<MirLocalId>,
    pub(super) function_type: TypeId,
}

impl<'input> MirBuilder<'input> {
    pub(super) fn new(
        package: &'input HirPackage,
        sources: &'input SourceMap,
        types: &'input mut ShadowTypeOutput,
        options: MirBuildOptions,
    ) -> Self {
        let instance_count = types.instances.records().len();
        let mut external_items = BTreeMap::new();
        for (_, item) in package.items.enumerate() {
            let HirItemKind::Extern(block) = item.kind else {
                continue;
            };
            for &function in package.items(block.functions) {
                external_items.insert(function, (block.abi, block.link_library));
            }
        }
        Self {
            package,
            sources,
            types,
            options,
            type_budget: TypeWorkBudget::with_max_depth(
                options.max_type_work,
                options.max_type_depth,
            ),
            counts: BuildCounts::default(),
            program: MirProgram::with_capacity(
                package.items.len(),
                package.locals.len(),
                package.expressions.len(),
                package.statements.len(),
            ),
            item_functions: vec![None; package.items.len()],
            definition_items: vec![None; package.names.definitions.len()],
            definition_functions: vec![None; package.names.definitions.len()],
            external_items,
            external_function_ids: BTreeMap::new(),
            instance_functions: vec![None; instance_count],
            instance_type_functions: BTreeMap::new(),
            aggregate_types: BTreeMap::new(),
            aggregate_fields: BTreeMap::new(),
            aggregate_variants: BTreeMap::new(),
            local_map: vec![None; package.locals.len()],
            string_cache: BTreeMap::new(),
            pending_closures: std::collections::VecDeque::new(),
            cell_sources: BTreeSet::new(),
        }
    }

    pub(super) fn build(mut self) -> Result<MirProgram, MirBuildError> {
        let fallback_origin = self
            .package
            .items
            .iter()
            .next()
            .map(|item| item.origin)
            .expect("a typed HIR package has at least one item");
        if !self.types.instance_diagnostics.is_empty() || self.types.instances.finish().is_err() {
            return Err(self.error(
                fallback_origin,
                MirBuildErrorKind::InvalidInstancePlan {
                    diagnostics: self.types.instance_diagnostics.len(),
                },
            ));
        }
        for (item_id, item) in self.package.items.enumerate() {
            if let Some(definition) = item.definition {
                self.definition_items[definition.index()] = Some(item_id);
            }
        }

        let mut work = Vec::new();
        for (item_id, item) in self.package.items.enumerate() {
            let HirItemKind::Function(function) = item.kind else {
                continue;
            };
            if function.body.is_none() || !function.type_parameters.is_empty() {
                continue;
            }
            self.validate_function_shape(function, item.origin)?;
            let function_type = self.type_of_item(item_id, item.origin)?;
            let function_id = self.reserve_function(work.len(), item.origin)?;
            self.item_functions[item_id.index()] = Some(function_id);
            if let Some(definition) = item.definition {
                self.definition_functions[definition.index()] = Some(function_id);
            }
            work.push(FunctionWork {
                item: item_id,
                instance: None,
                instance_arguments: self.types.interner.empty_list(),
                function_type,
                substitution: TypeSubstitution::new(),
            });
        }

        let records = self.types.instances.records().to_vec();
        for record in records {
            if record.kind != InstanceRequestKind::Required {
                continue;
            }
            let item = self.package.items[record.key.item];
            let HirItemKind::Function(function) = item.kind else {
                continue;
            };
            if function.body.is_none() || function.type_parameters.is_empty() {
                continue;
            }
            self.validate_function_shape(function, item.origin)?;
            let parameters = self
                .package
                .type_parameters(function.type_parameters)
                .to_vec();
            let arguments = self.types.interner.list(record.key.arguments).to_vec();
            if parameters.len() != arguments.len() {
                return Err(self.error(
                    record.origin.unwrap_or(item.origin),
                    MirBuildErrorKind::InvalidInstanceArity {
                        instance: record.id,
                        expected: parameters.len(),
                        actual: arguments.len(),
                    },
                ));
            }
            let mut substitution = TypeSubstitution::new();
            for (parameter, argument) in parameters.iter().zip(arguments) {
                substitution.insert(*parameter, argument);
            }
            let template_type = self.type_of_item(record.key.item, item.origin)?;
            let function_type = self.materialize_type(&substitution, template_type, item.origin)?;
            let function_id = self.reserve_function(work.len(), item.origin)?;
            self.instance_functions[record.id.index()] = Some(function_id);
            self.instance_type_functions
                .entry((record.key.item, function_type))
                .and_modify(|entry| *entry = None)
                .or_insert(Some(function_id));
            work.push(FunctionWork {
                item: record.key.item,
                instance: Some(record.id),
                instance_arguments: record.key.arguments,
                function_type,
                substitution,
            });
        }

        for function in work {
            self.lower_function(function)?;
        }
        // Drain the closure queue once, after every item is lowered:
        // items are reserved first, then closures in encounter order, so
        // a single FIFO drain allocates every function in reservation
        // order. Draining per item would interleave closure allocations
        // with later item allocations and break the ID numbering.
        self.lower_pending_closures()?;
        self.ensure_program_aggregate_types(fallback_origin)?;
        Ok(self.program)
    }

    pub(super) fn lower_function(&mut self, work: FunctionWork) -> Result<(), MirBuildError> {
        let item = self.package.items[work.item];
        let HirItemKind::Function(function) = item.kind else {
            unreachable!("the function worklist contains only functions")
        };
        let TypeKind::Function {
            parameters: _,
            result,
        } = self.types.interner.kind(work.function_type)
        else {
            return Err(self.error(
                item.origin,
                MirBuildErrorKind::InvalidFunctionType(work.function_type),
            ));
        };
        // Async functions expose `fn(...) -> Task<R>`; the body itself
        // produces `R`, so the MIR function's return type is `R`.
        let return_type = if function.is_async {
            let TypeKind::Task(return_type) = self.types.interner.kind(result) else {
                return Err(self.error(
                    item.origin,
                    MirBuildErrorKind::InvalidFunctionType(work.function_type),
                ));
            };
            return_type
        } else {
            result
        };
        let body = function.body.ok_or_else(|| {
            self.error(
                item.origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
            )
        })?;
        let entry_origin = self.package.bodies[body].origin;
        let parameters = self.package.locals(function.parameters).to_vec();
        // Default parameters are supported: the body sees every parameter as a
        // normal local, and call sites fill omitted trailing arguments with the
        // parameter's default expression (see `lower_defaulted_call`).
        let function_id = match work.instance {
            Some(instance) => self.instance_functions[instance.index()],
            None => self.item_functions[work.item.index()],
        }
        .expect("the function was assigned an identity before lowering");
        let mut fn_attrs = MirFunctionAttrs {
            is_unsafe: function.is_unsafe,
            ..MirFunctionAttrs::default()
        };
        for &attr_sym in self.package.symbols(item.attributes) {
            let Some(text) = self.package.names.symbols.resolve(attr_sym) else {
                continue;
            };
            let text = text.trim();
            if text == "naked" {
                fn_attrs.is_naked = true;
            } else if text == "interrupt" {
                fn_attrs.is_interrupt = true;
            } else if let Some(inner) = text
                .strip_prefix("export_name(")
                .and_then(|s| s.strip_suffix(')'))
            {
                let unquoted = inner.trim().trim_matches('"').to_owned();
                if !unquoted.is_empty() {
                    fn_attrs.export_name = Some(unquoted);
                }
            } else if let Some(inner) = text
                .strip_prefix("section(")
                .and_then(|s| s.strip_suffix(')'))
            {
                let unquoted = inner.trim().trim_matches('"').to_owned();
                if !unquoted.is_empty() {
                    fn_attrs.section = Some(unquoted);
                }
            }
        }
        if fn_attrs != MirFunctionAttrs::default() {
            self.program.function_attrs.insert(function_id, fn_attrs);
        }
        if function.is_unsafe {
            self.program.unsafe_sites.push(UnsafeSite {
                origin: item.origin,
                kind: UnsafeSiteKind::UnsafeFunction,
            });
        }
        // Mark by-reference capture cells before any local of this body
        // is materialized: the mark changes how each local is typed and
        // referenced throughout the frame.
        let parameter_sources: BTreeSet<LocalId> = parameters.iter().copied().collect();
        self.mark_write_captures(body, &parameter_sources);
        let mut function_builder = FunctionBuilder {
            core: self,
            source: work.item,
            substitution: work.substitution,
            type_cache: BTreeMap::new(),
            mapped_sources: Vec::new(),
            return_type,
            locals: Vec::new(),
            parameters: Vec::new(),
            blocks: Vec::new(),
            current: None,
            pending_switch_targets: 0,
            loops: Vec::new(),
        };
        let entry = function_builder.new_block(entry_origin)?;
        function_builder.current = Some(entry);
        for parameter in parameters {
            let local = function_builder.local(parameter, MirLocalKind::Parameter)?;
            function_builder.parameters.push(local);
        }
        function_builder.lower_body(body)?;
        let kind = if function.is_async {
            MirFunctionKind::Async
        } else {
            MirFunctionKind::Function
        };
        function_builder.finish(
            FunctionDescriptor {
                source: work.item,
                instance: work.instance,
                instance_arguments: work.instance_arguments,
                definition: item.definition,
                name: item.name,
                kind,
                captures: Vec::new(),
                origin: item.origin,
            },
            function_id,
            work.function_type,
            entry,
        )
    }

    pub(super) fn validate_function_shape(
        &self,
        _function: lpp_hir::Function,
        _origin: OriginId,
    ) -> Result<(), MirBuildError> {
        // Variadic functions are supported: the type checker rewrites the rest
        // parameter to `List[element]`, so the definition lowers like any other
        // function with a trailing list parameter, and call sites collect the
        // trailing arguments into that list (see `lower_variadic_call`).
        Ok(())
    }

    /// Reserve the closure's MIR function ID and queue its body for
    /// lowering. The body finishes after the enclosing function so function
    /// IDs keep reservation order while allocation order follows body
    /// completion. The closure frame begins with one capture local per
    /// enclosing local in `capture_locals` (in HIR declaration order),
    /// followed by the closure parameters.
    pub(super) fn lower_closure_function(
        &mut self,
        spec: ClosureLowerSpec,
    ) -> Result<MirFunctionId, MirBuildError> {
        let ClosureLowerSpec {
            expression,
            parameters,
            body,
            enclosing_source,
            substitution,
            capture_locals,
            function_type,
        } = spec;
        let ExpressionKind::Closure { .. } = self.package.expressions[expression].kind else {
            unreachable!("closure lowering receives closure expressions only");
        };
        let TypeKind::Function {
            parameters: _,
            result: _,
        } = self.types.interner.kind(function_type)
        else {
            return Err(self.error(
                self.package.expressions[expression].origin,
                MirBuildErrorKind::InvalidFunctionType(function_type),
            ));
        };
        let origin = self.package.expressions[expression].origin;
        let function_id = self.reserve_function(self.counts.functions, origin)?;
        self.pending_closures.push_back(ClosureWork {
            expression,
            parameters,
            body,
            enclosing_source,
            substitution,
            capture_locals,
            function_type,
            function_id,
        });
        Ok(function_id)
    }

    /// Lower every closure body queued during the enclosing pass, in
    /// reservation order, so function IDs keep their reserved numbering.
    pub(super) fn lower_pending_closures(&mut self) -> Result<(), MirBuildError> {
        while let Some(work) = self.pending_closures.pop_front() {
            self.lower_queued_closure(work)?;
        }
        Ok(())
    }

    pub(super) fn lower_queued_closure(&mut self, work: ClosureWork) -> Result<(), MirBuildError> {
        let ClosureWork {
            expression,
            parameters,
            body,
            enclosing_source,
            substitution,
            capture_locals,
            function_type,
            function_id,
        } = work;
        let TypeKind::Function {
            parameters: _,
            result,
        } = self.types.interner.kind(function_type)
        else {
            return Err(self.error(
                self.package.expressions[expression].origin,
                MirBuildErrorKind::InvalidFunctionType(function_type),
            ));
        };
        let origin = self.package.expressions[expression].origin;

        // Pair each enclosing MIR local with the source local it mirrors so
        // the closure body's name references (still keyed by the original
        // LocalId) map into this frame.
        let mut capture_sources: Vec<LocalId> = Vec::with_capacity(capture_locals.len());
        for enclosing in &capture_locals {
            let source = self
                .program
                .local(*enclosing)
                .and_then(|local| local.source)
                .expect("enclosing captures reference source locals");
            capture_sources.push(source);
        }

        // The closure frame owns one writable slot per captured source local,
        // initialized from the enclosing frame at creation time. Capture
        // slots precede the closure parameters in frame order.
        let mut capture_frame_locals = Vec::with_capacity(capture_sources.len());
        for source in &capture_sources {
            let hir_local = self.package.locals[*source];
            let ty = self.types.assignments.local(*source).ok_or_else(|| {
                self.error(
                    hir_local.origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Local(*source)),
                )
            })?;
            let ty = self.materialize_type(&substitution, ty, hir_local.origin)?;
            // A by-reference capture cell continues into this frame: the
            // capture local is the cell (list) itself, so reads and
            // stores in the body go through the shared element.
            let ty = if self.cell_sources.contains(source) {
                self.cell_type(ty, hir_local.origin)?
            } else {
                ty
            };
            self.check_next(
                MirCapacity::Locals,
                self.counts.locals,
                self.options.max_locals,
                hir_local.origin,
            )?;
            let capture = self
                .program
                .locals
                .alloc(MirLocal {
                    ty,
                    source: Some(*source),
                    kind: MirLocalKind::Capture,
                    mutable: true,
                    origin: hir_local.origin,
                })
                .map_err(|_| {
                    self.error(
                        hir_local.origin,
                        MirBuildErrorKind::Capacity(MirCapacity::Storage),
                    )
                })?;
            self.counts.locals += 1;
            capture_frame_locals.push(capture);
        }

        let closure_parameters = self.package.locals(parameters).to_vec();
        for parameter in &closure_parameters {
            if self.package.locals[*parameter].default.is_some() {
                return Err(self.error(
                    self.package.locals[*parameter].origin,
                    MirBuildErrorKind::Unsupported(UnsupportedConstruct::ParameterDefault),
                ));
            }
        }
        let entry_origin = self.package.bodies[body].origin;
        let empty_list = self.types.interner.empty_list();

        // Map the captured source locals into this frame before the body
        // lowers; restore the enclosing mapping afterwards so the enclosing
        // function keeps its own locals.
        let mut remapped: Vec<(LocalId, Option<MirLocalId>)> = Vec::new();
        for (source, capture) in capture_sources.iter().zip(capture_frame_locals.iter()) {
            remapped.push((*source, self.local_map[source.index()]));
            self.local_map[source.index()] = Some(*capture);
        }
        let mut builder = FunctionBuilder {
            core: self,
            source: enclosing_source,
            substitution,
            type_cache: BTreeMap::new(),
            mapped_sources: Vec::new(),
            return_type: result,
            locals: capture_frame_locals.clone(),
            parameters: capture_frame_locals.clone(),
            blocks: Vec::new(),
            current: None,
            pending_switch_targets: 0,
            loops: Vec::new(),
        };

        for parameter in closure_parameters {
            let local = builder.local(parameter, MirLocalKind::Parameter)?;
            builder.parameters.push(local);
        }

        let entry = builder.new_block(entry_origin)?;
        builder.current = Some(entry);
        builder.lower_body(body)?;
        let descriptor = FunctionDescriptor {
            source: enclosing_source,
            instance: None,
            instance_arguments: empty_list,
            definition: None,
            name: None,
            kind: MirFunctionKind::Closure,
            captures: capture_locals,
            origin,
        };
        builder.finish(descriptor, function_id, function_type, entry)?;
        for (source, previous) in remapped {
            self.local_map[source.index()] = previous;
        }
        Ok(())
    }
}
