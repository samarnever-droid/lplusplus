use super::*;

impl<'m> Lowering<'m> {
    /// The generated C-ABI `main` wrapper: call `lpp_main` and forward its
    /// integer result to the OS as the process exit status. A `Void` (or
    /// non-`Int`-returning) `main` exits with status 0.
    pub(crate) fn lower_main_wrapper(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
    ) -> Result<(), CodegenError> {
        let Some(lpp_main) = self.lpp_main_id else {
            return Ok(());
        };
        let Some(main_fn_id) = self.lpp_main_function_id else {
            return Ok(());
        };
        let main_fn = program
            .function(main_fn_id)
            .expect("the main declaration retained its function");

        // `main() -> Int` forwards its return value to the OS process exit
        // code; news user code that returns the status explicitly behaves
        // like every C program.
        let forward_exit_code =
            matches!(machine_type(types, main_fn.return_type), Some(cltypes::I64));

        let mut sig = self.module.make_signature();
        sig.returns.push(AbiParam::new(cltypes::I32));
        let main_id = self
            .module
            .declare_function("main", Linkage::Export, &sig)
            .map_err(|e| emission_failed(format!("declare export main: {e:?}")))?;
        self.exported.insert("main".to_owned());
        self.entry = Some("main".to_owned());

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(1, 0);
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            builder.switch_to_block(entry);
            let entry_code: Option<Value> = if matches!(main_fn.kind, MirFunctionKind::Async) {
                // The async entry is drained here — the execution
                // boundary holds no references: empty env tuple,
                // task, await, destroy.
                let tuple_ref = self.import_ref(&mut builder, IMP_TUPLE_ALLOC);
                let size = builder.ins().iconst(cltypes::I64, 16);
                let zero = builder.ins().iconst(cltypes::I64, 0);
                let call = builder.ins().call(tuple_ref, &[size, zero, zero]);
                let env = builder.func.dfg.first_result(call);
                let thunk_ref = self
                    .module
                    .declare_func_in_func(self.task_thunk_ids[&main_fn_id], builder.func);
                let code = builder.ins().func_addr(cltypes::I64, thunk_ref);
                let task = self.task_new(
                    &mut builder,
                    code,
                    env,
                    Self::task_managed_flag(types, main_fn.return_type),
                );
                let await_ref = self.import_ref(&mut builder, IMP_TASK_AWAIT);
                let await_call = builder.ins().call(await_ref, &[task]);
                let result = builder.func.dfg.first_result(await_call);
                let destroy_ref = self.import_ref(&mut builder, IMP_TASK_DESTROY);
                builder.ins().call(destroy_ref, &[task]);
                if forward_exit_code {
                    Some(result)
                } else {
                    None
                }
            } else {
                let main_ref = self.module.declare_func_in_func(lpp_main, builder.func);
                let call = builder.ins().call(main_ref, &[]);
                if forward_exit_code {
                    Some(builder.func.dfg.first_result(call))
                } else {
                    None
                }
            };
            if !self.shared_types.is_empty() {
                // Reclaim any cyclic garbage still buffered below the
                // collector's threshold so the process exits leak-free.
                let collect_ref = self.import_ref(&mut builder, IMP_COLLECT_CYCLES);
                builder.ins().call(collect_ref, &[]);
            }
            if let Some(fingerprint) = self.pgo_fingerprint {
                let flush_ref = self.import_ref(&mut builder, IMP_PGO_FLUSH);
                let fingerprint = builder.ins().iconst(cltypes::I64, fingerprint as i64);
                builder.ins().call(flush_ref, &[fingerprint]);
            }
            let status = match entry_code {
                Some(code) => builder.ins().ireduce(cltypes::I32, code),
                None => builder.ins().iconst(cltypes::I32, 0),
            };
            builder.ins().return_(&[status]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(main_id, &mut ctx)
            .map_err(|e| verify_failed(None, format!("define main wrapper: {e:?}")))?;

        Ok(())
    }

    // ── ARC helpers ────────────────────────────────────────────────────────

    pub(super) fn import_ref(
        &mut self,
        builder: &mut FunctionBuilder,
        name: &'static str,
    ) -> FuncRef {
        let id = *self
            .import_ids
            .get(name)
            .unwrap_or_else(|| panic!("pre_scan did not declare import {name}"));
        self.module.declare_func_in_func(id, builder.func)
    }

    /// A new owner is created: `lpp_arc_retain(ptr)`.
    pub(super) fn arc_retain(&mut self, builder: &mut FunctionBuilder, value: Value) {
        let func_ref = self.import_ref(builder, IMP_ARC_RETAIN);
        builder.ins().call(func_ref, &[value]);
    }

    /// Report one owned child during a generated trace:
    /// `lpp_trace_visit(ctx, child)`.
    pub(super) fn trace_visit(&mut self, builder: &mut FunctionBuilder, ctx: Value, child: Value) {
        let func_ref = self.import_ref(builder, IMP_TRACE_VISIT);
        builder.ins().call(func_ref, &[ctx, child]);
    }

    /// Register a fresh `Shared` allocation with the runtime cycle
    /// collector, using a compiler-generated trace function.
    pub(super) fn mark_collectible_generated(
        &mut self,
        builder: &mut FunctionBuilder,
        ptr: Value,
        trace_id: FuncId,
    ) {
        let trace_ref = self.module.declare_func_in_func(trace_id, builder.func);
        let trace_addr = builder.ins().func_addr(cltypes::I64, trace_ref);
        let func_ref = self.import_ref(builder, IMP_MARK_COLLECTIBLE);
        builder.ins().call(func_ref, &[ptr, trace_addr]);
    }

    /// Register a fresh `Shared` allocation with one of the runtime's
    /// own trace functions (lists, tuples, tasks, closure capsules).
    pub(super) fn mark_collectible_import(
        &mut self,
        builder: &mut FunctionBuilder,
        ptr: Value,
        trace_import: &'static str,
    ) {
        let trace_ref = self.import_ref(builder, trace_import);
        let trace_addr = builder.ins().func_addr(cltypes::I64, trace_ref);
        let func_ref = self.import_ref(builder, IMP_MARK_COLLECTIBLE);
        builder.ins().call(func_ref, &[ptr, trace_addr]);
    }

    /// An owning slot dies: `lpp_arc_release(ptr)` (null-safe and
    /// immortal-safe in the v1 runtime, so the call is unconditional).
    pub(super) fn arc_release(&mut self, builder: &mut FunctionBuilder, value: Value) {
        let func_ref = self.import_ref(builder, IMP_ARC_RELEASE);
        builder.ins().call(func_ref, &[value]);
    }

    /// Allocate the ARC block for a structural tuple of type `ty`:
    /// POD `Owned` tuples pass a null destructor (omitting the 8-byte prefix
    /// and indirect destructor call); tuples containing managed slots pass
    /// `lpp_drop_t{n}` and `Shared` tuples are additionally marked collectible.
    pub(super) fn alloc_tuple(
        &mut self,
        types: &TypeInterner,
        builder: &mut FunctionBuilder,
        size: u32,
        ty: TypeId,
    ) -> Value {
        let func_ref = self.import_ref(builder, IMP_ARC_ALLOC);
        let size_value = builder.ins().iconst(cltypes::I64, size as i64);
        let dtor_addr = if !self.tuple_trace_ids.contains_key(&ty)
            && lpp_codegen_api::is_pod_tuple(types, ty)
        {
            builder.ins().iconst(cltypes::I64, 0)
        } else {
            let dtor_ref = self
                .module
                .declare_func_in_func(self.tuple_dtor_ids[&ty], builder.func);
            builder.ins().func_addr(cltypes::I64, dtor_ref)
        };
        let call = builder.ins().call(func_ref, &[size_value, dtor_addr]);
        let ptr = builder.func.dfg.first_result(call);
        if let Some(&trace_id) = self.tuple_trace_ids.get(&ty) {
            self.mark_collectible_generated(builder, ptr, trace_id);
        }
        ptr
    }

    pub(super) fn arc_alloc(
        &mut self,
        types: &TypeInterner,
        builder: &mut FunctionBuilder,
        size: u32,
        aggregate: MirAggregateId,
    ) -> Value {
        let func_ref = self.import_ref(builder, IMP_ARC_ALLOC);
        let size_value = builder.ins().iconst(cltypes::I64, size as i64);
        let is_pod_owned = !self.trace_ids.contains_key(&aggregate)
            && self
                .layouts
                .get(&aggregate)
                .is_some_and(|layout| lpp_codegen_api::is_pod_aggregate(layout, types));
        let dtor_addr = if is_pod_owned {
            builder.ins().iconst(cltypes::I64, 0)
        } else {
            let dtor_ref = self
                .module
                .declare_func_in_func(self.dtor_ids[&aggregate], builder.func);
            builder.ins().func_addr(cltypes::I64, dtor_ref)
        };
        let call = builder.ins().call(func_ref, &[size_value, dtor_addr]);
        let ptr = builder.func.dfg.first_result(call);
        if let Some(&trace_id) = self.trace_ids.get(&aggregate) {
            self.mark_collectible_generated(builder, ptr, trace_id);
        }
        ptr
    }

    /// The exit release pass: every heap-managed local of the function, in
    /// declaration order (the interpreter's rule at `Return`), skipping
    /// stack-allocated `Frame` locals.
    pub(super) fn exit_release_pass(&mut self, inst: &Inst, builder: &mut FunctionBuilder) {
        for &local in inst.program.function_locals(inst.function) {
            let descriptor = inst
                .program
                .local(local)
                .expect("validated MIR retains locals");
            if matches!(descriptor.kind, lpp_mir::MirLocalKind::Capture) {
                continue;
            }
            if self.stack_locals.contains_key(&(inst.mir_id, local)) {
                continue;
            }
            if is_managed(inst.types, descriptor.ty) {
                let variable = *inst.local_vars.get(&local).expect("local variable");
                let value = builder.use_var(variable);
                self.arc_release(builder, value);
            }
        }
    }

    // ── inst.function lowering ─────────────────────────────────────────────────
}
