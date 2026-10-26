use super::*;

impl<'m> Lowering<'m> {
    pub(super) fn lower_assign(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        target: MirLocalId,
        value: &Rvalue,
    ) -> Result<(), CodegenError> {
        let target_ty = inst
            .program
            .local(target)
            .expect("validated MIR retains locals")
            .ty;
        let is_void = is_void_type(inst.types, target_ty);

        let produced = match value {
            Rvalue::Use(operand) => {
                let v = if let Operand::Function(function_id) = operand {
                    self.materialize_function_value(inst, builder, *function_id)?
                } else {
                    self.owned_value(inst, builder, operand)?
                };
                Some(v)
            }
            Rvalue::Unary { operator, operand } => {
                let value = self.operand_value(inst, builder, operand)?;
                let ty = builder.func.dfg.value_type(value);
                let result = match (operator, ty) {
                    (UnaryOperator::Negate, t) if t.is_int() => builder.ins().ineg(value),
                    (UnaryOperator::Negate, cltypes::F64) => builder.ins().fneg(value),
                    (UnaryOperator::Not, cltypes::I8) => {
                        let one = builder.ins().iconst(cltypes::I8, 1);
                        builder.ins().bxor(value, one)
                    }
                    _ => return Err(unsupported(inst.mir_id, "invalid unary operands")),
                };
                Some(result)
            }
            Rvalue::Binary {
                left,
                operator,
                right,
            } => {
                let l = self.operand_value(inst, builder, left)?;
                let r = self.operand_value(inst, builder, right)?;
                let left_is_string = operand_language_kind(inst.program, inst.types, left)
                    == Some(TypeKind::Primitive(PrimitiveType::String));
                // String `+` is contents concatenation, not pointer
                // arithmetic: route it through `lpp_str_concat`, which
                // borrows both operands and returns a fresh owned string.
                if *operator == BinaryOperator::Add && left_is_string {
                    // Coerce a scalar right operand to a string before
                    // concatenation (`"n=" + 3`, and f-string interpolation of a
                    // non-string value).
                    let right_str = match operand_language_kind(inst.program, inst.types, right) {
                        Some(TypeKind::Primitive(PrimitiveType::String)) => r,
                        Some(TypeKind::Primitive(PrimitiveType::Int)) => {
                            let f = self.import_ref(builder, IMP_INT_TO_STR);
                            let c = builder.ins().call(f, &[r]);
                            builder.func.dfg.first_result(c)
                        }
                        Some(TypeKind::Primitive(PrimitiveType::Float)) => {
                            let f = self.import_ref(builder, IMP_FLOAT_TO_STR);
                            let c = builder.ins().call(f, &[r]);
                            builder.func.dfg.first_result(c)
                        }
                        Some(TypeKind::Primitive(PrimitiveType::Bool)) => {
                            let f = self.import_ref(builder, IMP_BOOL_TO_STR);
                            let c = builder.ins().call(f, &[r]);
                            builder.func.dfg.first_result(c)
                        }
                        _ => return Err(unsupported(inst.mir_id, "string concat with non-scalar")),
                    };
                    let func_ref = self.import_ref(builder, IMP_STR_CONCAT);
                    let call_inst = builder.ins().call(func_ref, &[l, right_str]);
                    Some(builder.func.dfg.first_result(call_inst))
                } else {
                    let string_eq =
                        matches!(*operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
                            && left_is_string;
                    let left_kind = operand_language_kind(inst.program, inst.types, left);
                    let right_kind = operand_language_kind(inst.program, inst.types, right);
                    let (l, r, ty) = normalize_integer_operands(
                        builder,
                        l,
                        r,
                        is_signed_integer_kind(left_kind),
                        is_signed_integer_kind(right_kind),
                    );
                    Some(self.lower_binary(builder, *operator, ty, string_eq, l, r)?)
                }
            }
            Rvalue::ExternalCall {
                function: external_id,
                arguments,
            } => {
                let external = inst
                    .program
                    .external_function(*external_id)
                    .expect("validated MIR retains external functions");
                // A L++ `Str` operand is an ARC allocation whose value is
                // its NUL-terminated payload pointer. Pass that pointer as a
                // borrowed C string; the foreign call contract forbids
                // retaining or mutating it.
                let args = inst
                    .program
                    .operands(*arguments)
                    .iter()
                    .map(|argument| self.operand_value(inst, builder, argument))
                    .collect::<Result<Vec<_>, _>>()?;
                let function_id = self
                    .external_ids
                    .get(external_id)
                    .copied()
                    .ok_or_else(|| unsupported(inst.mir_id, "undeclared external function"))?;
                let function_ref = self.module.declare_func_in_func(function_id, builder.func);
                let call = builder.ins().call(function_ref, &args);
                let TypeKind::Function { result, .. } = inst.types.kind(external.ty) else {
                    return Err(unsupported(inst.mir_id, "external has non-function type"));
                };
                if matches!(
                    inst.types.kind(result),
                    TypeKind::Primitive(PrimitiveType::String)
                ) {
                    // The C pointer is borrowed and must remain readable
                    // through this immediate copy (for example, static or
                    // thread-local storage). `lpp_str_concat` copies its bytes
                    // into a fresh L++ ARC string; never adopt/release it.
                    let foreign = builder.func.dfg.first_result(call);
                    let empty_fn = self.import_ref(builder, IMP_EMPTY_STR);
                    let empty_call = builder.ins().call(empty_fn, &[]);
                    let empty = builder.func.dfg.first_result(empty_call);
                    let copy_fn = self.import_ref(builder, IMP_STR_CONCAT);
                    let copy_call = builder.ins().call(copy_fn, &[foreign, empty]);
                    Some(builder.func.dfg.first_result(copy_call))
                } else if is_void_type(inst.types, result) {
                    None
                } else {
                    Some(builder.func.dfg.first_result(call))
                }
            }
            Rvalue::Call { callee, arguments } => {
                let args_ops = inst.program.operands(*arguments);
                match callee {
                    Operand::Function(callee_id) => {
                        let callee_fn = inst
                            .program
                            .function(*callee_id)
                            .expect("validated MIR retains functions");
                        match callee_fn.kind {
                            // Closures are called through their
                            // capsule; the pre-scan rejects the
                            // function-operand spelling.
                            MirFunctionKind::Closure => {
                                return Err(unsupported(
                                    inst.mir_id,
                                    "closure as function operand",
                                ));
                            }
                            // An async call builds the task env tuple
                            // and the handle through the thunk.
                            MirFunctionKind::Async => {
                                let collectible =
                                    self.shared_types.contains(&callee_fn.return_type);
                                let env =
                                    self.task_env_for_args(inst, builder, args_ops, collectible)?;
                                let thunk_ref = self.module.declare_func_in_func(
                                    self.task_thunk_ids[callee_id],
                                    builder.func,
                                );
                                let code = builder.ins().func_addr(cltypes::I64, thunk_ref);
                                let task = self.task_new(
                                    builder,
                                    code,
                                    env,
                                    Self::task_managed_flag(inst.types, callee_fn.return_type),
                                );
                                if collectible {
                                    self.mark_collectible_import(builder, task, IMP_TASK_TRACE);
                                }
                                Some(task)
                            }
                            MirFunctionKind::Function => {
                                let mut args: Vec<Value> = Vec::with_capacity(args_ops.len());
                                for operand in args_ops {
                                    // A managed argument retains for
                                    // the callee (the callee's exit
                                    // pass releases it); the source
                                    // keeps its own reference.
                                    args.push(self.owned_value(inst, builder, operand)?);
                                }
                                let func_ref = self
                                    .module
                                    .declare_func_in_func(self.func_ids[callee_id], builder.func);
                                let call_inst = builder.ins().call(func_ref, &args);
                                (machine_type(inst.types, callee_fn.return_type).is_some())
                                    .then(|| builder.func.dfg.first_result(call_inst))
                            }
                        }
                    }
                    Operand::Copy(local) => {
                        // A capsule call: the code pointer and env come
                        // from the capsule; the provenance class picks
                        // the ABI.
                        let class = self
                            .value_classes
                            .get(&(inst.mir_id, *local))
                            .copied()
                            .expect("pre-scan classified every function value");
                        let callee_ty = {
                            inst.program
                                .local(*local)
                                .expect("validated MIR retains locals")
                                .ty
                        };
                        let (param_tys, result_ty) = match inst.types.kind(callee_ty) {
                            TypeKind::Function { parameters, result } => {
                                (inst.types.list(parameters).to_vec(), result)
                            }
                            _ => return Err(unsupported(inst.mir_id, "non-function callee")),
                        };
                        let capsule = self.operand_value(inst, builder, &Operand::Copy(*local))?;
                        let code = builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), capsule, 0);
                        match class {
                            ValueClass::Closure => {
                                let env =
                                    builder
                                        .ins()
                                        .load(cltypes::I64, MemFlags::new(), capsule, 8);
                                if param_tys.is_empty() {
                                    // The zero-parameter capsule code
                                    // is the task-code trampoline:
                                    // (env) -> boxed I64.
                                    let mut sig = self.module.make_signature();
                                    sig.params.push(AbiParam::new(cltypes::I64));
                                    sig.returns.push(AbiParam::new(cltypes::I64));
                                    let sig_ref = builder.import_signature(sig);
                                    let call = builder.ins().call_indirect(sig_ref, code, &[env]);
                                    let boxed = builder.func.dfg.first_result(call);
                                    unbox_result(inst.types, builder, boxed, result_ty)
                                } else {
                                    let mut sig = self.module.make_signature();
                                    sig.params.push(AbiParam::new(cltypes::I64));
                                    for param_ty in &param_tys {
                                        sig.params.push(AbiParam::new(
                                            machine_type(inst.types, *param_ty)
                                                .expect("pre-scan checked the types"),
                                        ));
                                    }
                                    if let Some(cl) = machine_type(inst.types, result_ty) {
                                        sig.returns.push(AbiParam::new(cl));
                                    }
                                    let sig_ref = builder.import_signature(sig);
                                    let mut call_args: Vec<Value> = vec![env];
                                    for operand in args_ops {
                                        call_args.push(self.owned_value(inst, builder, operand)?);
                                    }
                                    let call =
                                        builder.ins().call_indirect(sig_ref, code, &call_args);
                                    (machine_type(inst.types, result_ty).is_some())
                                        .then(|| builder.func.dfg.first_result(call))
                                }
                            }
                            ValueClass::FunctionValue => {
                                if matches!(inst.types.kind(result_ty), TypeKind::Task(_)) {
                                    // A call through an async function
                                    // value constructs a task; the
                                    // capsule code is the task thunk.
                                    let collectible = self.shared_types.contains(&result_ty);
                                    let env = self.task_env_for_args(
                                        inst,
                                        builder,
                                        args_ops,
                                        collectible,
                                    )?;
                                    let task = self.task_new(
                                        builder,
                                        code,
                                        env,
                                        Self::task_managed_flag(inst.types, result_ty),
                                    );
                                    if collectible {
                                        self.mark_collectible_import(builder, task, IMP_TASK_TRACE);
                                    }
                                    Some(task)
                                } else {
                                    let mut sig = self.module.make_signature();
                                    for param_ty in &param_tys {
                                        sig.params.push(AbiParam::new(
                                            machine_type(inst.types, *param_ty)
                                                .expect("pre-scan checked the types"),
                                        ));
                                    }
                                    if let Some(cl) = machine_type(inst.types, result_ty) {
                                        sig.returns.push(AbiParam::new(cl));
                                    }
                                    let sig_ref = builder.import_signature(sig);
                                    let mut call_args: Vec<Value> =
                                        Vec::with_capacity(args_ops.len());
                                    for operand in args_ops {
                                        call_args.push(self.owned_value(inst, builder, operand)?);
                                    }
                                    let call =
                                        builder.ins().call_indirect(sig_ref, code, &call_args);
                                    (machine_type(inst.types, result_ty).is_some())
                                        .then(|| builder.func.dfg.first_result(call))
                                }
                            }
                        }
                    }
                    _ => return Err(unsupported(inst.mir_id, "non-function callee")),
                }
            }
            Rvalue::Load(place) => {
                let resolution = resolve_place(
                    inst.program,
                    inst.types,
                    *place,
                    &self.aggregates,
                    &self.layouts,
                    inst.mir_id,
                    false,
                )?;
                if resolution.steps.is_empty() {
                    Some(self.operand_value(inst, builder, &Operand::Copy(resolution.root))?)
                } else {
                    let base = self.place_chain_base(inst, builder, &resolution)?;
                    let loaded = match resolution.steps.last().expect("non-empty projections") {
                        PlaceStep::Field {
                            offset,
                            ty,
                            managed,
                            ..
                        } => {
                            let field_type =
                                machine_type(inst.types, *ty).expect("checked by the pre-scan");
                            let v = builder.ins().load(
                                field_type,
                                MemFlags::new(),
                                base,
                                *offset as i32,
                            );
                            if *managed {
                                // A field read gains a reference (the
                                // container keeps its own).
                                self.arc_retain(builder, v);
                            }
                            v
                        }
                        PlaceStep::ListIndex { element, index } => {
                            let idx = self.operand_value(inst, builder, index)?;
                            match element_class(inst.types, *element) {
                                ElementClass::Managed => {
                                    let func_ref = self.import_ref(builder, IMP_LIST_GET_ARC);
                                    let call = builder.ins().call(func_ref, &[base, idx]);
                                    let v = builder.func.dfg.first_result(call);
                                    // An element read gains a reference.
                                    self.arc_retain(builder, v);
                                    v
                                }
                                ElementClass::Scalar => {
                                    let func_ref = self.import_ref(builder, IMP_LIST_GET);
                                    let call = builder.ins().call(func_ref, &[base, idx]);
                                    let v = builder.func.dfg.first_result(call);
                                    coerce_to(
                                        builder,
                                        v,
                                        machine_type(inst.types, *element).expect("element type"),
                                    )
                                }
                                ElementClass::Float => {
                                    let func_ref = self.import_ref(builder, IMP_LIST_GET_FLOAT);
                                    let call = builder.ins().call(func_ref, &[base, idx]);
                                    builder.func.dfg.first_result(call)
                                }
                                ElementClass::Bool => {
                                    let func_ref = self.import_ref(builder, IMP_LIST_GET_BOOL);
                                    let call = builder.ins().call(func_ref, &[base, idx]);
                                    builder.func.dfg.first_result(call)
                                }
                            }
                        }
                    };
                    Some(loaded)
                }
            }
            Rvalue::ListLen(operand) => {
                let list = self.operand_value(inst, builder, operand)?;
                let func_ref = self.import_ref(builder, IMP_LIST_LEN);
                let call = builder.ins().call(func_ref, &[list]);
                Some(builder.func.dfg.first_result(call))
            }
            Rvalue::List(operands) => {
                let TypeKind::List(element) = inst.types.kind(target_ty) else {
                    return Err(unsupported(inst.mir_id, "list literal of non-list"));
                };
                let arc = is_managed(inst.types, element);
                let list = {
                    let func_ref =
                        self.import_ref(builder, if arc { IMP_LIST_NEW_ARC } else { IMP_LIST_NEW });
                    let call = builder.ins().call(func_ref, &[]);
                    builder.func.dfg.first_result(call)
                };
                if self.shared_types.contains(&target_ty) {
                    self.mark_collectible_import(builder, list, IMP_LIST_TRACE);
                }
                for operand in inst.program.operands(*operands) {
                    match element_class(inst.types, element) {
                        ElementClass::Managed => {
                            // Elements MOVE into the list (the
                            // oracle's `eval_operand_moved`): the
                            // runtime's push retains for the list, so
                            // consume the source reference and empty
                            // its local — retaining here as well (the
                            // old `owned_value`) leaked one reference
                            // per element.
                            let v = self.operand_value(inst, builder, operand)?;
                            let func_ref = self.import_ref(builder, IMP_LIST_PUSH_ARC);
                            builder.ins().call(func_ref, &[list, v]);
                            self.arc_release(builder, v);
                            self.empty_moved_local(inst, builder, operand);
                        }
                        ElementClass::Scalar => {
                            let v = self.operand_value(inst, builder, operand)?;
                            let stored = coerce_to(builder, v, cltypes::I64);
                            let func_ref = self.import_ref(builder, IMP_LIST_PUSH);
                            builder.ins().call(func_ref, &[list, stored]);
                        }
                        ElementClass::Float => {
                            let v = self.operand_value(inst, builder, operand)?;
                            let stored = coerce_to(builder, v, cltypes::F64);
                            let func_ref = self.import_ref(builder, IMP_LIST_PUSH_FLOAT);
                            builder.ins().call(func_ref, &[list, stored]);
                        }
                        ElementClass::Bool => {
                            let v = self.operand_value(inst, builder, operand)?;
                            let stored = coerce_to(builder, v, cltypes::I8);
                            let func_ref = self.import_ref(builder, IMP_LIST_PUSH_BOOL);
                            builder.ins().call(func_ref, &[list, stored]);
                        }
                    }
                }
                Some(list)
            }
            Rvalue::ConstructStruct { aggregate, fields } => {
                let layout = self
                    .layouts
                    .get(aggregate)
                    .expect("pre-scan laid out every nominal")
                    .clone();
                let ptr = self.arc_alloc(builder, layout.total_size, *aggregate);
                let values = inst.program.operands(*fields);
                if values.len() != layout.struct_fields.len() {
                    return Err(unsupported(inst.mir_id, "struct field count"));
                }
                for (value_operand, slot) in values.iter().zip(layout.struct_fields.iter()) {
                    let v = self.owned_value(inst, builder, value_operand)?;
                    let stored = coerce_to(
                        builder,
                        v,
                        machine_type(inst.types, slot.ty).expect("field type"),
                    );
                    builder
                        .ins()
                        .store(MemFlags::new(), stored, ptr, slot.offset as i32);
                }
                Some(ptr)
            }
            Rvalue::ConstructVariant {
                aggregate,
                variant,
                fields,
            } => {
                let layout = self
                    .layouts
                    .get(aggregate)
                    .expect("pre-scan laid out every nominal")
                    .clone();
                let ordinal = inst
                    .program
                    .variant(*variant)
                    .expect("validated MIR retains every variant")
                    .ordinal;
                let ptr = self.arc_alloc(builder, layout.total_size, *aggregate);
                let tag = builder.ins().iconst(cltypes::I64, ordinal as i64);
                builder.ins().store(MemFlags::new(), tag, ptr, 0);
                let slots = &layout.variant_fields[ordinal as usize];
                let values = inst.program.operands(*fields);
                if values.len() != slots.len() {
                    return Err(unsupported(inst.mir_id, "variant field count"));
                }
                for (value_operand, slot) in values.iter().zip(slots.iter()) {
                    let v = self.owned_value(inst, builder, value_operand)?;
                    let stored = coerce_to(
                        builder,
                        v,
                        machine_type(inst.types, slot.ty).expect("field type"),
                    );
                    builder
                        .ins()
                        .store(MemFlags::new(), stored, ptr, slot.offset as i32);
                }
                Some(ptr)
            }
            Rvalue::Builtin { builtin, arguments } => {
                let args = inst.program.operands(*arguments);
                match *builtin {
                    PRINT_STR => {
                        let value = self.operand_value(inst, builder, &args[0])?;
                        let func_ref = self.import_ref(builder, IMP_PRINT_STR);
                        builder.ins().call(func_ref, &[value]);
                        None
                    }
                    LIST_NEW => {
                        let TypeKind::List(element) = inst.types.kind(target_ty) else {
                            return Err(unsupported(inst.mir_id, "list allocation of non-list"));
                        };
                        let arc = is_managed(inst.types, element);
                        let func_ref = self
                            .import_ref(builder, if arc { IMP_LIST_NEW_ARC } else { IMP_LIST_NEW });
                        let call = builder.ins().call(func_ref, &[]);
                        let list = builder.func.dfg.first_result(call);
                        if self.shared_types.contains(&target_ty) {
                            self.mark_collectible_import(builder, list, IMP_LIST_TRACE);
                        }
                        Some(list)
                    }
                    other => self.lower_builtin_families(inst, builder, other, args)?,
                }
            }
            Rvalue::Tuple(elements) => {
                let (slots, total_size) = crate::layout::tuple_layout(inst.types, target_ty)
                    .ok_or_else(|| unsupported(inst.mir_id, "tuple of non-tuple type"))?;
                let values = inst.program.operands(*elements);
                if values.len() != slots.len() {
                    return Err(unsupported(inst.mir_id, "tuple element count"));
                }
                let ptr = self.alloc_tuple(builder, total_size, target_ty);
                for (value_operand, (offset, element_ty)) in values.iter().zip(slots.iter()) {
                    let v = self.owned_value(inst, builder, value_operand)?;
                    let stored = coerce_to(
                        builder,
                        v,
                        machine_type(inst.types, *element_ty).expect("tuple element type"),
                    );
                    builder
                        .ins()
                        .store(MemFlags::new(), stored, ptr, *offset as i32);
                }
                Some(ptr)
            }
            Rvalue::MakeClosure {
                function: closure_id,
                captures,
            } => {
                let closure_fn = inst
                    .program
                    .function(*closure_id)
                    .expect("validated MIR retains functions");
                let capture_args = inst.program.operands(*captures);
                let zero_params = user_parameters(inst.program, closure_fn).is_empty();
                let collectible = self.shared_types.contains(&closure_fn.ty);
                // The raw capture env (slots at 8*i) — NULL when the
                // closure captures nothing.
                let raw_env = if capture_args.is_empty() {
                    builder.ins().iconst(cltypes::I64, 0)
                } else {
                    let dtor_ref = self
                        .module
                        .declare_func_in_func(self.closure_dtor_ids[closure_id], builder.func);
                    let env =
                        self.arc_alloc_ptr(builder, (8 * capture_args.len()) as u32, dtor_ref);
                    if collectible {
                        let trace_id = self.closure_trace_ids[closure_id];
                        self.mark_collectible_generated(builder, env, trace_id);
                    }
                    for (i, operand) in capture_args.iter().enumerate() {
                        let captured_ty = match operand {
                            Operand::Copy(local) => {
                                inst.program
                                    .local(*local)
                                    .expect("validated MIR retains locals")
                                    .ty
                            }
                            _ => return Err(unsupported(inst.mir_id, "non-local capture")),
                        };
                        let value = self.operand_value(inst, builder, operand)?;
                        let value = if is_managed(inst.types, captured_ty) {
                            self.arc_retain_value(builder, value)
                        } else {
                            value
                        };
                        builder
                            .ins()
                            .store(MemFlags::new(), value, env, 8 * i as i32);
                    }
                    env
                };
                // The capsule: [code, env].
                let dtor_id = *self
                    .import_ids
                    .get(IMP_CLOSURE_DESTROY)
                    .expect("pre-scan declared lpp_closure_destroy");
                let dtor_ref = self.module.declare_func_in_func(dtor_id, builder.func);
                let capsule = self.arc_alloc_ptr(builder, 16, dtor_ref);
                if collectible {
                    self.mark_collectible_import(builder, capsule, IMP_CLOSURE_TRACE);
                }
                let code_ref = if zero_params {
                    self.closure_thunk_ids[closure_id]
                } else {
                    self.func_ids[closure_id]
                };
                let fn_ref = self.module.declare_func_in_func(code_ref, builder.func);
                let code = builder.ins().func_addr(cltypes::I64, fn_ref);
                builder.ins().store(MemFlags::new(), code, capsule, 0);
                if zero_params {
                    // Spawnable: the capsule env is the 1-slot task
                    // env tuple holding the raw env (a NULL slot is
                    // released as a no-op), so the same code pointer
                    // serves the direct call and the spawned task.
                    let tuple_ref = self.import_ref(builder, IMP_TUPLE_ALLOC);
                    let size = builder.ins().iconst(cltypes::I64, 24);
                    let mask = builder.ins().iconst(cltypes::I64, 1);
                    let offsets = builder.ins().iconst(cltypes::I64, 16);
                    let call = builder.ins().call(tuple_ref, &[size, mask, offsets]);
                    let wrapper = builder.func.dfg.first_result(call);
                    if collectible {
                        // The capsule trace reaches the raw env through
                        // this wrapper tuple; keep it traversable.
                        self.mark_collectible_import(builder, wrapper, IMP_TUPLE_TRACE);
                    }
                    // The allocation reference of the raw env is
                    // transferred to the wrapper slot (released by the
                    // tuple destructor at wrapper death) — no retain:
                    // retaining would leak one reference per closure.
                    let wrapped = raw_env;
                    builder.ins().store(MemFlags::new(), wrapped, wrapper, 16);
                    builder.ins().store(MemFlags::new(), wrapper, capsule, 8);
                } else {
                    builder.ins().store(MemFlags::new(), raw_env, capsule, 8);
                }
                Some(capsule)
            }
            Rvalue::Await(operand) => {
                let Operand::Copy(local) = operand else {
                    return Err(unsupported(inst.mir_id, "await of non-local task"));
                };
                let task = self.operand_value(inst, builder, operand)?;
                let func_ref = self.import_ref(builder, IMP_TASK_AWAIT);
                let call = builder.ins().call(func_ref, &[task]);
                let boxed = builder.func.dfg.first_result(call);
                let TypeKind::Task(inner) = inst.types.kind(
                    inst.program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .ty,
                ) else {
                    return Err(unsupported(inst.mir_id, "await of non-task"));
                };
                unbox_result(inst.types, builder, boxed, inner)
            }
            Rvalue::Spawn(operand) => {
                let Operand::Copy(local) = operand else {
                    return Err(unsupported(inst.mir_id, "spawn of non-local value"));
                };
                let capsule = self.operand_value(inst, builder, &Operand::Copy(*local))?;
                let code = builder
                    .ins()
                    .load(cltypes::I64, MemFlags::new(), capsule, 0);
                let env = builder
                    .ins()
                    .load(cltypes::I64, MemFlags::new(), capsule, 8);
                // The task holds the capsule's env for the closure's
                // (eager) lifetime; the capsule keeps its own.
                let env_retained = self.arc_retain_value(builder, env);
                let callee_ty = inst
                    .program
                    .local(*local)
                    .expect("validated MIR retains locals")
                    .ty;
                let TypeKind::Function { result, .. } = inst.types.kind(callee_ty) else {
                    return Err(unsupported(inst.mir_id, "spawn of non-function"));
                };
                let task = self.task_new(
                    builder,
                    code,
                    env_retained,
                    Self::task_managed_flag(inst.types, result),
                );
                let poll_ref = self.import_ref(builder, IMP_TASK_POLL);
                builder.ins().call(poll_ref, &[task]);
                let destroy_ref = self.import_ref(builder, IMP_TASK_DESTROY);
                builder.ins().call(destroy_ref, &[task]);
                None
            }
        };

        if let Some(value) = produced
            && !is_void
        {
            // Cell death: the slot's old reference (if any) is
            // released after the new value is fully produced — the
            // interpreter's order (a self-assignment retains before it
            // releases).
            let variable = *inst
                .local_vars
                .get(&target)
                .expect("validated MIR retains local variables");
            if is_managed(inst.types, target_ty) {
                let old = builder.use_var(variable);
                self.arc_release(builder, old);
            }
            // Context-typed integer literals enter MIR as canonical i64
            // constants and are materialized through a local carrying the
            // inferred fixed-width type. Coerce at the assignment boundary so
            // Cranelift's SSA variable type agrees with that language type.
            let value = machine_type(inst.types, target_ty)
                .map(|expected| coerce_to(builder, value, expected))
                .unwrap_or(value);
            builder.def_var(variable, value);
        }
        Ok(())
    }
}
