use super::*;

pub(crate) fn check_instruction(
    lower: &mut ScanLower<'_>,
    kind: &InstructionKind,
) -> Result<(), CodegenError> {
    match kind {
        InstructionKind::Assign { target, value } => {
            collect_strings(lower.program, value, lower.strings);
            check_rvalue(lower, value)?;
            // Function-value provenance (5D2b slice 3): a value
            // established here is the only one a `spawn` in this
            // function may target.
            match value {
                Rvalue::Use(Operand::Function(id)) => {
                    lower.value_origins.insert((lower.fn_id, *target), *id);
                }
                Rvalue::MakeClosure { function, .. } => {
                    lower
                        .value_origins
                        .insert((lower.fn_id, *target), *function);
                }
                Rvalue::Use(Operand::Copy(src)) => {
                    if let Some(origin) = lower.value_origins.get(&(lower.fn_id, *src)) {
                        lower.value_origins.insert((lower.fn_id, *target), *origin);
                    }
                }
                _ => {}
            }
            // A list literal's element class comes from the target.
            if let Rvalue::List(_) = value {
                let TypeKind::List(element) =
                    lower.types.kind(lower.program.local(*target).unwrap().ty)
                else {
                    return Err(unsupported(
                        "list literal to non-list target",
                        Some(lower.fn_id),
                    ));
                };
                check_list_literal(lower, element)?;
            }
            if let Rvalue::Tuple(items) = value {
                let target_ty = lower.program.local(*target).unwrap().ty;
                let TypeKind::Tuple(elements) = lower.types.kind(target_ty) else {
                    return Err(unsupported(
                        "tuple literal to non-tuple target",
                        Some(lower.fn_id),
                    ));
                };
                let element_types = lower.types.list(elements);
                let values = lower.program.operands(*items);
                if values.len() != element_types.len() || values.len() > 64 {
                    return Err(unsupported("tuple arity", Some(lower.fn_id)));
                }
                *lower.has_tuple = true;
                *lower.has_managed = true;
                for value in values {
                    scan_operand_class(lower, value)?;
                }
            }
            Ok(())
        }
        InstructionKind::Store { place, value } => {
            let resolution = resolve_place(
                lower.program,
                lower.types,
                *place,
                lower.layouts,
                lower.aggregates,
                true,
                lower.fn_id,
            )?;
            register_list_load_needs(lower, &resolution);
            if let Some(PlaceStep::ListIndex { element, .. }) = resolution.steps.last() {
                match element_class(lower.types, *element) {
                    ElementClass::I64 => lower.needs.insert(H::ListSetI64),
                    ElementClass::F64 => lower.needs.insert(H::ListSetF64),
                    ElementClass::Ptr => lower.needs.insert(H::ListSetPtr),
                };
            }
            if let Operand::Constant(Constant::String { string, .. }) = value
                && let Some(content) = lower.program.string(*string)
            {
                lower.strings.insert(content.clone());
            }
            scan_operand_class(lower, value).map(|_| ())
        }
    }
}

pub(crate) fn check_rvalue(lower: &mut ScanLower<'_>, rvalue: &Rvalue) -> Result<(), CodegenError> {
    match rvalue {
        Rvalue::Use(operand) => {
            // A bare function value materializes a capsule (5D2b
            // slice 2); tasks stay rejected by `scan_operand_class`.
            scan_operand_class(lower, operand).map(|_| ())
        }
        Rvalue::Unary { operator, operand } => {
            let class = scan_operand_class(lower, operand)?;
            if class == Class::Ptr {
                return Err(unsupported(
                    "unary operator on managed type",
                    Some(lower.fn_id),
                ));
            }
            match (operator, class) {
                (UnaryOperator::Not, Class::Bool | Class::I64) => {}
                (UnaryOperator::Negate, Class::I64 | Class::F64) => {}
                _ => {
                    return Err(unsupported(
                        "unary operator on this operand type",
                        Some(lower.fn_id),
                    ));
                }
            }
            Ok(())
        }
        Rvalue::Binary {
            left,
            operator,
            right,
        } => {
            let lc = scan_operand_class(lower, left)?;
            let rc = scan_operand_class(lower, right)?;
            if lc != rc {
                return Err(unsupported(
                    "binary operand type mismatch",
                    Some(lower.fn_id),
                ));
            }
            if lc == Class::Ptr {
                return Err(unsupported(
                    "binary operator on managed type",
                    Some(lower.fn_id),
                ));
            }
            match (operator, lc) {
                (BinaryOperator::Equal, Class::Str) | (BinaryOperator::NotEqual, Class::Str) => {
                    lower.needs.insert(H::StrEq);
                }
                (BinaryOperator::Add, Class::Str) => {
                    *lower.has_managed = true;
                    lower.needs.insert(H::Alloc);
                    lower.needs.insert(H::StrConcat);
                }
                (BinaryOperator::Subtract, Class::Str)
                | (BinaryOperator::Multiply, Class::Str)
                | (BinaryOperator::Divide, Class::Str)
                | (BinaryOperator::Modulo, Class::Str) => {
                    return Err(unsupported("string arithmetic", Some(lower.fn_id)));
                }
                _ => {}
            }
            // The remaining operator/type combinations are lowered to native
            // wasm ops (verified in the lowering).
            let _ = rc;
            Ok(())
        }
        Rvalue::ExternalCall { .. } => Err(unsupported("external call", Some(lower.fn_id))),
        Rvalue::Call { callee, arguments } => {
            match callee {
                Operand::Function(f) => {
                    // A direct async call constructs a task through
                    // the function's thunk (5D2b slice 3).
                    if matches!(
                        lower.program.function(*f).unwrap().kind,
                        lpp_mir::MirFunctionKind::Async
                    ) {
                        *lower.has_tasks = true;
                        lower.task_thunks.push(*f);
                        lower.needs.insert(H::TaskNew);
                    }
                }
                Operand::Copy(local) => {
                    // A capsule call: the callee local holds the
                    // 16-byte [code, env] node. A value whose result
                    // is a Task constructs a task (5D2b slice 3).
                    let ty = lower.program.local(*local).unwrap().ty;
                    let TypeKind::Function { result, .. } = lower.types.kind(ty) else {
                        return Err(unsupported("indirect call", Some(lower.fn_id)));
                    };
                    if matches!(lower.types.kind(result), TypeKind::Task(_)) {
                        *lower.has_tasks = true;
                        lower.needs.insert(H::TaskNew);
                    }
                }
                _ => {
                    return Err(unsupported("indirect call", Some(lower.fn_id)));
                }
            }
            for operand in lower.program.operands(*arguments) {
                scan_operand_class(lower, operand)?;
            }
            Ok(())
        }
        Rvalue::Builtin { builtin, arguments } => {
            let args = lower.program.operands(*arguments);
            check_builtin(lower, *builtin, args)
        }
        Rvalue::Load(place) => {
            let resolution = resolve_place(
                lower.program,
                lower.types,
                *place,
                lower.layouts,
                lower.aggregates,
                false,
                lower.fn_id,
            )?;
            register_list_load_needs(lower, &resolution);
            scan_operand_class(lower, &Operand::Copy(resolution.root)).map(|_| ())
        }
        // List literals are checked from their assignment context (the
        // target/place type supplies the element class of `[]`).
        Rvalue::List(_) => Ok(()),
        // Tuple literals are validated from their assignment context because
        // the target local supplies the element types.
        Rvalue::Tuple(_) => Ok(()),
        Rvalue::ConstructStruct { fields, .. } => {
            *lower.has_managed = true;
            for operand in lower.program.operands(*fields) {
                scan_operand_class(lower, operand)?;
            }
            Ok(())
        }
        Rvalue::ConstructVariant { fields, .. } => {
            *lower.has_managed = true;
            for operand in lower.program.operands(*fields) {
                scan_operand_class(lower, operand)?;
            }
            Ok(())
        }
        Rvalue::ListLen(operand) => {
            if scan_operand_class(lower, operand)? != Class::Ptr {
                return Err(unsupported("list length of non-list", Some(lower.fn_id)));
            }
            *lower.has_list = true;
            *lower.has_managed = true;
            lower.needs.insert(H::ListLen);
            Ok(())
        }
        Rvalue::MakeClosure { captures, .. } => {
            for operand in lower.program.operands(*captures) {
                if !matches!(operand, Operand::Copy(_)) {
                    return Err(unsupported("non-local capture", Some(lower.fn_id)));
                }
            }
            // The capsule and env are ARC nodes.
            *lower.has_managed = true;
            Ok(())
        }
        Rvalue::Await(operand) => {
            // 5D2b slice 3: `x.await` on a task local.
            let Operand::Copy(local) = operand else {
                return Err(unsupported("await of non-local task", Some(lower.fn_id)));
            };
            let ty = lower.program.local(*local).unwrap().ty;
            if !matches!(lower.types.kind(ty), TypeKind::Task(_)) {
                return Err(unsupported("await of non-task", Some(lower.fn_id)));
            }
            *lower.has_tasks = true;
            lower.needs.insert(H::TaskAwait);
            Ok(())
        }
        Rvalue::Spawn(operand) => {
            // 5D2b slice 3: eager spawn of a zero-parameter,
            // void-result value (closure or bare function).
            let Operand::Copy(local) = operand else {
                return Err(unsupported("spawn of non-local value", Some(lower.fn_id)));
            };
            let ty = lower.program.local(*local).unwrap().ty;
            let TypeKind::Function { parameters, result } = lower.types.kind(ty) else {
                return Err(unsupported("spawn of non-function", Some(lower.fn_id)));
            };
            let void = lower.types.primitive(PrimitiveType::Void);
            if !lower.types.list(parameters).is_empty() || result != void {
                return Err(unsupported(
                    "spawn of a non-void or parameterized value",
                    Some(lower.fn_id),
                ));
            }
            let origin = lower
                .value_origins
                .get(&(lower.fn_id, *local))
                .copied()
                .ok_or_else(|| {
                    unsupported(
                        "spawn of a function value of unknown origin",
                        Some(lower.fn_id),
                    )
                })?;
            let origin_fn = lower.program.function(origin).unwrap();
            let user_params = lower
                .program
                .function_parameters(origin_fn)
                .iter()
                .filter(|&p| {
                    !matches!(
                        lower.program.local(*p).unwrap().kind,
                        lpp_mir::MirLocalKind::Capture
                    )
                })
                .count();
            if user_params != 0 {
                return Err(unsupported(
                    "spawn of a parameterized value",
                    Some(lower.fn_id),
                ));
            }
            match origin_fn.kind {
                lpp_mir::MirFunctionKind::Closure => lower.closure_thunks.push(origin),
                lpp_mir::MirFunctionKind::Function => lower.task_thunks.push(origin),
                lpp_mir::MirFunctionKind::Async => {
                    return Err(unsupported("spawn of an async value", Some(lower.fn_id)));
                }
            }
            *lower.has_tasks = true;
            lower.needs.insert(H::TaskNew);
            lower.needs.insert(H::TaskPoll);
            lower.needs.insert(H::TaskDestroy);
            Ok(())
        }
    }
}

/// Register the `ListGet` needs for every list-index step of a place
/// (intermediate steps borrow through the same plain element read).
pub(crate) fn register_list_load_needs(lower: &mut ScanLower<'_>, resolution: &PlaceResolution) {
    for step in &resolution.steps {
        match step {
            PlaceStep::ListIndex { element, .. } => {
                *lower.has_list = true;
                *lower.has_managed = true;
                match element_class(lower.types, *element) {
                    ElementClass::I64 => lower.needs.insert(H::ListGetI64),
                    ElementClass::F64 => lower.needs.insert(H::ListGetF64),
                    ElementClass::Ptr => lower.needs.insert(H::ListGetPtr),
                };
            }
            PlaceStep::TupleField { .. } => {
                *lower.has_tuple = true;
                *lower.has_managed = true;
            }
            PlaceStep::Downcast | PlaceStep::Field { .. } => {}
        }
    }
}

/// Register the list-helper needs for a list literal of element type
/// `element` (the literal only needs `ListNew` + the element's `Push`;
/// reads/writes/lengths register their own helpers at the use sites).
pub(crate) fn check_list_literal(
    lower: &mut ScanLower<'_>,
    element: TypeId,
) -> Result<(), CodegenError> {
    *lower.has_list = true;
    *lower.has_managed = true;
    lower.needs.insert(H::ListNew);
    match element_class(lower.types, element) {
        ElementClass::I64 => lower.needs.insert(H::ListPushI64),
        ElementClass::F64 => lower.needs.insert(H::ListPushF64),
        ElementClass::Ptr => lower.needs.insert(H::ListPushPtr),
    };
    Ok(())
}
