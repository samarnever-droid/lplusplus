use super::*;

impl<'m> Lowering<'m> {
    /// `lpp_arc_retain(value)` — the v1 ABI returns void, so the
    /// retained value is the input itself.
    pub(super) fn arc_retain_value(
        &mut self,
        builder: &mut FunctionBuilder,
        value: Value,
    ) -> Value {
        self.arc_retain(builder, value);
        value
    }

    /// `lpp_arc_alloc_with_destructor(size, dtor)` with a raw
    /// destructor function reference (the aggregate dtors, the
    /// closure capsule's `lpp_closure_destroy`).
    pub(super) fn arc_alloc_ptr(
        &mut self,
        builder: &mut FunctionBuilder,
        size: u32,
        dtor_ref: FuncRef,
    ) -> Value {
        let func_ref = self.import_ref(builder, IMP_ARC_ALLOC);
        let size_value = builder.ins().iconst(cltypes::I64, size as i64);
        let dtor_addr = builder.ins().func_addr(cltypes::I64, dtor_ref);
        let call = builder.ins().call(func_ref, &[size_value, dtor_addr]);
        builder.func.dfg.first_result(call)
    }

    /// The task result `managed` flag: `1` when the user result
    /// behind a `Task` return type is ARC-managed.
    pub(super) fn task_managed_flag(types: &TypeInterner, return_type: TypeId) -> i64 {
        let inner = match types.kind(return_type) {
            TypeKind::Task(inner) => inner,
            _ => return_type,
        };
        if is_managed(types, inner) { 1 } else { 0 }
    }

    /// `lpp_task_new(code, env, managed)`.
    pub(super) fn task_new(
        &mut self,
        builder: &mut FunctionBuilder,
        code: Value,
        env: Value,
        managed: i64,
    ) -> Value {
        let func_ref = self.import_ref(builder, IMP_TASK_NEW);
        let managed_value = builder.ins().iconst(cltypes::I64, managed);
        let call = builder.ins().call(func_ref, &[code, env, managed_value]);
        builder.func.dfg.first_result(call)
    }

    /// The task env tuple for a call's arguments:
    /// `lpp_tuple_alloc(16 + 8*n, mask, packed_offsets)` with each
    /// argument stored at `16 + 8*i` (managed arguments retained for
    /// the tuple, which owns them).
    pub(super) fn task_env_for_args(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        args_ops: &[Operand],
        collectible: bool,
    ) -> Result<Value, CodegenError> {
        let mut mask = 0i64;
        let mut offsets = 0i64;
        for (i, operand) in args_ops.iter().enumerate() {
            if let Operand::Copy(local) = operand
                && is_managed(
                    inst.types,
                    inst.program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .ty,
                )
            {
                mask |= 1i64 << i;
            }
            offsets |= ((16 + 8 * i) as i64) << (16 * i as i64);
        }
        let size = 16 + 8 * args_ops.len() as i64;
        let func_ref = self.import_ref(builder, IMP_TUPLE_ALLOC);
        let size_value = builder.ins().iconst(cltypes::I64, size);
        let mask_value = builder.ins().iconst(cltypes::I64, mask);
        let offsets_value = builder.ins().iconst(cltypes::I64, offsets);
        let call = builder
            .ins()
            .call(func_ref, &[size_value, mask_value, offsets_value]);
        let env = builder.func.dfg.first_result(call);
        for (i, operand) in args_ops.iter().enumerate() {
            let value = self.owned_value(inst, builder, operand)?;
            builder
                .ins()
                .store(MemFlags::new(), value, env, (16 + 8 * i) as i32);
        }
        if collectible {
            // The task's trace reaches its arguments through this
            // tuple, so the tuple itself must be traversable.
            self.mark_collectible_import(builder, env, IMP_TUPLE_TRACE);
        }
        Ok(env)
    }

    /// `Use(Operand::Function)` materialization: the 16-byte capsule
    /// `[code, env]`. A sync value points at `lpp_f` with a `NULL`
    /// env; an async value points at its task thunk with an empty
    /// task-env tuple (spawned values need a non-NULL env).
    pub(super) fn materialize_function_value(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        function_id: MirFunctionId,
    ) -> Result<Value, CodegenError> {
        let function = inst
            .program
            .function(function_id)
            .expect("validated MIR retains functions");
        let dtor_id = *self
            .import_ids
            .get(IMP_CLOSURE_DESTROY)
            .expect("pre-scan declared lpp_closure_destroy");
        let dtor_ref = self.module.declare_func_in_func(dtor_id, builder.func);
        let capsule = self.arc_alloc_ptr(builder, 16, dtor_ref);
        let (code_ref, env) = match function.kind {
            MirFunctionKind::Async => {
                let thunk_ref = self
                    .module
                    .declare_func_in_func(self.task_thunk_ids[&function_id], builder.func);
                let code = builder.ins().func_addr(cltypes::I64, thunk_ref);
                let tuple_ref = self.import_ref(builder, IMP_TUPLE_ALLOC);
                let zero = builder.ins().iconst(cltypes::I64, 0);
                let size = builder.ins().iconst(cltypes::I64, 16);
                let call = builder.ins().call(tuple_ref, &[size, zero, zero]);
                let env = builder.func.dfg.first_result(call);
                (code, env)
            }
            MirFunctionKind::Function => {
                let fn_ref = self
                    .module
                    .declare_func_in_func(self.func_ids[&function_id], builder.func);
                let code = builder.ins().func_addr(cltypes::I64, fn_ref);
                (code, builder.ins().iconst(cltypes::I64, 0))
            }
            MirFunctionKind::Closure => {
                return Err(unsupported(inst.mir_id, "closure as value operand"));
            }
        };
        builder.ins().store(MemFlags::new(), code_ref, capsule, 0);
        builder.ins().store(MemFlags::new(), env, capsule, 8);
        if self.shared_types.contains(&function.ty) {
            self.mark_collectible_import(builder, capsule, IMP_CLOSURE_TRACE);
        }
        Ok(capsule)
    }

    /// The 5C2 builtin table policy beyond the 5C arms: `print`
    /// dispatch, Family A (import of the descriptor's v1 symbol),
    /// Family B (native 128-bit SIMD, the scalar checksum imported),
    /// and Family C (slice handles over the v1 slice runtime).
    pub(super) fn lower_builtin_families(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        builtin: BuiltinId,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        let descriptor = builtin.descriptor();
        let name = descriptor.name;

        // Family A: `print` dispatches on the argument's semantic
        // type; everything else imports the descriptor's v1 symbol
        // (arguments are plain reads — the v1 C functions never take
        // ownership of their string/list arguments).
        if name == "print" {
            // Family A `print` dispatches on the argument's semantic type.
            // A char argument lowers to i32 but the v1 print-int symbol
            // carries the ABI's i64 int, so it is widened before the call
            // (int arguments are already i64).
            let (printer, is_char) = match args.first().expect("print argument") {
                Operand::Constant(constant) => match constant {
                    Constant::Integer(_) => (IMP_PRINT_INT, false),
                    Constant::Character { .. } => (IMP_PRINT_INT, true),
                    Constant::FloatBits(_) => (IMP_PRINT_FLOAT, false),
                    Constant::Bool(_) => (IMP_PRINT_BOOL, false),
                    Constant::String { .. } => (IMP_PRINT_STR, false),
                },
                Operand::Copy(local) => match inst.types.kind(
                    inst.program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .ty,
                ) {
                    TypeKind::Primitive(PrimitiveType::Int) => (IMP_PRINT_INT, false),
                    TypeKind::Primitive(PrimitiveType::Char) => (IMP_PRINT_INT, true),
                    TypeKind::Primitive(PrimitiveType::Float) => (IMP_PRINT_FLOAT, false),
                    TypeKind::Primitive(PrimitiveType::Bool) => (IMP_PRINT_BOOL, false),
                    TypeKind::Primitive(PrimitiveType::String) => (IMP_PRINT_STR, false),
                    _ => return Err(unsupported(inst.mir_id, "print of unsupported argument")),
                },
                _ => return Err(unsupported(inst.mir_id, "print of non-local argument")),
            };
            let mut value =
                self.operand_value(inst, builder, args.first().expect("print argument"))?;
            if is_char {
                value = builder.ins().uextend(cltypes::I64, value);
            }
            let func_ref = self.import_ref(builder, printer);
            builder.ins().call(func_ref, &[value]);
            // print consumes its value: the reference dies with it.
            self.empty_moved_local(inst, builder, args.first().expect("print argument"));
            return Ok(None);
        }
        if name == "list_push" || name == "list_set" {
            return self.lower_list_mutation(inst, builder, name, args);
        }
        if name == "list_get" || name == "get" {
            return self.lower_list_get(inst, builder, args);
        }
        if is_family_a(name) {
            let symbol = descriptor.symbol;
            let mut call_args: Vec<Value> = Vec::with_capacity(args.len());
            for operand in args {
                call_args.push(self.operand_value(inst, builder, operand)?);
            }
            let func_ref = self.import_ref(builder, symbol);
            let call = builder.ins().call(func_ref, &call_args);
            if name == "write_str" {
                // write_str consumes its value: the reference dies with it.
                self.empty_moved_local(inst, builder, args.first().expect("write_str argument"));
            }
            let has_result =
                !matches!(descriptor.result, lpp_runtime_abi::generated::AbiType::Void);
            let result = has_result.then(|| builder.func.dfg.first_result(call));
            return Ok(match result {
                // The v1 C predicates return an `i64` 0/1, but the
                // language result is a `bool` (the oracle yields a
                // `Bool`): reduce to the machine bool.
                Some(raw)
                    if matches!(
                        descriptor.semantic_result,
                        lpp_runtime_abi::generated::SemanticAbiType::Bool
                    ) =>
                {
                    Some(builder.ins().ireduce(cltypes::I8, raw))
                }
                other => other,
            });
        }
        if name.starts_with("vec_") {
            return self.lower_simd(inst, builder, builtin, name, args);
        }
        if FAMILY_C.contains(&name) {
            return self.lower_slice(inst, builder, builtin, name, args);
        }
        if is_family_map(name) {
            let symbol = descriptor.symbol;
            let mut call_args: Vec<Value> = Vec::with_capacity(args.len());
            for operand in args {
                call_args.push(self.operand_value(inst, builder, operand)?);
            }
            let func_ref = self.import_ref(builder, symbol);
            let call = builder.ins().call(func_ref, &call_args);
            let has_result =
                !matches!(descriptor.result, lpp_runtime_abi::generated::AbiType::Void);
            let result = has_result.then(|| builder.func.dfg.first_result(call));
            return Ok(match result {
                // Membership predicates: the runtime returns an i64 0/1 but the
                // language result is a machine bool (i8).
                Some(raw) if is_map_predicate(name) => {
                    Some(builder.ins().ireduce(cltypes::I8, raw))
                }
                other => other,
            });
        }
        if is_family_net(name) {
            let symbol = descriptor.symbol;
            let mut call_args: Vec<Value> = Vec::with_capacity(args.len());
            for operand in args {
                call_args.push(self.operand_value(inst, builder, operand)?);
            }
            let func_ref = self.import_ref(builder, symbol);
            let call = builder.ins().call(func_ref, &call_args);
            let has_result =
                !matches!(descriptor.result, lpp_runtime_abi::generated::AbiType::Void);
            return Ok(has_result.then(|| builder.func.dfg.first_result(call)));
        }
        Err(CodegenError::new(
            Some(inst.mir_id),
            CodegenErrorKind::UnrepresentableBuiltin {
                builtin,
                reason: "not in the 5C2 table policy",
            },
        ))
    }
}
