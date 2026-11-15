use super::*;

impl<'m> Lowering<'m> {
    pub(crate) fn new(
        module: &'m mut ObjectModule,
        aggregates: AggregateIndex,
        layouts: HashMap<MirAggregateId, AggregateLayout>,
        value_classes: BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
        shared_types: BTreeSet<TypeId>,
        stack_locals: BTreeMap<(MirFunctionId, MirLocalId), u32>,
        pgo_fingerprint: Option<u64>,
    ) -> Self {
        Self {
            module,
            aggregates,
            layouts,
            func_ids: BTreeMap::new(),
            dtor_ids: BTreeMap::new(),
            trace_ids: BTreeMap::new(),
            closure_dtor_ids: BTreeMap::new(),
            closure_trace_ids: BTreeMap::new(),
            tuple_dtor_ids: BTreeMap::new(),
            tuple_trace_ids: BTreeMap::new(),
            shared_types,
            stack_locals,
            pgo_fingerprint,
            task_thunk_ids: BTreeMap::new(),
            closure_thunk_ids: BTreeMap::new(),
            value_classes,
            import_ids: BTreeMap::new(),
            external_ids: BTreeMap::new(),
            exported: BTreeSet::new(),
            imported: BTreeSet::new(),
            entry: None,
            lpp_main_id: None,
            lpp_main_function_id: None,
            emitted_strings: BTreeMap::new(),
            debug_records: Vec::new(),
        }
    }

    pub(crate) fn take_debug_records(&mut self) -> Vec<crate::dwarf::CompiledDebugFunction> {
        std::mem::take(&mut self.debug_records)
    }

    pub(crate) fn exported_symbols(&self) -> BTreeSet<String> {
        self.exported.clone()
    }

    pub(crate) fn imported_symbols(&self) -> BTreeSet<String> {
        self.imported.clone()
    }

    pub(crate) fn entry(&self) -> Option<String> {
        self.entry.clone()
    }

    /// Declare the module's symbols: runtime imports first (only the
    /// ones the pre-scan found used, in stable order), then direct C
    /// imports in `MirExternalFunctionId` order, generated destructors
    /// in `MirAggregateId` order, and user exports in `MirFunctionId`
    /// order.
    pub(crate) fn declare(
        &mut self,
        plan: &Plan,
        program: &MirProgram,
        types: &TypeInterner,
    ) -> Result<(), CodegenError> {
        for name in &plan.imports {
            let (params, result) = import_signature(name);
            let mut sig = self.module.make_signature();
            for param in params {
                sig.params.push(AbiParam::new(*param));
            }
            if let Some(result) = result {
                sig.returns.push(AbiParam::new(result));
            }
            let id = self
                .module
                .declare_function(name, Linkage::Import, &sig)
                .map_err(|e| emission_failed(format!("declare import {name}: {e:?}")))?;
            self.import_ids.insert(name, id);
            self.imported.insert(name.to_string());
        }

        for &external_id in &plan.external_functions {
            let external = program
                .external_function(external_id)
                .expect("pre-scan validates external function references");
            let TypeKind::Function { parameters, result } = types.kind(external.ty) else {
                return Err(emission_failed(
                    "external function has a non-function type".to_string(),
                ));
            };
            let mut sig = self.module.make_signature();
            for parameter in types.list(parameters) {
                let ty = machine_type(types, *parameter).ok_or_else(|| {
                    emission_failed("unsupported external parameter type".to_string())
                })?;
                sig.params.push(AbiParam::new(ty));
            }
            if !is_void_type(types, result) {
                let ty = machine_type(types, result).ok_or_else(|| {
                    emission_failed("unsupported external result type".to_string())
                })?;
                sig.returns.push(AbiParam::new(ty));
            }
            let symbol = external.name.clone();
            let id = self
                .module
                .declare_function(&symbol, Linkage::Import, &sig)
                .map_err(|e| emission_failed(format!("declare external {symbol}: {e:?}")))?;
            self.external_ids.insert(external_id, id);
            self.imported.insert(symbol);
        }

        for &aggregate in &plan.destructors {
            let agg = program
                .aggregate(aggregate)
                .expect("validated MIR retains every aggregate");
            let tag = match agg.kind {
                lpp_mir::MirAggregateKind::Struct => 's',
                lpp_mir::MirAggregateKind::Enum => 'e',
            };
            let name = format!("lpp_drop_{tag}{}", aggregate.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare destructor {name}: {e:?}")))?;
            self.dtor_ids.insert(aggregate, id);
            self.exported.insert(name);
        }

        for &function_id in &plan.closure_dtors {
            let name = format!("lpp_drop_c{}", function_id.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare closure dtor {name}: {e:?}")))?;
            self.closure_dtor_ids.insert(function_id, id);
            self.exported.insert(name);
        }

        for &aggregate in &plan.traces {
            let agg = program
                .aggregate(aggregate)
                .expect("validated MIR retains every aggregate");
            let tag = match agg.kind {
                lpp_mir::MirAggregateKind::Struct => 's',
                lpp_mir::MirAggregateKind::Enum => 'e',
            };
            let name = format!("lpp_trace_{tag}{}", aggregate.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.params.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare trace {name}: {e:?}")))?;
            self.trace_ids.insert(aggregate, id);
            self.exported.insert(name);
        }

        for &function_id in &plan.closure_traces {
            let name = format!("lpp_trace_c{}", function_id.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.params.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare closure trace {name}: {e:?}")))?;
            self.closure_trace_ids.insert(function_id, id);
            self.exported.insert(name);
        }

        for &ty in &plan.tuple_destructors {
            let name = format!("lpp_drop_t{}", ty.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare tuple dtor {name}: {e:?}")))?;
            self.tuple_dtor_ids.insert(ty, id);
            self.exported.insert(name);
        }

        for &ty in &plan.tuple_traces {
            let name = format!("lpp_trace_t{}", ty.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.params.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare tuple trace {name}: {e:?}")))?;
            self.tuple_trace_ids.insert(ty, id);
            self.exported.insert(name);
        }

        for &function_id in &plan.task_thunks {
            let name = format!("__lpp_task_thunk{}", function_id.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.returns.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare task thunk {name}: {e:?}")))?;
            self.task_thunk_ids.insert(function_id, id);
            self.exported.insert(name);
        }

        for &function_id in &plan.closure_thunks {
            let name = format!("__lpp_closure_thunk{}", function_id.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.returns.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare closure thunk {name}: {e:?}")))?;
            self.closure_thunk_ids.insert(function_id, id);
            self.exported.insert(name);
        }

        for (mir_id, function) in program.functions() {
            let name = plan
                .export_names
                .get(&mir_id)
                .expect("pre_scan named every function");
            let is_closure = matches!(function.kind, MirFunctionKind::Closure);
            let mut sig = self.module.make_signature();
            // A closure's ABI is the env pointer plus the user
            // parameters (the capture parameters are env slots).
            if is_closure {
                sig.params.push(AbiParam::new(cltypes::I64));
            }
            for &param in program.function_parameters(function) {
                if is_closure
                    && matches!(
                        program
                            .local(param)
                            .expect("validated MIR retains locals")
                            .kind,
                        lpp_mir::MirLocalKind::Capture
                    )
                {
                    continue;
                }
                let ty = program
                    .local(param)
                    .expect("validated MIR retains locals")
                    .ty;
                sig.params
                    .push(AbiParam::new(check_value_type(types, ty, mir_id)?));
            }
            if let Some(cl) = machine_type(types, function.return_type) {
                sig.returns.push(AbiParam::new(cl));
            }
            let id = self
                .module
                .declare_function(name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare export {name}: {e:?}")))?;
            if name == "lpp_main" {
                self.lpp_main_id = Some(id);
                self.lpp_main_function_id = Some(mir_id);
            } else if name == "_start" && self.entry.is_none() {
                self.entry = Some("_start".to_owned());
            }
            self.func_ids.insert(mir_id, id);
            self.exported.insert(name.clone());
        }

        Ok(())
    }

    /// Lower every generated destructor in `MirAggregateId` order,
    /// then every generated cycle-collector trace likewise.
    pub(crate) fn lower_destructors(
        &mut self,
        types: &TypeInterner,
        plan: &Plan,
    ) -> Result<(), CodegenError> {
        for &aggregate in &plan.destructors {
            self.lower_destructor(types, aggregate)?;
        }
        for &aggregate in &plan.traces {
            self.lower_trace(types, aggregate)?;
        }
        for &ty in &plan.tuple_destructors {
            self.lower_tuple_destructor(types, ty)?;
        }
        for &ty in &plan.tuple_traces {
            self.lower_tuple_trace(types, ty)?;
        }
        Ok(())
    }

    /// `lpp_drop_t{n}(payload)` — release the tuple record's managed
    /// element slots, walking exactly the `tuple_layout` offsets the
    /// construction stored through.
    pub(super) fn lower_tuple_destructor(
        &mut self,
        types: &TypeInterner,
        ty: TypeId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .tuple_dtor_ids
            .get(&ty)
            .expect("declare ran before lowering");
        let (slots, _) = crate::layout::tuple_layout(types, ty)
            .expect("pre-scan collected constructed tuple types");

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(8, ty.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let payload = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            for (offset, element) in &slots {
                if is_managed(types, *element) {
                    let old =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), payload, *offset as i32);
                    self.arc_release(&mut builder, old);
                }
            }
            builder.ins().return_(&[]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(None, format!("define tuple destructor: {e:?}")))?;
        Ok(())
    }

    /// `lpp_trace_t{n}(payload, ctx)` — report the tuple record's
    /// managed element slots to the cycle collector (the
    /// `lpp_drop_t{n}` walk without the releases).
    pub(super) fn lower_tuple_trace(
        &mut self,
        types: &TypeInterner,
        ty: TypeId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .tuple_trace_ids
            .get(&ty)
            .expect("declare ran before lowering");
        let (slots, _) = crate::layout::tuple_layout(types, ty)
            .expect("pre-scan collected constructed tuple types");

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        sig.params.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(9, ty.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let payload = builder.append_block_param(entry, cltypes::I64);
            let trace_ctx = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            for (offset, element) in &slots {
                if is_managed(types, *element) {
                    let child =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), payload, *offset as i32);
                    self.trace_visit(&mut builder, trace_ctx, child);
                }
            }
            builder.ins().return_(&[]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(None, format!("define tuple trace: {e:?}")))?;
        Ok(())
    }

    /// `lpp_trace_{s|e}{n}(payload, ctx)` — report the payload's owned
    /// ARC children to the cycle collector, walking exactly the slots
    /// the matching destructor releases (same layout, same managed
    /// test). The collector's proof relies on the two walks agreeing.
    pub(super) fn lower_trace(
        &mut self,
        types: &TypeInterner,
        aggregate: MirAggregateId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .trace_ids
            .get(&aggregate)
            .expect("declare ran before lowering");
        let layout = self
            .layouts
            .get(&aggregate)
            .expect("pre-scan laid out every nominal")
            .clone();

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        sig.params.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(6, aggregate.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let payload = builder.append_block_param(entry, cltypes::I64);
            let trace_ctx = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);

            match layout.kind {
                lpp_mir::MirAggregateKind::Struct => {
                    for slot in &layout.struct_fields {
                        if is_managed(types, slot.ty) {
                            let child = builder.ins().load(
                                cltypes::I64,
                                MemFlags::new(),
                                payload,
                                slot.offset as i32,
                            );
                            self.trace_visit(&mut builder, trace_ctx, child);
                        }
                    }
                    builder.ins().return_(&[]);
                }
                lpp_mir::MirAggregateKind::Enum => {
                    let tag = builder
                        .ins()
                        .load(cltypes::I64, MemFlags::new(), payload, 0);
                    let variant_blocks: Vec<Block> = layout
                        .variant_fields
                        .iter()
                        .map(|_| builder.create_block())
                        .collect();
                    emit_tag_cascade(&mut builder, tag, &variant_blocks);
                    for (ordinal, slots) in layout.variant_fields.iter().enumerate() {
                        let variant_block = variant_blocks[ordinal];
                        builder.seal_block(variant_block);
                        builder.switch_to_block(variant_block);
                        for slot in slots {
                            if is_managed(types, slot.ty) {
                                let child = builder.ins().load(
                                    cltypes::I64,
                                    MemFlags::new(),
                                    payload,
                                    slot.offset as i32,
                                );
                                self.trace_visit(&mut builder, trace_ctx, child);
                            }
                        }
                        builder.ins().return_(&[]);
                    }
                }
            }
            builder.seal_all_blocks();
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(None, format!("define trace: {e:?}")))?;
        Ok(())
    }

    pub(super) fn lower_destructor(
        &mut self,
        types: &TypeInterner,
        aggregate: MirAggregateId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .dtor_ids
            .get(&aggregate)
            .expect("declare ran before lowering");
        let layout = self
            .layouts
            .get(&aggregate)
            .expect("pre-scan laid out every nominal")
            .clone();

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(2, aggregate.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let payload = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);

            match layout.kind {
                lpp_mir::MirAggregateKind::Struct => {
                    for slot in &layout.struct_fields {
                        if is_managed(types, slot.ty) {
                            let old = builder.ins().load(
                                cltypes::I64,
                                MemFlags::new(),
                                payload,
                                slot.offset as i32,
                            );
                            self.arc_release(&mut builder, old);
                        }
                    }
                    builder.ins().return_(&[]);
                }
                lpp_mir::MirAggregateKind::Enum => {
                    let tag = builder
                        .ins()
                        .load(cltypes::I64, MemFlags::new(), payload, 0);
                    let variant_blocks: Vec<Block> = layout
                        .variant_fields
                        .iter()
                        .map(|_| builder.create_block())
                        .collect();
                    // The cascade dispatches from the entry block while
                    // it is still current (cranelift forbids revisiting
                    // a filled block); the variant blocks are still
                    // pristine targets.
                    emit_tag_cascade(&mut builder, tag, &variant_blocks);
                    for (ordinal, slots) in layout.variant_fields.iter().enumerate() {
                        let variant_block = variant_blocks[ordinal];
                        builder.seal_block(variant_block);
                        builder.switch_to_block(variant_block);
                        for slot in slots {
                            if is_managed(types, slot.ty) {
                                let old = builder.ins().load(
                                    cltypes::I64,
                                    MemFlags::new(),
                                    payload,
                                    slot.offset as i32,
                                );
                                self.arc_release(&mut builder, old);
                            }
                        }
                        builder.ins().return_(&[]);
                    }
                }
            }
            builder.seal_all_blocks();
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(None, format!("define destructor: {e:?}")))?;
        Ok(())
    }

    /// Lower every function in `MirFunctionId` order, then the
    /// generated closure-env destructors and task/closure thunks in
    /// plan order.
    pub(crate) fn lower_functions_with_debug(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        plan: &Plan,
        names: &dyn lpp_codegen_api::NameResolver,
        debug_info: bool,
    ) -> Result<(), CodegenError> {
        for (mir_id, function) in program.functions() {
            let export_name = plan
                .export_names
                .get(&mir_id)
                .map(String::as_str)
                .unwrap_or("lpp_fn");
            let source_name = function
                .name
                .and_then(|s| names.resolve(s.raw()))
                .unwrap_or(export_name);
            self.lower_function(
                program,
                types,
                mir_id,
                function,
                export_name,
                source_name,
                debug_info,
            )?;
        }
        for &function_id in &plan.closure_dtors {
            self.lower_closure_dtor(program, types, function_id)?;
        }
        for &function_id in &plan.closure_traces {
            self.lower_closure_trace(program, types, function_id)?;
        }
        for &function_id in &plan.task_thunks {
            self.lower_task_thunk(program, types, function_id)?;
        }
        for &function_id in &plan.closure_thunks {
            self.lower_closure_thunk(program, types, function_id)?;
        }
        Ok(())
    }

    /// `lpp_drop_c{n}` — releases the closure env's managed capture
    /// slots (offsets `8*i`), in capture order.
    pub(super) fn lower_closure_dtor(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        function_id: MirFunctionId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .closure_dtor_ids
            .get(&function_id)
            .expect("declare ran before lowering");
        let function = program
            .function(function_id)
            .expect("validated MIR retains functions");
        let captures: Vec<(MirLocalId, TypeId)> = program
            .function_parameters(function)
            .iter()
            .filter(|local| {
                matches!(
                    program
                        .local(**local)
                        .expect("validated MIR retains locals")
                        .kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .map(|local| {
                (
                    *local,
                    program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .ty,
                )
            })
            .collect();

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(3, function_id.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let env = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            for (i, (_local, ty)) in captures.iter().enumerate() {
                if is_managed(types, *ty) {
                    let old =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), env, (8 * i) as i32);
                    self.arc_release(&mut builder, old);
                }
            }
            builder.ins().return_(&[]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(Some(function_id), format!("define closure dtor: {e:?}")))?;
        Ok(())
    }

    /// `lpp_trace_c{n}` — reports the closure env's managed capture
    /// slots to the cycle collector (the `lpp_drop_c{n}` walk without
    /// the releases).
    pub(super) fn lower_closure_trace(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        function_id: MirFunctionId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .closure_trace_ids
            .get(&function_id)
            .expect("declare ran before lowering");
        let function = program
            .function(function_id)
            .expect("validated MIR retains functions");
        let captures: Vec<TypeId> = program
            .function_parameters(function)
            .iter()
            .filter(|local| {
                matches!(
                    program
                        .local(**local)
                        .expect("validated MIR retains locals")
                        .kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .map(|local| {
                program
                    .local(*local)
                    .expect("validated MIR retains locals")
                    .ty
            })
            .collect();

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        sig.params.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(7, function_id.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let env = builder.append_block_param(entry, cltypes::I64);
            let trace_ctx = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            for (i, ty) in captures.iter().enumerate() {
                if is_managed(types, *ty) {
                    let child =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), env, (8 * i) as i32);
                    self.trace_visit(&mut builder, trace_ctx, child);
                }
            }
            builder.ins().return_(&[]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| {
                verify_failed(Some(function_id), format!("define closure trace: {e:?}"))
            })?;
        Ok(())
    }

    /// `__lpp_task_thunk{n}` — the task code for async function
    /// `n`: loads the argument slots from the task env tuple
    /// (offsets `16 + 8*i`), retains the managed ones, calls the
    /// function, and boxes the result.
    pub(super) fn lower_task_thunk(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        function_id: MirFunctionId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .task_thunk_ids
            .get(&function_id)
            .expect("declare ran before lowering");
        let function = program
            .function(function_id)
            .expect("validated MIR retains functions");

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        sig.returns.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(4, function_id.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let env = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            let mut args: Vec<Value> = Vec::new();
            for (i, &param) in user_parameters(program, function).iter().enumerate() {
                let ty = program
                    .local(param)
                    .expect("validated MIR retains locals")
                    .ty;
                let mut v =
                    builder
                        .ins()
                        .load(cltypes::I64, MemFlags::new(), env, (16 + 8 * i) as i32);
                if is_managed(types, ty) {
                    v = self.arc_retain_value(&mut builder, v);
                }
                args.push(v);
            }
            let callee_ref = self
                .module
                .declare_func_in_func(self.func_ids[&function_id], builder.func);
            let call = builder.ins().call(callee_ref, &args);
            let result = machine_type(types, function.return_type)
                .is_some()
                .then(|| builder.func.dfg.first_result(call));
            let boxed = box_result(types, &mut builder, result, function.return_type);
            builder.ins().return_(&[boxed]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(Some(function_id), format!("define task thunk: {e:?}")))?;
        Ok(())
    }

    /// `__lpp_closure_thunk{n}` — the task code for the
    /// zero-user-parameter closure `n`: the task env slot holds the
    /// closure env (offset `16`); invoke the closure and box the
    /// result.
    pub(super) fn lower_closure_thunk(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        function_id: MirFunctionId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .closure_thunk_ids
            .get(&function_id)
            .expect("declare ran before lowering");
        let function = program
            .function(function_id)
            .expect("validated MIR retains functions");

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        sig.returns.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(5, function_id.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            // The thunk's single parameter is the capsule env: a
            // 1-slot wrapper tuple whose slot 16 holds the raw closure
            // env (a NULL slot for zero-capture closures), so the same
            // code pointer serves the direct call and the spawned task.
            let wrapper = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            let closure_env = builder
                .ins()
                .load(cltypes::I64, MemFlags::new(), wrapper, 16);
            let callee_ref = self
                .module
                .declare_func_in_func(self.func_ids[&function_id], builder.func);
            let call = builder.ins().call(callee_ref, &[closure_env]);
            let result = machine_type(types, function.return_type)
                .is_some()
                .then(|| builder.func.dfg.first_result(call));
            let boxed = box_result(types, &mut builder, result, function.return_type);
            builder.ins().return_(&[boxed]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| {
                verify_failed(Some(function_id), format!("define closure thunk: {e:?}"))
            })?;
        Ok(())
    }
}
