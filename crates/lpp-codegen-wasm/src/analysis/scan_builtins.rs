use super::*;

pub(crate) fn check_builtin(
    lower: &mut ScanLower<'_>,
    builtin: BuiltinId,
    args: &[Operand],
) -> Result<(), CodegenError> {
    let name = builtin.descriptor().name;
    let short = name.strip_prefix("lpp_").unwrap_or(name);

    match short {
        // Argument-free.
        "list_new" => {
            *lower.has_list = true;
            *lower.has_managed = true;
            lower.needs.insert(H::ListNew);
            Ok(())
        }
        "map_new" | "map_new_arc" => {
            lower.needs.insert(H::Alloc);
            lower.needs.insert(H::MapNew);
            Ok(())
        }
        "input" => {
            *lower.has_managed = true;
            lower.needs.insert(H::FdRead);
            lower.needs.insert(H::Input);
            Ok(())
        }
        _ => {
            let arg0 = args
                .first()
                .ok_or_else(|| unsupported("builtin with no argument", Some(lower.fn_id)))?;
            match short {
                "print" => {
                    let class = scan_operand_class(lower, arg0)?;
                    match class {
                        Class::I64 | Class::Char => lower.needs.insert(H::PrintInt),
                        Class::Bool => lower.needs.insert(H::PrintBool),
                        Class::Str => lower.needs.insert(H::PrintStr),
                        Class::F64 => lower.needs.insert(H::PrintFloat),
                        Class::Ptr => {
                            return Err(unsupported("print of managed type", Some(lower.fn_id)));
                        }
                    };
                    Ok(())
                }
                "print_str" | "eprint_str" => {
                    require_str(lower, arg0)?;
                    lower.needs.insert(H::PrintStr);
                    Ok(())
                }
                "print_int" => {
                    require_i64(lower, arg0)?;
                    lower.needs.insert(H::PrintInt);
                    Ok(())
                }
                "print_bool" => {
                    require_bool(lower, arg0)?;
                    lower.needs.insert(H::PrintBool);
                    Ok(())
                }
                "print_float" => {
                    require_f64(lower, arg0)?;
                    lower.needs.insert(H::PrintFloat);
                    Ok(())
                }
                "write_str" => {
                    require_str(lower, arg0)?;
                    lower.needs.insert(H::WriteStr);
                    Ok(())
                }
                "str_len" => {
                    require_str(lower, arg0)?;
                    lower.needs.insert(H::StrLen);
                    Ok(())
                }
                "list_push" | "list_set" => {
                    let element = list_element_of(lower, arg0)?;
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    // list_push: (list, value); list_set: (list, index, value).
                    for arg in args.iter().skip(1) {
                        scan_operand_class(lower, arg)?;
                    }
                    let push = short == "list_push";
                    match element_class(lower.types, element) {
                        ElementClass::I64 => {
                            lower
                                .needs
                                .insert(if push { H::ListPushI64 } else { H::ListSetI64 })
                        }
                        ElementClass::F64 => {
                            lower
                                .needs
                                .insert(if push { H::ListPushF64 } else { H::ListSetF64 })
                        }
                        ElementClass::Ptr => {
                            lower
                                .needs
                                .insert(if push { H::ListPushPtr } else { H::ListSetPtr })
                        }
                    };
                    Ok(())
                }
                "list_get" => {
                    let element = list_element_of(lower, arg0)?;
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("list index argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    match element_class(lower.types, element) {
                        ElementClass::I64 => lower.needs.insert(H::ListGetI64),
                        ElementClass::F64 => lower.needs.insert(H::ListGetF64),
                        ElementClass::Ptr => lower.needs.insert(H::ListGetPtr),
                    };
                    Ok(())
                }
                "list_len" => {
                    list_element_of(lower, arg0)?;
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    lower.needs.insert(H::ListLen);
                    Ok(())
                }
                "slice" | "str_slice" => {
                    let start = args.get(1).ok_or_else(|| {
                        unsupported("slice start argument missing", Some(lower.fn_id))
                    })?;
                    let length = args.get(2).ok_or_else(|| {
                        unsupported("slice length argument missing", Some(lower.fn_id))
                    })?;
                    require_i64(lower, start)?;
                    require_i64(lower, length)?;
                    lower.needs.insert(H::Alloc);
                    if short == "slice" {
                        list_element_of(lower, arg0)?;
                        *lower.has_list = true;
                        *lower.has_managed = true;
                        lower.needs.insert(H::ListLen);
                        lower.needs.insert(H::SliceNewList);
                    } else {
                        require_str(lower, arg0)?;
                        lower.needs.insert(H::StrLen);
                        lower.needs.insert(H::SliceNewStr);
                    }
                    Ok(())
                }
                "slice_len" => {
                    slice_kind_of(lower.program, lower.types, arg0, lower.fn_id)?;
                    lower.needs.insert(H::SliceLen);
                    Ok(())
                }
                "slice_get" => {
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("slice index argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    match slice_kind_of(lower.program, lower.types, arg0, lower.fn_id)? {
                        SliceKind::String => {
                            *lower.has_managed = true;
                            lower.needs.insert(H::Alloc);
                            lower.needs.insert(H::StrSliceGet);
                        }
                        SliceKind::List(element) => {
                            *lower.has_list = true;
                            *lower.has_managed = true;
                            match element_class(lower.types, element) {
                                ElementClass::I64 => lower.needs.insert(H::SliceGetI64),
                                ElementClass::F64 => lower.needs.insert(H::SliceGetF64),
                                ElementClass::Ptr => lower.needs.insert(H::SliceGetPtr),
                            };
                            match element_class(lower.types, element) {
                                ElementClass::I64 => lower.needs.insert(H::ListGetI64),
                                ElementClass::F64 => lower.needs.insert(H::ListGetF64),
                                ElementClass::Ptr => lower.needs.insert(H::ListGetPtr),
                            };
                        }
                    }
                    Ok(())
                }
                "slice_get_bool" => {
                    let index = args.get(1).ok_or_else(|| {
                        unsupported("slice index argument missing", Some(lower.fn_id))
                    })?;
                    require_i64(lower, index)?;
                    let SliceKind::List(element) =
                        slice_kind_of(lower.program, lower.types, arg0, lower.fn_id)?
                    else {
                        return Err(unsupported(
                            "slice_get_bool requires a list slice",
                            Some(lower.fn_id),
                        ));
                    };
                    if !matches!(
                        lower.types.kind(element),
                        TypeKind::Primitive(PrimitiveType::Bool)
                    ) {
                        return Err(unsupported(
                            "slice_get_bool requires boolean elements",
                            Some(lower.fn_id),
                        ));
                    }
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    lower.needs.insert(H::ListGetI64);
                    lower.needs.insert(H::SliceGetI64);
                    Ok(())
                }
                "slice_to_str" | "str_slice_to_str" => {
                    if slice_kind_of(lower.program, lower.types, arg0, lower.fn_id)?
                        != SliceKind::String
                    {
                        return Err(unsupported(
                            "slice_to_str requires a string slice",
                            Some(lower.fn_id),
                        ));
                    }
                    *lower.has_managed = true;
                    lower.needs.insert(H::Alloc);
                    lower.needs.insert(H::StrSliceToStr);
                    Ok(())
                }
                "map_put" | "map_put_str" | "map_put_float" | "map_put_str_float" | "map_get"
                | "map_get_str" | "map_get_float" | "map_get_str_float" | "map_has"
                | "map_has_str" | "map_remove" | "map_remove_str" => {
                    require_i64(lower, arg0)?;
                    let string_keys = short.contains("_str");
                    let float_value = short.ends_with("_float");
                    let is_put = short.starts_with("map_put");
                    let is_get = short.starts_with("map_get");
                    let is_has = short.starts_with("map_has");
                    let key = args.get(1).ok_or_else(|| {
                        unsupported("map key argument missing", Some(lower.fn_id))
                    })?;
                    let expected_key_class = if string_keys { Class::Str } else { Class::I64 };
                    if scan_operand_class(lower, key)? != expected_key_class {
                        return Err(unsupported("map key type mismatch", Some(lower.fn_id)));
                    }
                    if is_put {
                        let value = args.get(2).ok_or_else(|| {
                            unsupported("map value argument missing", Some(lower.fn_id))
                        })?;
                        if float_value {
                            require_f64(lower, value)?;
                        } else {
                            require_i64(lower, value)?;
                        }
                    }
                    let find = if string_keys {
                        lower.needs.insert(H::StrEq);
                        H::MapFindStr
                    } else {
                        H::MapFindI64
                    };
                    lower.needs.insert(find);
                    lower
                        .needs
                        .insert(match (is_put, is_get, is_has, string_keys) {
                            (true, _, _, false) => H::MapPutI64,
                            (true, _, _, true) => H::MapPutStr,
                            (_, true, _, false) => H::MapGetI64,
                            (_, true, _, true) => H::MapGetStr,
                            (_, _, true, false) => H::MapHasI64,
                            (_, _, true, true) => H::MapHasStr,
                            (_, _, _, false) => H::MapRemoveI64,
                            (_, _, _, true) => H::MapRemoveStr,
                        });
                    if is_put {
                        lower.needs.insert(H::MapEnsure);
                        lower.needs.insert(H::Alloc);
                    }
                    Ok(())
                }
                "map_len" => {
                    require_i64(lower, arg0)?;
                    lower.needs.insert(H::MapLen);
                    Ok(())
                }
                // ── 5D2b slice 4, batch 1: integer + float builtins ──
                "abs" | "clz64" | "ctz64" | "popcount64" | "bswap16" | "bswap32" | "bswap64"
                | "trunc_u8" | "trunc_u16" | "trunc_u32" | "trunc_i8" | "trunc_i16"
                | "trunc_i32" => {
                    require_i64(lower, arg0)?;
                    Ok(())
                }
                "min" | "max" | "min_u" | "max_u" | "lt_u" | "le_u" | "gt_u" | "ge_u" | "div_u"
                | "rem_u" | "shr_u" | "shl_u" | "rotl64" | "rotr64" | "rotl32" | "rotr32"
                | "add_wrap" | "sub_wrap" | "mul_wrap" | "add_checked" | "sub_checked"
                | "mul_checked" => {
                    let arg1 = args.get(1).ok_or_else(|| {
                        unsupported("builtin argument missing", Some(lower.fn_id))
                    })?;
                    require_i64(lower, arg0)?;
                    require_i64(lower, arg1)?;
                    Ok(())
                }
                "int_pow" => {
                    let arg1 = args.get(1).ok_or_else(|| {
                        unsupported("builtin argument missing", Some(lower.fn_id))
                    })?;
                    require_i64(lower, arg0)?;
                    require_i64(lower, arg1)?;
                    Ok(())
                }
                "ceil" | "floor" | "sqrt" | "sin" | "cos" => {
                    require_f64(lower, arg0)?;
                    if short == "sin" {
                        lower.needs.insert(H::Sin);
                    } else if short == "cos" {
                        lower.needs.insert(H::Cos);
                    }
                    Ok(())
                }
                "pow" => {
                    let arg1 = args.get(1).ok_or_else(|| {
                        unsupported("builtin argument missing", Some(lower.fn_id))
                    })?;
                    require_f64(lower, arg0)?;
                    require_f64(lower, arg1)?;
                    lower.needs.insert(H::Log2);
                    lower.needs.insert(H::Exp2);
                    lower.needs.insert(H::Pow);
                    Ok(())
                }
                "fmod" => {
                    let arg1 = args.get(1).ok_or_else(|| {
                        unsupported("builtin argument missing", Some(lower.fn_id))
                    })?;
                    require_f64(lower, arg0)?;
                    require_f64(lower, arg1)?;
                    Ok(())
                }
                // ── string construction, indexing, and splitting ──
                "str_substr" => {
                    require_str(lower, arg0)?;
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    require_i64(
                        lower,
                        args.get(2).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    *lower.has_managed = true;
                    lower.needs.insert(H::StrSubstr);
                    Ok(())
                }
                "str_repeat" | "char_at" => {
                    require_str(lower, arg0)?;
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    *lower.has_managed = true;
                    lower.needs.insert(if short == "str_repeat" {
                        H::StrRepeat
                    } else {
                        H::CharAt
                    });
                    Ok(())
                }
                "str_split" => {
                    require_str(lower, arg0)?;
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    lower.needs.insert(H::StrSplit);
                    lower.needs.insert(H::ListNew);
                    lower.needs.insert(H::ListPushPtr);
                    Ok(())
                }
                "ord" => {
                    require_str(lower, arg0)?;
                    lower.needs.insert(H::Ord);
                    Ok(())
                }
                "chr" => {
                    require_i64(lower, arg0)?;
                    *lower.has_managed = true;
                    lower.needs.insert(H::Chr);
                    Ok(())
                }
                "str_eq" => {
                    require_str(lower, arg0)?;
                    require_str(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    lower.needs.insert(H::StrEq);
                    Ok(())
                }
                // ── 5D2b slice 4, batch 3: string builtins ──
                "str_concat" | "str_replace" | "str_trim" | "str_to_lower" | "str_lower"
                | "str_to_upper" | "str_upper" | "int_to_str" | "str_to_int" | "float_to_str"
                | "bool_to_str" | "u64_to_str" | "u64_to_hex" | "str_to_u64" | "str_contains"
                | "str_starts_with" | "str_ends_with" | "str_find" => {
                    match short {
                        "str_concat" => {
                            let arg1 = args.get(1).ok_or_else(|| {
                                unsupported("builtin argument missing", Some(lower.fn_id))
                            })?;
                            require_str(lower, arg0)?;
                            require_str(lower, arg1)?;
                        }
                        "str_replace" => {
                            let arg1 = args.get(1).ok_or_else(|| {
                                unsupported("builtin argument missing", Some(lower.fn_id))
                            })?;
                            let a2 = args.get(2).ok_or_else(|| {
                                unsupported("builtin argument missing", Some(lower.fn_id))
                            })?;
                            require_str(lower, arg0)?;
                            require_str(lower, arg1)?;
                            require_str(lower, a2)?;
                        }
                        "str_contains" | "str_starts_with" | "str_ends_with" | "str_find" => {
                            let arg1 = args.get(1).ok_or_else(|| {
                                unsupported("builtin argument missing", Some(lower.fn_id))
                            })?;
                            require_str(lower, arg0)?;
                            require_str(lower, arg1)?;
                        }
                        "str_to_int" | "str_to_u64" | "str_trim" | "str_to_lower" | "str_lower"
                        | "str_to_upper" | "str_upper" | "int_to_str" | "float_to_str"
                        | "bool_to_str" | "u64_to_str" | "u64_to_hex" => match short {
                            "float_to_str" => require_f64(lower, arg0)?,
                            "bool_to_str" => require_bool(lower, arg0)?,
                            "int_to_str" | "u64_to_str" | "u64_to_hex" => require_i64(lower, arg0)?,
                            _ => require_str(lower, arg0)?,
                        },
                        _ => unreachable!("string builtin guarded by the outer match"),
                    }
                    // Every string-producing builtin allocates on the ARC
                    // bump heap, which needs the managed machinery.
                    if !matches!(
                        short,
                        "str_contains"
                            | "str_starts_with"
                            | "str_ends_with"
                            | "str_find"
                            | "str_to_int"
                            | "str_to_u64"
                    ) {
                        *lower.has_managed = true;
                        lower.needs.insert(H::Alloc);
                    }
                    match short {
                        "str_concat" => {
                            lower.needs.insert(H::StrConcat);
                        }
                        "str_replace" => {
                            lower.needs.insert(H::StrReplace);
                        }
                        "str_trim" => {
                            lower.needs.insert(H::StrTrim);
                        }
                        "str_to_lower" | "str_lower" => {
                            lower.needs.insert(H::StrLower);
                        }
                        "str_to_upper" | "str_upper" => {
                            lower.needs.insert(H::StrUpper);
                        }
                        "int_to_str" => {
                            lower.needs.insert(H::IntToStr);
                        }
                        "str_to_int" => {
                            lower.needs.insert(H::StrToInt);
                        }
                        "float_to_str" => {
                            lower.needs.insert(H::FloatToStr);
                        }
                        "bool_to_str" => {
                            lower.needs.insert(H::BoolToStr);
                        }
                        "u64_to_str" => {
                            lower.needs.insert(H::U64ToStr);
                        }
                        "u64_to_hex" => {
                            lower.needs.insert(H::U64ToHex);
                        }
                        "str_to_u64" => {
                            lower.needs.insert(H::StrToU64);
                        }
                        "str_contains" => {
                            lower.needs.insert(H::StrContains);
                        }
                        "str_starts_with" => {
                            lower.needs.insert(H::StrStartsWith);
                        }
                        "str_ends_with" => {
                            lower.needs.insert(H::StrEndsWith);
                        }
                        "str_find" => {
                            lower.needs.insert(H::StrFind);
                        }
                        _ => {}
                    };
                    Ok(())
                }
                _ => Err(unrepresentable(
                    builtin,
                    "not representable on wasm32-wasip1",
                    lower.fn_id,
                )),
            }
        }
    }
}
