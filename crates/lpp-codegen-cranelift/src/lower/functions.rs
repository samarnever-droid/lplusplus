use super::*;

impl<'m> Lowering<'m> {
    pub(super) fn lower_function(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        mir_id: MirFunctionId,
        function: &MirFunction,
        export_name: &str,
        source_name: &str,
        debug_info: bool,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .func_ids
            .get(&mir_id)
            .expect("declare ran before lowering");

        let is_closure = matches!(function.kind, MirFunctionKind::Closure);
        let capture_locals: Vec<MirLocalId> = program
            .function_parameters(function)
            .iter()
            .copied()
            .filter(|local| {
                matches!(
                    program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .collect();
        let mut capture_slots: HashMap<MirLocalId, u32> = HashMap::new();
        for (i, &local) in capture_locals.iter().enumerate() {
            capture_slots.insert(local, i as u32);
        }

        let mut sig = self.module.make_signature();
        if is_closure {
            sig.params.push(AbiParam::new(cltypes::I64));
        }
        for &param in program.function_parameters(function) {
            if is_closure && capture_locals.contains(&param) {
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

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(0, mir_id.raw());
        let mut fn_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);

            // Every MIR local is a cranelift variable. Soundness comes
            // from the 4B definite-initialization invariant: a
            // variable is only read where a def dominates.
            let locals = program.function_locals(function);
            let mut local_vars: HashMap<MirLocalId, Variable> = HashMap::new();
            for (index, &local) in locals.iter().enumerate() {
                let ty = program
                    .local(local)
                    .expect("validated MIR retains locals")
                    .ty;
                let cl = machine_type(types, ty).unwrap_or(cltypes::I8);
                let variable = Variable::new(index);
                builder.declare_var(variable, cl);
                local_vars.insert(local, variable);
            }

            let mut cl_blocks: HashMap<BasicBlockId, Block> = HashMap::new();
            for &block in program.function_blocks(function) {
                cl_blocks.insert(block, builder.create_block());
            }

            let entry = *cl_blocks
                .get(&function.entry)
                .expect("every function has its entry block");
            builder.switch_to_block(entry);
            builder.append_block_params_for_function_params(entry);
            let param_vals: Vec<Value> = builder.block_params(entry).to_vec();
            let params = program.function_parameters(function);
            let env_var = if is_closure {
                let env_val = param_vals[0];
                let variable = Variable::new(locals.len());
                builder.declare_var(variable, cltypes::I64);
                builder.def_var(variable, env_val);
                for (i, &local) in capture_locals.iter().enumerate() {
                    let view =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), env_val, (8 * i) as i32);
                    let variable = *local_vars
                        .get(&local)
                        .expect("capture parameters are locals");
                    builder.def_var(variable, view);
                }
                Some(variable)
            } else {
                None
            };
            let mut user_index = 0usize;
            for &param in params {
                if is_closure && capture_locals.contains(&param) {
                    continue;
                }
                let variable = *local_vars.get(&param).expect("parameters are locals");
                builder.def_var(
                    variable,
                    param_vals[if is_closure { 1 } else { 0 } + user_index],
                );
                user_index += 1;
            }

            // Zero-init every non-parameter value local; the 4B
            // invariant keeps the reads sound. Non-escaping `Frame` POD
            // structs/tuples sit directly in a sized `StackSlot` (0 B heap,
            // 0 atomics); heap-managed locals zero to null.
            for &local in locals {
                if !params.contains(&local) {
                    let variable = *local_vars.get(&local).expect("local variable");
                    if let Some(&stack_bytes) = self.stack_locals.get(&(mir_id, local)) {
                        let slot = builder.func.create_sized_stack_slot(StackSlotData {
                            kind: StackSlotKind::ExplicitSlot,
                            size: stack_bytes,
                            align_shift: 3,
                        });
                        let addr = builder.ins().stack_addr(cltypes::I64, slot, 0);
                        builder.def_var(variable, addr);
                        continue;
                    }
                    let ty = program
                        .local(local)
                        .expect("validated MIR retains locals")
                        .ty;
                    if let Some(cl) = machine_type(types, ty) {
                        let zero = match cl {
                            cltypes::I8 => builder.ins().iconst(cltypes::I8, 0),
                            cltypes::I16 => builder.ins().iconst(cltypes::I16, 0),
                            cltypes::I32 => builder.ins().iconst(cltypes::I32, 0),
                            cltypes::I64 => builder.ins().iconst(cltypes::I64, 0),
                            cltypes::F64 => builder.ins().f64const(0.0),
                            cltypes::I64X2 => {
                                let zero_lane = builder.ins().iconst(cltypes::I64, 0);
                                builder.ins().splat(cltypes::I64X2, zero_lane)
                            }
                            _ => unreachable!("slice machine types"),
                        };
                        builder.def_var(variable, zero);
                    }
                }
            }

            if self.pgo_fingerprint.is_some() {
                let pgo_ref = self.import_ref(&mut builder, IMP_PGO_HIT);
                let function_id = builder.ins().iconst(cltypes::I64, i64::from(mir_id.raw()));
                builder.ins().call(pgo_ref, &[function_id]);
            }

            // The entry block is lowered first (it already holds the
            // parameter defs and zero inits); cranelift forbids
            // switching back to a block that is not pristine. Every
            // other block is lowered in `MirFunctionId`-block order.
            let blocks = program.function_blocks(function);
            let inst = Inst {
                program,
                types,
                local_vars: &local_vars,
                env_var,
                capture_slots: &capture_slots,
                mir_id,
                function,
            };
            // The cursor is already at the entry block (it holds the
            // parameter defs and zero inits), so its body lowers
            // without switching; every other block switches first.
            self.lower_block(&inst, &mut builder, &cl_blocks, function.entry, false)?;
            for &block in blocks {
                if block == function.entry {
                    continue;
                }
                self.lower_block(&inst, &mut builder, &cl_blocks, block, true)?;
            }

            builder.seal_all_blocks();
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(Some(mir_id), format!("define function: {e:?}")))?;

        if debug_info && let Some(compiled) = ctx.compiled_code() {
            let srclocs = compiled
                .buffer
                .get_srclocs_sorted()
                .iter()
                .map(|m| (m.start, m.loc))
                .collect();
            self.debug_records
                .push(crate::dwarf::CompiledDebugFunction {
                    export_name: export_name.to_string(),
                    source_name: source_name.to_string(),
                    origin_raw: function.origin.raw(),
                    code_len: compiled.buffer.data().len() as u32,
                    srclocs,
                });
        }

        Ok(())
    }

    pub(super) fn lower_block(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        cl_blocks: &HashMap<BasicBlockId, Block>,
        block: BasicBlockId,
        switch: bool,
    ) -> Result<(), CodegenError> {
        let cl_block = *cl_blocks.get(&block).expect("block mapping");
        let basic = inst
            .program
            .block(block)
            .expect("validated MIR retains blocks");
        if switch {
            builder.switch_to_block(cl_block);
        }
        for &instruction in inst.program.block_instructions(basic) {
            let instr = inst
                .program
                .instruction(instruction)
                .expect("validated MIR retains instructions");
            builder.set_srcloc(cranelift_codegen::ir::SourceLoc::new(instr.origin.raw()));
            self.lower_instr(inst, builder, instr)?;
        }
        builder.set_srcloc(cranelift_codegen::ir::SourceLoc::new(basic.origin.raw()));
        self.lower_terminator(inst, builder, &basic.terminator, cl_blocks)
    }

    pub(super) fn lower_instr(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        instruction: &lpp_mir::Instruction,
    ) -> Result<(), CodegenError> {
        match &instruction.kind {
            InstructionKind::Store { place, value } => {
                self.lower_store(inst, builder, *place, value)
            }
            InstructionKind::Assign { target, value } => {
                self.lower_assign(inst, builder, *target, value)
            }
        }
    }

    /// A value operand that becomes an owner: a `Copy` of a managed
    /// local retains for the new owner (the source keeps its own
    /// reference and releases it on overwrite or exit).
    pub(super) fn owned_value(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        operand: &Operand,
    ) -> Result<Value, CodegenError> {
        let value = self.operand_value(inst, builder, operand)?;
        if let Operand::Copy(local) = operand {
            let ty = inst
                .program
                .local(*local)
                .expect("validated MIR retains every local")
                .ty;
            if is_managed(inst.types, ty) {
                self.arc_retain(builder, value);
            }
        }
        Ok(value)
    }

    /// Walk the projection chain up to (but excluding) the final step.
    /// Intermediate steps always yield a base pointer: a struct field
    /// load or an ARC-list element read.
    pub(super) fn place_chain_base(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        resolution: &PlaceResolution,
    ) -> Result<Value, CodegenError> {
        let mut base = {
            let variable = *inst
                .local_vars
                .get(&resolution.root)
                .expect("validated MIR retains local variables");
            builder.use_var(variable)
        };
        for step in resolution.steps.iter().take(resolution.steps.len() - 1) {
            base = match step {
                PlaceStep::Field { offset, .. } => {
                    builder
                        .ins()
                        .load(cltypes::I64, MemFlags::new(), base, *offset as i32)
                }
                PlaceStep::ListIndex { index, .. } => {
                    let idx = self.operand_value(inst, builder, index)?;
                    let func_ref = self.import_ref(builder, IMP_LIST_GET_ARC);
                    let call = builder.ins().call(func_ref, &[base, idx]);
                    builder.func.dfg.first_result(call)
                }
            };
        }
        Ok(base)
    }

    pub(super) fn lower_store(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        place: lpp_mir::MirPlaceId,
        value: &Operand,
    ) -> Result<(), CodegenError> {
        let resolution = resolve_place(
            inst.program,
            inst.types,
            place,
            &self.aggregates,
            &self.layouts,
            inst.mir_id,
            true,
        )?;

        if resolution.steps.is_empty() {
            // Bare local: cell death, then the new value lands. A
            // `Copy` source retains for the new owner first (the
            // interpreter's order: the rvalue completes before the
            // old value dies).
            //
            // A capture local is a *view* of the env slot: the slot
            // owns the reference, the local holds none. The writeback
            // below performs the single old-value release; releasing
            // the local's value here as well would double-free the
            // captured object (the env slot and the view alias one
            // reference).
            let is_capture = inst.capture_slots.contains_key(&resolution.root);
            let new_value = self.owned_value(inst, builder, value)?;
            let variable = *inst
                .local_vars
                .get(&resolution.root)
                .expect("validated MIR retains local variables");
            if is_managed(inst.types, resolution.ty) && !is_capture {
                let old = builder.use_var(variable);
                self.arc_release(builder, old);
            }
            builder.def_var(variable, new_value);
            // A capture store is the env-slot writeback: the env slot
            // is the reference owner, the local is a view refreshed to
            // the stored value (retain-then-release order, as above).
            if let Some(&slot) = inst.capture_slots.get(&resolution.root) {
                let env = builder.use_var(inst.env_var.expect("closure env"));
                if is_managed(inst.types, resolution.ty) {
                    let old =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), env, 8 * slot as i32);
                    self.arc_release(builder, old);
                }
                builder
                    .ins()
                    .store(MemFlags::new(), new_value, env, 8 * slot as i32);
            }
            return Ok(());
        }

        match resolution.steps.last().expect("non-empty projections") {
            PlaceStep::Field {
                offset,
                ty,
                managed,
                ..
            } => {
                let base = self.place_chain_base(inst, builder, &resolution)?;
                let new_value = self.owned_value(inst, builder, value)?;
                if *managed {
                    let old =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), base, *offset as i32);
                    self.arc_release(builder, old);
                }
                let field_type = machine_type(inst.types, *ty).expect("checked by the pre-scan");
                let stored = coerce_to(builder, new_value, field_type);
                builder
                    .ins()
                    .store(MemFlags::new(), stored, base, *offset as i32);
                Ok(())
            }
            PlaceStep::ListIndex { element, .. } => {
                let base = self.place_chain_base(inst, builder, &resolution)?;
                let idx = {
                    let index = resolution.steps.last().expect("non-empty projections");
                    let PlaceStep::ListIndex { index, .. } = index else {
                        unreachable!("list index step")
                    };
                    self.operand_value(inst, builder, index)?
                };
                match element_class(inst.types, *element) {
                    ElementClass::Managed => {
                        // lpp_list_set_arc retains the new element and
                        // releases the replaced one (the interpreter's
                        // store_place, runtime-internal).
                        let new_value = self.operand_value(inst, builder, value)?;
                        let func_ref = self.import_ref(builder, IMP_LIST_SET_ARC);
                        builder.ins().call(func_ref, &[base, idx, new_value]);
                    }
                    ElementClass::Scalar => {
                        let raw = self.operand_value(inst, builder, value)?;
                        let stored = coerce_to(builder, raw, cltypes::I64);
                        let func_ref = self.import_ref(builder, IMP_LIST_SET);
                        builder.ins().call(func_ref, &[base, idx, stored]);
                    }
                    ElementClass::Float => {
                        let raw = self.operand_value(inst, builder, value)?;
                        let stored = coerce_to(builder, raw, cltypes::F64);
                        let func_ref = self.import_ref(builder, IMP_LIST_SET_FLOAT);
                        builder.ins().call(func_ref, &[base, idx, stored]);
                    }
                    ElementClass::Bool => {
                        let raw = self.operand_value(inst, builder, value)?;
                        let stored = coerce_to(builder, raw, cltypes::I8);
                        let func_ref = self.import_ref(builder, IMP_LIST_SET_BOOL);
                        builder.ins().call(func_ref, &[base, idx, stored]);
                    }
                }
                Ok(())
            }
        }
    }
}
