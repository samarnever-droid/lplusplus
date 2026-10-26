use super::*;

pub(super) fn check_builtin_shape(
    program: &MirProgram,
    types: &TypeInterner,
    target: MirLocalId,
    target_ty: TypeId,
    builtin: &BuiltinId,
    arguments: &lpp_mir::ListRange<Operand>,
    mir_id: MirFunctionId,
    scan: &mut ScanState,
) -> Result<(), CodegenError> {
    let check_operand = |operand: &Operand| operand_type(program, types, operand, mir_id);
    let local_ty = |local: MirLocalId| {
        program
            .local(local)
            .expect("validated MIR retains locals")
            .ty
    };
    match *builtin {
        PRINT_STR => {
            scan.imports.insert(IMP_PRINT_STR);
            let args = program.operands(*arguments);
            let descriptor = PRINT_STR.descriptor();
            if args.len() != descriptor.parameters.len() {
                return Err(CodegenError::new(
                    Some(mir_id),
                    CodegenErrorKind::AbiMismatch {
                        symbol: descriptor.symbol.to_owned(),
                        expected_arity: descriptor.parameters.len() as u32,
                        actual_arity: args.len() as u32,
                    },
                ));
            }
            for operand in args {
                check_operand(operand)?;
            }
        }
        LIST_NEW => {
            let TypeKind::List(element) = types.kind(target_ty) else {
                return Err(unsupported(mir_id, "list allocation of non-list"));
            };
            check_value_type(types, element, mir_id)?;
            if is_managed(types, element) {
                scan.imports.insert(IMP_LIST_NEW_ARC);
            } else {
                scan.imports.insert(IMP_LIST_NEW);
            }
            if scan.shared.contains(&target_ty) {
                scan.imports.insert(IMP_MARK_COLLECTIBLE);
                scan.imports.insert(IMP_LIST_TRACE);
            }
            let args = program.operands(*arguments);
            let descriptor = LIST_NEW.descriptor();
            if args.len() != descriptor.parameters.len() {
                return Err(CodegenError::new(
                    Some(mir_id),
                    CodegenErrorKind::AbiMismatch {
                        symbol: descriptor.symbol.to_owned(),
                        expected_arity: descriptor.parameters.len() as u32,
                        actual_arity: args.len() as u32,
                    },
                ));
            }
            for operand in args {
                check_operand(operand)?;
            }
        }
        other => {
            'family: {
                let descriptor = other.descriptor();
                let name = descriptor.name;
                let args = program.operands(*arguments);
                // Family A: print dispatches on
                // the argument's semantic type;
                // the rest import the
                // descriptor's v1 symbol.
                if name == "print" {
                    let printer = match args.first() {
                        Some(Operand::Constant(constant)) => match constant {
                            Constant::Integer(_) | Constant::Character { .. } => IMP_PRINT_INT,
                            Constant::FloatBits(_) => IMP_PRINT_FLOAT,
                            Constant::Bool(_) => IMP_PRINT_BOOL,
                            Constant::String { .. } => IMP_PRINT_STR,
                        },
                        Some(Operand::Copy(local)) => match types.kind(local_ty(*local)) {
                            TypeKind::Primitive(PrimitiveType::Int | PrimitiveType::Char) => {
                                IMP_PRINT_INT
                            }
                            TypeKind::Primitive(PrimitiveType::Float) => IMP_PRINT_FLOAT,
                            TypeKind::Primitive(PrimitiveType::Bool) => IMP_PRINT_BOOL,
                            TypeKind::Primitive(PrimitiveType::String) => IMP_PRINT_STR,
                            _ => {
                                return Err(unsupported(mir_id, "print of unsupported argument"));
                            }
                        },
                        _ => {
                            return Err(unsupported(mir_id, "print of non-local argument"));
                        }
                    };
                    scan.imports.insert(printer);
                    if args.len() != 1 {
                        return Err(unsupported(mir_id, "print arity"));
                    }
                    check_operand(args.first().expect("print argument"))?;
                    break 'family;
                }
                // The list builtins dispatch on
                // the element class (the
                // descriptor's symbol is the
                // scalar spelling).
                if name == "list_push" || name == "list_set" || name == "list_get" || name == "get"
                {
                    let Operand::Copy(list_local) = args[0] else {
                        return Err(unsupported(mir_id, "list builtin of non-local list"));
                    };
                    let TypeKind::List(element) = types.kind(local_ty(list_local)) else {
                        return Err(unsupported(mir_id, "list builtin of non-list"));
                    };
                    if args.len() != descriptor.semantic_parameters.len() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::AbiMismatch {
                                symbol: descriptor.symbol.to_owned(),
                                expected_arity: descriptor.semantic_parameters.len() as u32,
                                actual_arity: args.len() as u32,
                            },
                        ));
                    }
                    for operand in args {
                        check_operand(operand)?;
                    }
                    match element_class(types, element) {
                        ElementClass::Managed => {
                            scan.imports.insert(match name {
                                "list_push" => IMP_LIST_PUSH_ARC,
                                "list_set" => IMP_LIST_SET_ARC,
                                _ => IMP_LIST_GET_ARC,
                            });
                            if name == "list_get" || name == "get" {
                                // An element read
                                // gains a reference.
                                scan.imports.insert(IMP_ARC_RETAIN);
                            }
                            // Function values in
                            // lists carry their
                            // provenance class: a
                            // push records it on
                            // the list, a read
                            // propagates it to the
                            // result local.
                            if matches!(types.kind(element), TypeKind::Function { .. }) {
                                match name {
                                    "list_get" | "get" => {
                                        let class = scan
                                            .value_classes
                                            .get(&(mir_id, list_local))
                                            .copied()
                                            .ok_or_else(|| {
                                                unsupported(
                                                    mir_id,
                                                    "function value of unknown origin",
                                                )
                                            })?;
                                        record_value_class(scan, mir_id, target, class)?;
                                    }
                                    _ => {
                                        if let Operand::Copy(source) = args[1] {
                                            let class = scan
                                                .value_classes
                                                .get(&(mir_id, source))
                                                .copied()
                                                .ok_or_else(|| {
                                                    unsupported(
                                                        mir_id,
                                                        "function value of unknown origin",
                                                    )
                                                })?;
                                            record_value_class(scan, mir_id, list_local, class)?;
                                        }
                                    }
                                }
                            }
                        }
                        ElementClass::Scalar => {
                            scan.imports.insert(match name {
                                "list_push" => IMP_LIST_PUSH,
                                "list_set" => IMP_LIST_SET,
                                _ => IMP_LIST_GET,
                            });
                        }
                        ElementClass::Float => {
                            scan.imports.insert(match name {
                                "list_push" => IMP_LIST_PUSH_FLOAT,
                                "list_set" => IMP_LIST_SET_FLOAT,
                                _ => IMP_LIST_GET_FLOAT,
                            });
                        }
                        ElementClass::Bool => {
                            scan.imports.insert(match name {
                                "list_push" => IMP_LIST_PUSH_BOOL,
                                "list_set" => IMP_LIST_SET_BOOL,
                                _ => IMP_LIST_GET_BOOL,
                            });
                        }
                    }
                    break 'family;
                }
                if is_family_a(name) {
                    if descriptor.symbol.is_empty() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::UnrepresentableBuiltin {
                                builtin: other,
                                reason: "no v1 symbol",
                            },
                        ));
                    }
                    scan.imports.insert(descriptor.symbol);
                    if args.len() != descriptor.semantic_parameters.len() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::AbiMismatch {
                                symbol: descriptor.symbol.to_owned(),
                                expected_arity: descriptor.semantic_parameters.len() as u32,
                                actual_arity: args.len() as u32,
                            },
                        ));
                    }
                    for operand in args {
                        check_operand(operand)?;
                    }
                    break 'family;
                }
                // Family B: SIMD — every entry
                // lowers to native 128-bit
                // instructions except the
                // scalar checksum, which imports
                // the deterministic v1 loop.
                if name.starts_with("vec_") {
                    if name == "vec_i64_checksum" {
                        scan.imports.insert(IMP_VEC_CHECKSUM);
                    }
                    if args.len() != descriptor.semantic_parameters.len() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::AbiMismatch {
                                symbol: descriptor.symbol.to_owned(),
                                expected_arity: descriptor.semantic_parameters.len() as u32,
                                actual_arity: args.len() as u32,
                            },
                        ));
                    }
                    for operand in args {
                        check_operand(operand)?;
                    }
                    break 'family;
                }
                // Family C: slice handles.
                if FAMILY_C.contains(&name) {
                    if descriptor.symbol.is_empty() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::UnrepresentableBuiltin {
                                builtin: other,
                                reason: "no v1 symbol",
                            },
                        ));
                    }
                    scan.imports.insert(descriptor.symbol);
                    // `slice_get` on a `StrSlice` dispatches to
                    // `lpp_str_slice_get` at lowering time (the v1 ABI
                    // has no dedicated builtin), so declare that import
                    // too when the receiver is a string slice.
                    if name == "slice_get"
                        && matches!(
                            args.first().and_then(|operand| {
                                operand_language_kind(program, types, operand)
                            }),
                            Some(TypeKind::Primitive(PrimitiveType::StrSlice))
                        )
                    {
                        scan.imports.insert("lpp_str_slice_get");
                    }
                    if args.len() != descriptor.semantic_parameters.len() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::AbiMismatch {
                                symbol: descriptor.symbol.to_owned(),
                                expected_arity: descriptor.semantic_parameters.len() as u32,
                                actual_arity: args.len() as u32,
                            },
                        ));
                    }
                    for operand in args {
                        check_operand(operand)?;
                    }
                    break 'family;
                }
                // Family MAP: hash map runtime symbols.
                if is_family_map(name) {
                    if descriptor.symbol.is_empty() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::UnrepresentableBuiltin {
                                builtin: other,
                                reason: "no v1 symbol",
                            },
                        ));
                    }
                    scan.imports.insert(descriptor.symbol);
                    if args.len() != descriptor.semantic_parameters.len() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::AbiMismatch {
                                symbol: descriptor.symbol.to_owned(),
                                expected_arity: descriptor.semantic_parameters.len() as u32,
                                actual_arity: args.len() as u32,
                            },
                        ));
                    }
                    for operand in args {
                        check_operand(operand)?;
                    }
                    break 'family;
                }
                // Family NET: host-backed sockets/HTTP.
                if is_family_net(name) {
                    if descriptor.symbol.is_empty() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::UnrepresentableBuiltin {
                                builtin: other,
                                reason: "no v1 symbol",
                            },
                        ));
                    }
                    scan.imports.insert(descriptor.symbol);
                    if args.len() != descriptor.semantic_parameters.len() {
                        return Err(CodegenError::new(
                            Some(mir_id),
                            CodegenErrorKind::AbiMismatch {
                                symbol: descriptor.symbol.to_owned(),
                                expected_arity: descriptor.semantic_parameters.len() as u32,
                                actual_arity: args.len() as u32,
                            },
                        ));
                    }
                    for operand in args {
                        check_operand(operand)?;
                    }
                    break 'family;
                }
                // Family D: typed rejection at
                // the exact function.
                return Err(CodegenError::new(
                    Some(mir_id),
                    CodegenErrorKind::UnrepresentableBuiltin {
                        builtin: other,
                        reason: "not in the 5C2 table policy",
                    },
                ));
            }
        }
    }
    Ok(())
}
