use super::*;

pub(super) fn check_assign_shape(
    program: &MirProgram,
    types: &TypeInterner,
    target: MirLocalId,
    value: &Rvalue,
    function: &MirFunction,
    mir_id: MirFunctionId,
    aggregates: &AggregateIndex,
    layouts: &HashMap<MirAggregateId, AggregateLayout>,
    scan: &mut ScanState,
) -> Result<(), CodegenError> {
    let check_operand = |operand: &Operand| operand_type(program, types, operand, mir_id);
    let local_ty = |local: MirLocalId| {
        program
            .local(local)
            .expect("validated MIR retains locals")
            .ty
    };
    let target_ty = local_ty(target);
    if !machine_type(types, target_ty).is_some() && !is_void_type(types, target_ty) {
        return Err(unsupported(
            mir_id,
            unsupported_construct_for(types.kind(target_ty)),
        ));
    }
    match value {
        Rvalue::Use(operand) => {
            check_operand(operand)?;
            if let Operand::Copy(local) = operand
                && is_managed(types, local_ty(*local))
            {
                scan.imports.insert(IMP_ARC_RETAIN);
            }
            // Function value materialization: capsule
            // plus, for async functions, the task
            // thunk that adapts the runtime argument
            // layout.
            if let Operand::Function(function_id) = operand {
                let callee_fn = program
                    .function(*function_id)
                    .expect("validated MIR retains functions");
                scan.imports.insert(IMP_ARC_ALLOC);
                scan.imports.insert(IMP_CLOSURE_DESTROY);
                if scan.shared.contains(&callee_fn.ty) {
                    scan.imports.insert(IMP_MARK_COLLECTIBLE);
                    scan.imports.insert(IMP_CLOSURE_TRACE);
                }
                match callee_fn.kind {
                    MirFunctionKind::Async => {
                        scan.task_thunks.push(*function_id);
                        // The value's capsule env is
                        // an empty task-env tuple
                        // (spawnable values must keep
                        // a non-NULL env).
                        scan.imports.insert(IMP_TUPLE_ALLOC);
                    }
                    // A sync value calls `lpp_f`
                    // directly through the capsule
                    // (no thunk: the capsule's code
                    // pointer is the function itself,
                    // the env is NULL).
                    MirFunctionKind::Function => {}
                    MirFunctionKind::Closure => {
                        return Err(unsupported(mir_id, "closure as value operand"));
                    }
                }
                record_value_class(scan, mir_id, target, ValueClass::FunctionValue)?;
            }
            // Copying a function-value local (or a list
            // of function values) keeps the provenance
            // class.
            if let Operand::Copy(local) = operand
                && is_function_value_kind(types, types.kind(local_ty(*local)))
            {
                let class = scan.value_classes.get(&(mir_id, *local)).copied();
                match class {
                    Some(class) => {
                        record_value_class(scan, mir_id, target, class)?;
                    }
                    // A list copy taken before any push
                    // (or list_new itself) has no class
                    // yet; the later push to that local
                    // establishes it. Direct function
                    // values must keep a strict chain.
                    None => {
                        if !matches!(types.kind(local_ty(*local)), TypeKind::List(_)) {
                            return Err(unsupported(mir_id, "function value of unknown origin"));
                        }
                    }
                }
            }
        }
        Rvalue::Unary { operator, operand } => {
            let ty = check_operand(operand)?;
            match (operator, ty) {
                (UnaryOperator::Negate, t) if t.is_int() => {}
                (UnaryOperator::Negate, cltypes::F64) => {}
                (UnaryOperator::Not, cltypes::I8) => {}
                _ => {
                    return Err(unsupported(mir_id, "invalid unary operands"));
                }
            }
        }
        Rvalue::Binary {
            left,
            operator,
            right,
        } => {
            let left_is_string = operand_language_kind(program, types, left)
                == Some(TypeKind::Primitive(PrimitiveType::String));
            // String `+` concatenates contents through
            // `lpp_str_concat`, coercing a scalar right operand
            // to a string first. The machine-type check does not
            // apply here (a float right operand is stringified,
            // not added), so it is skipped on this path.
            if *operator == BinaryOperator::Add && left_is_string {
                scan.imports.insert(IMP_STR_CONCAT);
                match operand_language_kind(program, types, right) {
                    Some(TypeKind::Primitive(PrimitiveType::String)) => {}
                    Some(TypeKind::Primitive(PrimitiveType::Int)) => {
                        scan.imports.insert(IMP_INT_TO_STR);
                    }
                    Some(TypeKind::Primitive(PrimitiveType::Float)) => {
                        scan.imports.insert(IMP_FLOAT_TO_STR);
                    }
                    Some(TypeKind::Primitive(PrimitiveType::Bool)) => {
                        scan.imports.insert(IMP_BOOL_TO_STR);
                    }
                    _ => {
                        return Err(unsupported(mir_id, "string concatenation with non-scalar"));
                    }
                }
            } else {
                let left_ty = check_operand(left)?;
                let right_ty = check_operand(right)?;
                check_binary(*operator, left_ty, right_ty, mir_id)?;
                if *operator == BinaryOperator::Modulo && left_ty == cltypes::F64 {
                    scan.imports.insert(IMP_FMOD);
                }
                // Integer div/mod guards the zero
                // divisor with the defined panic import.
                if matches!(*operator, BinaryOperator::Divide | BinaryOperator::Modulo)
                    && left_ty != cltypes::F64
                {
                    scan.imports.insert(IMP_PANIC_DIV_ZERO);
                }
                // String equality compares the contents, not the
                // heap pointers.
                if matches!(*operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
                    && left_is_string
                {
                    scan.imports.insert(IMP_STR_EQ);
                }
            }
        }
        Rvalue::ExternalCall {
            function: external_id,
            arguments,
        } => {
            let external = program
                .external_function(*external_id)
                .ok_or_else(|| unsupported(mir_id, "missing external function"))?;
            if external.abi != "C" {
                return Err(unsupported(mir_id, "unsupported external ABI"));
            }
            let TypeKind::Function { parameters, result } = types.kind(external.ty) else {
                return Err(unsupported(mir_id, "external has non-function type"));
            };
            let supported_scalar = |ty, allow_string, allow_void| {
                matches!(
                    types.kind(ty),
                    TypeKind::Primitive(
                        PrimitiveType::Bool
                            | PrimitiveType::Int
                            | PrimitiveType::Float
                            | PrimitiveType::U8
                            | PrimitiveType::U16
                            | PrimitiveType::U32
                            | PrimitiveType::U64
                            | PrimitiveType::USize
                            | PrimitiveType::I8
                            | PrimitiveType::I16
                            | PrimitiveType::I32
                            | PrimitiveType::ISize
                            | PrimitiveType::RawPtr
                    ) | TypeKind::Ptr(_)
                ) || (allow_string
                    && matches!(types.kind(ty), TypeKind::Primitive(PrimitiveType::String)))
                    || (allow_void && is_void_type(types, ty))
            };
            if !supported_scalar(result, true, true)
                || types
                    .list(parameters)
                    .iter()
                    .any(|parameter| !supported_scalar(*parameter, true, false))
            {
                return Err(unsupported(mir_id, "unsupported external signature"));
            }
            if !machine_type(types, result).is_some() && !is_void_type(types, result) {
                return Err(unsupported(
                    mir_id,
                    unsupported_construct_for(types.kind(result)),
                ));
            }
            if result != local_ty(target) {
                return Err(unsupported(mir_id, "external result type mismatch"));
            }
            let expected = types.list(parameters);
            let actual = program.operands(*arguments);
            if expected.len() != actual.len() {
                return Err(unsupported(mir_id, "external call arity mismatch"));
            }
            for (expected, operand) in expected.iter().zip(actual) {
                let expected_machine = check_value_type(types, *expected, mir_id)?;
                let actual_machine = check_operand(operand)?;
                if expected_machine != actual_machine {
                    return Err(unsupported(mir_id, "external argument type mismatch"));
                }
            }
            scan.external_functions.insert(*external_id);
            if matches!(
                types.kind(result),
                TypeKind::Primitive(PrimitiveType::String)
            ) {
                // C returns a borrowed NUL-terminated buffer.
                // Copy it to an ARC-owned L++ string before
                // normal MIR ownership cleanup can release it.
                scan.imports.insert(IMP_EMPTY_STR);
                scan.imports.insert(IMP_STR_CONCAT);
            }
        }
        Rvalue::Call { callee, arguments } => {
            let user_arity = |callee_ty: TypeId| match types.kind(callee_ty) {
                TypeKind::Function { parameters, result } => (types.list(parameters).len(), result),
                _ => (0, callee_ty),
            };
            let (user_arity, result_ty) = match callee {
                Operand::Function(callee_id) => {
                    let callee_fn = program
                        .function(*callee_id)
                        .expect("validated MIR retains functions");
                    match callee_fn.kind {
                        // Closures are called through
                        // their capsule, never as a
                        // function operand.
                        MirFunctionKind::Closure => {
                            return Err(unsupported(mir_id, "closure as function operand"));
                        }
                        // An async call builds the
                        // task env tuple and the task
                        // handle through the thunk.
                        MirFunctionKind::Async => {
                            scan.task_thunks.push(*callee_id);
                            scan.imports.insert(IMP_TUPLE_ALLOC);
                            scan.imports.insert(IMP_TASK_NEW);
                            if scan.shared.contains(&callee_fn.return_type) {
                                scan.imports.insert(IMP_MARK_COLLECTIBLE);
                                scan.imports.insert(IMP_TASK_TRACE);
                                scan.imports.insert(IMP_TUPLE_TRACE);
                            }
                        }
                        MirFunctionKind::Function => {}
                    }
                    let user_params: usize = program
                        .function_parameters(callee_fn)
                        .iter()
                        .filter(|local| {
                            !matches!(
                                program
                                    .local(**local)
                                    .expect("validated MIR retains locals")
                                    .kind,
                                lpp_mir::MirLocalKind::Capture
                            )
                        })
                        .count();
                    (user_params, callee_fn.return_type)
                }
                Operand::Copy(local) => {
                    // The provenance class decides the
                    // capsule code ABI; a value of
                    // unknown origin never reaches a
                    // call site.
                    if scan.value_classes.get(&(mir_id, *local)).is_none() {
                        return Err(unsupported(mir_id, "function value of unknown origin"));
                    }
                    let callee_ty = local_ty(*local);
                    let (arity, result) = user_arity(callee_ty);
                    if !matches!(types.kind(callee_ty), TypeKind::Function { .. }) {
                        return Err(unsupported(mir_id, "non-function callee"));
                    }
                    // A call through an async function
                    // value builds the task env tuple
                    // and the handle; the capsule's
                    // code pointer is the task thunk.
                    if matches!(types.kind(result), TypeKind::Task(_)) {
                        scan.imports.insert(IMP_TUPLE_ALLOC);
                        scan.imports.insert(IMP_TASK_NEW);
                        if scan.shared.contains(&result) {
                            scan.imports.insert(IMP_MARK_COLLECTIBLE);
                            scan.imports.insert(IMP_TASK_TRACE);
                            scan.imports.insert(IMP_TUPLE_TRACE);
                        }
                    }
                    (arity, result)
                }
                _ => return Err(unsupported(mir_id, "non-function callee")),
            };
            if !machine_type(types, result_ty).is_some() && !is_void_type(types, result_ty) {
                return Err(unsupported(
                    mir_id,
                    unsupported_construct_for(types.kind(result_ty)),
                ));
            }
            let args = program.operands(*arguments);
            if args.len() != user_arity {
                return Err(unsupported(mir_id, "call arity mismatch"));
            }
            for operand in args {
                check_operand(operand)?;
                // A managed argument retains for the
                // callee (released by the callee's
                // exit pass) or, for an async call,
                // for the task env tuple.
                if let Operand::Copy(local) = *operand
                    && is_managed(types, local_ty(local))
                {
                    scan.imports.insert(IMP_ARC_RETAIN);
                }
            }
        }
        Rvalue::Load(place) => {
            let resolution =
                resolve_place(program, types, *place, aggregates, layouts, mir_id, false)?;
            check_value_type(types, resolution.ty, mir_id)?;
            if is_managed(types, resolution.ty) {
                // A field/element read gains a
                // reference.
                scan.imports.insert(IMP_ARC_RETAIN);
            }
            // A list read of a function value
            // propagates the list's provenance class.
            if let Some(PlaceStep::ListIndex { element, .. }) = resolution.steps.last()
                && matches!(types.kind(*element), TypeKind::Function { .. })
            {
                let class = scan
                    .value_classes
                    .get(&(mir_id, resolution.root))
                    .copied()
                    .ok_or_else(|| unsupported(mir_id, "function value of unknown origin"))?;
                record_value_class(scan, mir_id, target, class)?;
            }
            record_chain_gets(types, &resolution, scan.imports);
        }
        Rvalue::ListLen(operand) => {
            let ty = check_operand(operand)?;
            if ty != cltypes::I64 {
                return Err(unsupported(mir_id, "list length of non-list"));
            }
            let operand_ty = match operand {
                Operand::Copy(local) => local_ty(*local),
                _ => return Err(unsupported(mir_id, "list length of non-list")),
            };
            if !matches!(types.kind(operand_ty), TypeKind::List(_)) {
                return Err(unsupported(mir_id, "list length of non-list"));
            }
            scan.imports.insert(IMP_LIST_LEN);
        }
        Rvalue::List(operands) => {
            let TypeKind::List(element) = types.kind(target_ty) else {
                return Err(unsupported(mir_id, "list literal of non-list"));
            };
            check_list_element(types, element, mir_id)?;
            let arc = is_managed(types, element);
            scan.imports
                .insert(if arc { IMP_LIST_NEW_ARC } else { IMP_LIST_NEW });
            if scan.shared.contains(&target_ty) {
                scan.imports.insert(IMP_MARK_COLLECTIBLE);
                scan.imports.insert(IMP_LIST_TRACE);
            }
            for operand in program.operands(*operands) {
                check_operand(operand)?;
                match element_class(types, element) {
                    ElementClass::Managed => {
                        scan.imports.insert(IMP_LIST_PUSH_ARC);
                    }
                    ElementClass::Scalar => {
                        scan.imports.insert(IMP_LIST_PUSH);
                    }
                    ElementClass::Float => {
                        scan.imports.insert(IMP_LIST_PUSH_FLOAT);
                    }
                    ElementClass::Bool => {
                        scan.imports.insert(IMP_LIST_PUSH_BOOL);
                    }
                };
                // A list literal of function values
                // inherits the element's provenance
                // class.
                if matches!(types.kind(element), TypeKind::Function { .. })
                    && let Operand::Copy(source) = *operand
                {
                    let class = scan
                        .value_classes
                        .get(&(mir_id, source))
                        .copied()
                        .ok_or_else(|| unsupported(mir_id, "function value of unknown origin"))?;
                    record_value_class(scan, mir_id, target, class)?;
                }
            }
        }
        Rvalue::ConstructStruct { aggregate, fields } => {
            let agg = program
                .aggregate(*aggregate)
                .expect("validated MIR retains every aggregate");
            if agg.kind != lpp_mir::MirAggregateKind::Struct {
                return Err(unsupported(mir_id, "struct construction of enum"));
            }
            for operand in program.operands(*fields) {
                check_operand(operand)?;
                if let Operand::Copy(local) = *operand
                    && is_managed(types, local_ty(local))
                {
                    scan.imports.insert(IMP_ARC_RETAIN);
                }
            }
            scan.imports.insert(IMP_ARC_ALLOC);
            scan.destructors.push(*aggregate);
            if scan.shared.contains(
                &program
                    .aggregate(*aggregate)
                    .expect("validated MIR retains every aggregate")
                    .ty,
            ) {
                scan.imports.insert(IMP_MARK_COLLECTIBLE);
                scan.imports.insert(IMP_TRACE_VISIT);
            }
        }
        Rvalue::ConstructVariant {
            aggregate,
            variant,
            fields,
        } => {
            let agg = program
                .aggregate(*aggregate)
                .expect("validated MIR retains every aggregate");
            if agg.kind != lpp_mir::MirAggregateKind::Enum {
                return Err(unsupported(mir_id, "variant construction of struct"));
            }
            let variant_descriptor = program
                .variant(*variant)
                .expect("validated MIR retains every variant");
            if variant_descriptor.aggregate != *aggregate {
                return Err(unsupported(mir_id, "variant aggregate mismatch"));
            }
            for operand in program.operands(*fields) {
                check_operand(operand)?;
                if let Operand::Copy(local) = *operand
                    && is_managed(types, local_ty(local))
                {
                    scan.imports.insert(IMP_ARC_RETAIN);
                }
            }
            scan.imports.insert(IMP_ARC_ALLOC);
            scan.destructors.push(*aggregate);
            if scan.shared.contains(
                &program
                    .aggregate(*aggregate)
                    .expect("validated MIR retains every aggregate")
                    .ty,
            ) {
                scan.imports.insert(IMP_MARK_COLLECTIBLE);
                scan.imports.insert(IMP_TRACE_VISIT);
            }
        }
        Rvalue::Builtin { builtin, arguments } => {
            check_builtin_shape(
                program, types, target, target_ty, builtin, arguments, mir_id, scan,
            )?;
        }
        Rvalue::Tuple(elements) => {
            for operand in program.operands(*elements) {
                check_operand(operand)?;
                if let Operand::Copy(local) = *operand
                    && is_managed(types, local_ty(local))
                {
                    scan.imports.insert(IMP_ARC_RETAIN);
                }
            }
            scan.imports.insert(IMP_ARC_ALLOC);
            scan.tuple_destructors.push(local_ty(target));
            if scan.shared.contains(&local_ty(target)) {
                scan.imports.insert(IMP_MARK_COLLECTIBLE);
                scan.imports.insert(IMP_TRACE_VISIT);
            }
        }
        Rvalue::MakeClosure {
            function: closure_id,
            captures,
        } => {
            let closure_fn = program
                .function(*closure_id)
                .expect("validated MIR retains functions");
            if !matches!(closure_fn.kind, MirFunctionKind::Closure) {
                return Err(unsupported(mir_id, "closure over non-closure"));
            }
            // The builder passes the captures in the
            // closure's capture-parameter order.
            let capture_params = program
                .function_parameters(closure_fn)
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
                .count();
            let capture_args: &[Operand] = program.operands(*captures);
            if capture_params != capture_args.len() {
                return Err(unsupported(mir_id, "capture count mismatch"));
            }
            scan.imports.insert(IMP_ARC_ALLOC);
            scan.imports.insert(IMP_TUPLE_ALLOC);
            scan.imports.insert(IMP_CLOSURE_DESTROY);
            // A zero-user-parameter closure is
            // spawnable: its capsule code pointer is
            // the task-code trampoline.
            let spawnable = user_parameters(program, closure_fn).is_empty();
            if spawnable {
                scan.closure_thunks.push(*closure_id);
            }
            if scan.shared.contains(&closure_fn.ty) {
                scan.imports.insert(IMP_MARK_COLLECTIBLE);
                scan.imports.insert(IMP_CLOSURE_TRACE);
                if !capture_args.is_empty() {
                    scan.imports.insert(IMP_TRACE_VISIT);
                }
                if spawnable {
                    scan.imports.insert(IMP_TUPLE_TRACE);
                }
            }
            if !capture_args.is_empty() {
                scan.closure_dtors.push(*closure_id);
            }
            record_value_class(scan, mir_id, target, ValueClass::Closure)?;
            for operand in capture_args {
                check_operand(operand)?;
                if let Operand::Copy(local) = *operand
                    && is_managed(types, local_ty(local))
                {
                    // The env slot retains the capture
                    // for the capsule's lifetime.
                    scan.imports.insert(IMP_ARC_RETAIN);
                }
            }
        }
        Rvalue::Await(operand) => {
            let local = match operand {
                Operand::Copy(local) => *local,
                _ => return Err(unsupported(mir_id, "await of non-local task")),
            };
            let TypeKind::Task(inner) = types.kind(local_ty(local)) else {
                return Err(unsupported(mir_id, "await of non-task"));
            };
            check_value_type(types, inner, mir_id)?;
            scan.imports.insert(IMP_TASK_AWAIT);
        }
        Rvalue::Spawn(operand) => {
            let local = match operand {
                Operand::Copy(local) => *local,
                _ => return Err(unsupported(mir_id, "spawn of non-local value")),
            };
            if scan.value_classes.get(&(mir_id, local)).is_none() {
                return Err(unsupported(mir_id, "function value of unknown origin"));
            }
            let TypeKind::Function { parameters, result } = types.kind(local_ty(local)) else {
                return Err(unsupported(mir_id, "spawn of non-function"));
            };
            // Spawned values run with no user
            // arguments (the oracle's arity rule):
            // the capsule's code pointer is the
            // task-code trampoline, and the capsule's
            // env is already task-env shaped.
            if !types.list(parameters).is_empty() {
                return Err(unsupported(mir_id, "spawn of parameterized value"));
            }
            if !machine_type(types, result).is_some() && !is_void_type(types, result) {
                return Err(unsupported(
                    mir_id,
                    unsupported_construct_for(types.kind(result)),
                ));
            }
            scan.imports.insert(IMP_TASK_NEW);
            scan.imports.insert(IMP_TASK_POLL);
            scan.imports.insert(IMP_TASK_DESTROY);
            scan.imports.insert(IMP_ARC_RETAIN);
        }
        Rvalue::Cast { value, target } => {
            check_operand(value)?;
            if machine_type(types, *target).is_none() {
                return Err(unsupported(
                    mir_id,
                    unsupported_construct_for(types.kind(*target)),
                ));
            }
        }
        Rvalue::Intrinsic {
            intrinsic,
            type_argument,
            arguments,
        } => {
            for arg in program.operands(*arguments) {
                check_operand(arg)?;
            }
            if let Some(ty_arg) = type_argument {
                let _ = static_type_size_align(program, types, *ty_arg);
            }
            if let Some(arch_imp) = intrinsic_arch_import(*intrinsic) {
                scan.imports.insert(arch_imp);
            }
        }
    }
    Ok(())
}
