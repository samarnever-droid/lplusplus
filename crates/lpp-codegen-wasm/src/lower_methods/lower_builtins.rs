use super::*;

impl<'a> FnLower<'a> {
    pub(super) fn emit_builtin(
        &self,
        fb: &mut FB,
        builtin: BuiltinId,
        arguments: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let args = self.env.program.operands(*arguments);
        let name = builtin.descriptor().name;
        let short = name.strip_prefix("lpp_").unwrap_or(name);
        if short == "input" {
            self.call_helper(fb, H::Input)?;
            return Ok(Class::Str);
        }
        let arg0 = args
            .first()
            .ok_or_else(|| unsupported("builtin with no argument", Some(self.fn_id)))?;
        match short {
            "print" => {
                let class = operand_class(self.env.program, self.env.types, arg0)?;
                match class {
                    Class::I64 => {
                        self.emit_operand(fb, arg0)?;
                    }
                    Class::Char => {
                        self.emit_operand(fb, arg0)?;
                        fb.op(op::I64_EXTEND_I32_U);
                    }
                    Class::Bool => {
                        self.emit_operand(fb, arg0)?;
                    }
                    Class::Str => {
                        self.emit_operand(fb, arg0)?;
                    }
                    Class::F64 => {
                        self.emit_operand(fb, arg0)?;
                    }
                    Class::Ptr => {
                        return Err(unsupported("print of managed type", Some(self.fn_id)));
                    }
                }
                let h = match class {
                    Class::I64 | Class::Char => H::PrintInt,
                    Class::Bool => H::PrintBool,
                    Class::Str => H::PrintStr,
                    Class::F64 => H::PrintFloat,
                    Class::Ptr => unreachable!(),
                };
                self.call_helper(fb, h)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "print_str" | "eprint_str" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::PrintStr)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "print_int" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::PrintInt)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "print_bool" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::PrintBool)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "print_float" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::PrintFloat)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "write_str" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::WriteStr)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "str_len" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::StrLen)?;
                Ok(Class::I64)
            }
            "list_new" => {
                // The element class comes from the assign target
                // (`emit_assign` routes here); the helper's is-ARC word
                // tells the list destroyer whether to release elements.
                Err(unsupported("list_new outside an assign", Some(self.fn_id)))
            }
            "list_push" | "list_set" => {
                let value = args.get(1).ok_or_else(|| {
                    unsupported("list element argument missing", Some(self.fn_id))
                })?;
                let element = self.list_element_of(arg0)?;
                let is_push = short == "list_push";
                // The list itself is a plain read (the helper never takes
                // ownership of the container).
                self.emit_operand(fb, arg0)?;
                if !is_push {
                    let index = args.get(1).ok_or_else(|| {
                        unsupported("list index argument missing", Some(self.fn_id))
                    })?;
                    let value = args.get(2).ok_or_else(|| {
                        unsupported("list element argument missing", Some(self.fn_id))
                    })?;
                    self.emit_operand(fb, index)?;
                    match element_class(self.env.types, element) {
                        ElementClass::I64 => {
                            self.emit_operand(fb, value)?;
                            widen_to_i64(fb, self.env.types, element);
                            self.call_helper(fb, H::ListSetI64)?;
                        }
                        ElementClass::F64 => {
                            self.emit_operand(fb, value)?;
                            self.call_helper(fb, H::ListSetF64)?;
                        }
                        ElementClass::Ptr => {
                            // The helper releases the replaced element
                            // and retains the new one (runtime-internal).
                            self.emit_operand(fb, value)?;
                            self.call_helper(fb, H::ListSetPtr)?;
                        }
                    }
                } else {
                    match element_class(self.env.types, element) {
                        ElementClass::I64 => {
                            // The element transfers into the list (the
                            // list owns the new reference).
                            self.emit_owned(fb, value)?;
                            widen_to_i64(fb, self.env.types, element);
                            self.call_helper(fb, H::ListPushI64)?;
                        }
                        ElementClass::F64 => {
                            self.emit_owned(fb, value)?;
                            self.call_helper(fb, H::ListPushF64)?;
                        }
                        ElementClass::Ptr => {
                            self.emit_owned(fb, value)?;
                            self.call_helper(fb, H::ListPushPtr)?;
                        }
                    }
                }
                fb.i64c(0);
                Ok(Class::I64)
            }
            "list_get" => {
                let element = self.list_element_of(arg0)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(
                    fb,
                    args.get(1).ok_or_else(|| {
                        unsupported("list index argument missing", Some(self.fn_id))
                    })?,
                )?;
                match element_class(self.env.types, element) {
                    ElementClass::I64 => {
                        self.call_helper(fb, H::ListGetI64)?;
                        Ok(Class::I64)
                    }
                    ElementClass::F64 => {
                        self.call_helper(fb, H::ListGetF64)?;
                        Ok(Class::F64)
                    }
                    ElementClass::Ptr => {
                        self.call_helper(fb, H::ListGetPtr)?;
                        // An element read gains a reference (the
                        // container keeps its own).
                        self.call_helper(fb, H::Retain)?;
                        Ok(Class::Ptr)
                    }
                }
            }
            "list_len" => {
                self.list_element_of(arg0)?;
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::ListLen)?;
                Ok(Class::I64)
            }
            "slice" | "str_slice" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(
                    fb,
                    args.get(1).ok_or_else(|| {
                        unsupported("slice start argument missing", Some(self.fn_id))
                    })?,
                )?;
                self.emit_operand(
                    fb,
                    args.get(2).ok_or_else(|| {
                        unsupported("slice length argument missing", Some(self.fn_id))
                    })?,
                )?;
                self.call_helper(
                    fb,
                    if short == "slice" {
                        H::SliceNewList
                    } else {
                        H::SliceNewStr
                    },
                )?;
                Ok(Class::Ptr)
            }
            "slice_len" => {
                slice_kind_of(self.env.program, self.env.types, arg0, self.fn_id)?;
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::SliceLen)?;
                Ok(Class::I64)
            }
            "slice_get" => {
                let kind = slice_kind_of(self.env.program, self.env.types, arg0, self.fn_id)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(
                    fb,
                    args.get(1).ok_or_else(|| {
                        unsupported("slice index argument missing", Some(self.fn_id))
                    })?,
                )?;
                match kind {
                    SliceKind::String => {
                        self.call_helper(fb, H::StrSliceGet)?;
                        Ok(Class::Str)
                    }
                    SliceKind::List(element) => match class_of_type(self.env.types, element)? {
                        Class::I64 => {
                            self.call_helper(fb, H::SliceGetI64)?;
                            Ok(Class::I64)
                        }
                        Class::Bool => {
                            self.call_helper(fb, H::SliceGetI64)?;
                            fb.op(op::I32_WRAP_I64);
                            Ok(Class::Bool)
                        }
                        Class::Char => {
                            self.call_helper(fb, H::SliceGetI64)?;
                            fb.op(op::I32_WRAP_I64);
                            Ok(Class::Char)
                        }
                        Class::F64 => {
                            self.call_helper(fb, H::SliceGetF64)?;
                            Ok(Class::F64)
                        }
                        Class::Str => {
                            self.call_helper(fb, H::SliceGetPtr)?;
                            self.call_helper(fb, H::Retain)?;
                            Ok(Class::Str)
                        }
                        Class::Ptr => {
                            self.call_helper(fb, H::SliceGetPtr)?;
                            self.call_helper(fb, H::Retain)?;
                            Ok(Class::Ptr)
                        }
                    },
                }
            }
            "slice_get_bool" => {
                let SliceKind::List(element) =
                    slice_kind_of(self.env.program, self.env.types, arg0, self.fn_id)?
                else {
                    return Err(unsupported(
                        "slice_get_bool requires a list slice",
                        Some(self.fn_id),
                    ));
                };
                if !matches!(
                    self.env.types.kind(element),
                    TypeKind::Primitive(PrimitiveType::Bool)
                ) {
                    return Err(unsupported(
                        "slice_get_bool requires boolean elements",
                        Some(self.fn_id),
                    ));
                }
                self.emit_operand(fb, arg0)?;
                self.emit_operand(
                    fb,
                    args.get(1).ok_or_else(|| {
                        unsupported("slice index argument missing", Some(self.fn_id))
                    })?,
                )?;
                self.call_helper(fb, H::SliceGetI64)?;
                fb.op(op::I32_WRAP_I64);
                Ok(Class::Bool)
            }
            "slice_to_str" | "str_slice_to_str" => {
                if slice_kind_of(self.env.program, self.env.types, arg0, self.fn_id)?
                    != SliceKind::String
                {
                    return Err(unsupported(
                        "slice_to_str requires a string slice",
                        Some(self.fn_id),
                    ));
                }
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::StrSliceToStr)?;
                Ok(Class::Str)
            }
            "map_put" | "map_put_str" | "map_put_float" | "map_put_str_float" | "map_get"
            | "map_get_str" | "map_get_float" | "map_get_str_float" | "map_has" | "map_has_str"
            | "map_remove" | "map_remove_str" => {
                let string_keys = short.contains("_str");
                let float_value = short.ends_with("_float");
                let is_put = short.starts_with("map_put");
                let is_get = short.starts_with("map_get");
                let is_has = short.starts_with("map_has");
                self.emit_operand(fb, arg0)?;
                fb.op(op::I32_WRAP_I64);
                self.emit_operand(
                    fb,
                    args.get(1)
                        .ok_or_else(|| unsupported("map key argument missing", Some(self.fn_id)))?,
                )?;
                let helper = match (is_put, is_get, is_has, string_keys) {
                    (true, _, _, false) => H::MapPutI64,
                    (true, _, _, true) => H::MapPutStr,
                    (_, true, _, false) => H::MapGetI64,
                    (_, true, _, true) => H::MapGetStr,
                    (_, _, true, false) => H::MapHasI64,
                    (_, _, true, true) => H::MapHasStr,
                    (_, _, _, false) => H::MapRemoveI64,
                    (_, _, _, true) => H::MapRemoveStr,
                };
                if is_put {
                    self.emit_operand(
                        fb,
                        args.get(2).ok_or_else(|| {
                            unsupported("map value argument missing", Some(self.fn_id))
                        })?,
                    )?;
                    if float_value {
                        fb.op(op::I64_REINTERPRET_F64);
                    }
                }
                self.call_helper(fb, helper)?;
                if is_has {
                    fb.op(op::I32_WRAP_I64);
                    Ok(Class::Bool)
                } else if is_get && float_value {
                    fb.op(op::F64_REINTERPRET_I64);
                    Ok(Class::F64)
                } else {
                    if is_put || short.starts_with("map_remove") {
                        fb.i64c(0);
                    }
                    Ok(Class::I64)
                }
            }
            "map_len" => {
                self.emit_operand(fb, arg0)?;
                fb.op(op::I32_WRAP_I64);
                self.call_helper(fb, H::MapLen)?;
                Ok(Class::I64)
            }
            // ── 5D2b slice 4, batch 1: integer + float builtins ──────
            "abs" => {
                // No single-op i64.abs in the core spec: the mask
                // trick — m = x >> 63 (all 0s or all 1s), and
                // abs(x) = (x ^ m) - m.
                let m = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.t(m);
                fb.g(m).i64c(63).op(op::I64_SHR_S).s(m);
                fb.g(m).op(op::I64_XOR).g(m).op(op::I64_SUB);
                Ok(Class::I64)
            }
            "clz64" | "ctz64" | "popcount64" => {
                self.emit_operand(fb, arg0)?;
                fb.op(match short {
                    "clz64" => op::I64_CLZ,
                    "ctz64" => op::I64_CTZ,
                    _ => op::I64_POPCNT,
                });
                Ok(Class::I64)
            }
            "bswap64" => {
                // No i64.bswap opcode: reverse the 8 byte lanes and
                // reassemble.
                let x = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.s(x);
                fb.g(x)
                    .i64c(0x00000000000000FF)
                    .op(op::I64_AND)
                    .i64c(56)
                    .op(op::I64_SHL);
                fb.g(x)
                    .i64c(0x000000000000FF00)
                    .op(op::I64_AND)
                    .i64c(40)
                    .op(op::I64_SHL)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x0000000000FF0000)
                    .op(op::I64_AND)
                    .i64c(24)
                    .op(op::I64_SHL)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x00000000FF000000)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHL)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x000000FF00000000)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x0000FF0000000000)
                    .op(op::I64_AND)
                    .i64c(24)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x00FF000000000000)
                    .op(op::I64_AND)
                    .i64c(40)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0xFF00000000000000u64 as i64)
                    .op(op::I64_AND)
                    .i64c(56)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                Ok(Class::I64)
            }
            "bswap32" => {
                // Zero-extended 32-bit result (the oracle: u32 as i64).
                let x = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.s(x);
                fb.g(x).i64c(0xFF).op(op::I64_AND).i64c(24).op(op::I64_SHL);
                fb.g(x)
                    .i64c(0xFF00)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHL)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0xFF0000)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0xFF000000)
                    .op(op::I64_AND)
                    .i64c(24)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                Ok(Class::I64)
            }
            "bswap16" => {
                let x = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.s(x);
                fb.g(x).i64c(0xFF).op(op::I64_AND).i64c(8).op(op::I64_SHL);
                fb.g(x)
                    .i64c(0xFF00)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                Ok(Class::I64)
            }
            "trunc_u8" | "trunc_u16" | "trunc_u32" => {
                self.emit_operand(fb, arg0)?;
                fb.i64c(match short {
                    "trunc_u8" => 0xFF,
                    "trunc_u16" => 0xFFFF,
                    _ => 0xFFFFFFFF,
                })
                .op(op::I64_AND);
                Ok(Class::I64)
            }
            "trunc_i8" | "trunc_i16" | "trunc_i32" => {
                let shift = match short {
                    "trunc_i8" => 56,
                    "trunc_i16" => 48,
                    _ => 32,
                };
                self.emit_operand(fb, arg0)?;
                fb.i64c(shift).op(op::I64_SHL).i64c(shift).op(op::I64_SHR_S);
                Ok(Class::I64)
            }
            "min" | "max" | "min_u" | "max_u" => {
                let a = fb.scratch(Val::I64);
                let b = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.t(a);
                self.emit_operand(fb, arg1)?;
                fb.t(b);
                fb.g(a).g(b).op(match short {
                    "min" => op::I64_LT_S,
                    "max" => op::I64_GT_S,
                    "min_u" => op::I64_LT_U,
                    _ => op::I64_GT_U,
                });
                fb.op(op::SELECT);
                Ok(Class::I64)
            }
            "lt_u" | "le_u" | "gt_u" | "ge_u" => {
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.op(match short {
                    "lt_u" => op::I64_LT_U,
                    "le_u" => op::I64_LE_U,
                    "gt_u" => op::I64_GT_U,
                    _ => op::I64_GE_U,
                });
                // The frozen ABI spells these predicates as i64 0/1
                // (the interpreter follows that contract). Materialize the
                // wasm comparison's i32 result into the ABI word so stores,
                // equality checks, and returns keep the same type.
                fb.op(op::I64_EXTEND_I32_U);
                Ok(Class::I64)
            }
            "div_u" | "rem_u" => {
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.op(if short == "div_u" {
                    op::I64_DIV_U
                } else {
                    op::I64_REM_U
                });
                Ok(Class::I64)
            }
            "shr_u" | "shl_u" => {
                // The oracle returns 0 for shifts outside [0, 64);
                // wasm masks the amount to 6 bits, so guard it.
                let amt = fb.scratch(Val::I64);
                let out = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.s(amt);
                fb.g(amt)
                    .i64c(63)
                    .op(op::I64_AND)
                    .op(if short == "shr_u" {
                        op::I64_SHR_U
                    } else {
                        op::I64_SHL
                    })
                    .s(out);
                fb.i64c(0);
                fb.g(out);
                fb.g(amt).i64c(0).op(op::I64_LT_S);
                fb.g(amt).i64c(64).op(op::I64_GE_S);
                fb.op(op::I32_OR);
                fb.op(op::SELECT);
                Ok(Class::I64)
            }
            "rotl64" | "rotr64" => {
                // The value goes on the bottom, the amount on top
                // (masked to 6 bits by the instruction — the oracle
                // masks with 63, same effect).
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.op(if short == "rotl64" {
                    op::I64_ROTL
                } else {
                    op::I64_ROTR
                });
                Ok(Class::I64)
            }
            "rotl32" | "rotr32" => {
                // Zero-extended 32-bit result (the oracle: u32 as i64);
                // value on the bottom, amount on top.
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.op(op::I32_WRAP_I64);
                self.emit_operand(fb, arg1)?;
                fb.op(op::I32_WRAP_I64);
                fb.op(if short == "rotl32" {
                    op::I32_ROTL
                } else {
                    op::I32_ROTR
                });
                fb.op(op::I64_EXTEND_I32_U);
                Ok(Class::I64)
            }
            "add_wrap" | "sub_wrap" | "mul_wrap" => {
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.op(match short {
                    "add_wrap" => op::I64_ADD,
                    "sub_wrap" => op::I64_SUB,
                    _ => op::I64_MUL,
                });
                Ok(Class::I64)
            }
            "add_checked" => {
                let a = fb.scratch(Val::I64);
                let b = fb.scratch(Val::I64);
                let r = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.s(a);
                self.emit_operand(fb, arg1)?;
                fb.s(b);
                fb.g(a).g(b).op(op::I64_ADD).s(r);
                // Overflow iff sign(a) == sign(b) != sign(r):
                // ((a & b) & ~r) < 0.
                fb.g(a)
                    .g(b)
                    .op(op::I64_AND)
                    .g(r)
                    .i64c(-1)
                    .op(op::I64_XOR)
                    .op(op::I64_AND)
                    .i64c(0)
                    .op(op::I64_LT_S);
                fb.if_();
                fb.op(op::UNREACHABLE);
                fb.end();
                fb.g(r);
                Ok(Class::I64)
            }
            "sub_checked" => {
                let a = fb.scratch(Val::I64);
                let b = fb.scratch(Val::I64);
                let r = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.s(a);
                self.emit_operand(fb, arg1)?;
                fb.s(b);
                fb.g(a).g(b).op(op::I64_SUB).s(r);
                // Overflow iff sign(a) != sign(b) and
                // sign(r) != sign(a).
                fb.g(a)
                    .g(b)
                    .op(op::I64_XOR)
                    .i64c(0)
                    .op(op::I64_LT_S)
                    .g(a)
                    .g(r)
                    .op(op::I64_XOR)
                    .i64c(0)
                    .op(op::I64_LT_S);
                fb.op(op::I32_AND);
                fb.if_();
                fb.op(op::UNREACHABLE);
                fb.end();
                fb.g(r);
                Ok(Class::I64)
            }
            "mul_checked" => {
                let a = fb.scratch(Val::I64);
                let b = fb.scratch(Val::I64);
                let r = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.s(a);
                self.emit_operand(fb, arg1)?;
                fb.s(b);
                fb.g(a).g(b).op(op::I64_MUL).s(r);
                // If neither operand is zero, the product is valid iff
                // r / a == b (the division only runs when a != 0).
                fb.g(a).i64c(0).op(op::I64_NE);
                fb.g(b).i64c(0).op(op::I64_NE);
                fb.op(op::I32_AND);
                fb.if_();
                fb.g(r).g(a).op(op::I64_DIV_S).g(b).op(op::I64_NE);
                fb.if_();
                fb.op(op::UNREACHABLE);
                fb.end();
                fb.end();
                fb.g(r);
                Ok(Class::I64)
            }
            "int_pow" => {
                let base = fb.scratch(Val::I64);
                let exp = fb.scratch(Val::I64);
                let result = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.s(base);
                self.emit_operand(fb, self.arg1(args)?)?;
                fb.s(exp);
                // Negative integer exponents are unrepresentable and yield 0.
                fb.g(exp).i64c(0).op(op::I64_LT_S).if_();
                fb.i64c(0).s(result);
                fb.else_();
                fb.i64c(1).s(result);
                fb.block();
                fb.loop_();
                fb.g(exp).op(op::I64_EQZ).br_if(1);
                fb.g(exp).i64c(1).op(op::I64_AND).op(op::I32_WRAP_I64).if_();
                fb.g(result).g(base).op(op::I64_MUL).s(result);
                fb.end();
                fb.g(base).g(base).op(op::I64_MUL).s(base);
                fb.g(exp).i64c(1).op(op::I64_SHR_U).s(exp);
                fb.br(0);
                fb.end();
                fb.end();
                fb.end();
                fb.g(result);
                Ok(Class::I64)
            }
            "ceil" | "floor" | "sqrt" => {
                self.emit_operand(fb, arg0)?;
                fb.op(match short {
                    "ceil" => op::F64_CEIL,
                    "floor" => op::F64_FLOOR,
                    _ => op::F64_SQRT,
                });
                Ok(Class::F64)
            }
            "pow" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.call_helper(fb, H::Pow)?;
                Ok(Class::F64)
            }
            "sin" | "cos" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, if short == "sin" { H::Sin } else { H::Cos })?;
                Ok(Class::F64)
            }
            "fmod" => {
                // No f64.fmod opcode: the oracle's exact formula,
                // a - floor(a / b) * b.
                let a = fb.scratch(Val::F64);
                let b = fb.scratch(Val::F64);
                let q = fb.scratch(Val::F64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.s(a);
                self.emit_operand(fb, arg1)?;
                fb.s(b);
                fb.g(a)
                    .g(b)
                    .op(op::F64_DIV)
                    .op(op::F64_FLOOR)
                    .g(b)
                    .op(op::F64_MUL)
                    .s(q);
                fb.g(a).g(q).op(op::F64_SUB);
                Ok(Class::F64)
            }
            // ── string construction, indexing, and splitting ──
            "str_substr" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.emit_operand(
                    fb,
                    args.get(2)
                        .ok_or_else(|| unsupported("builtin argument missing", Some(self.fn_id)))?,
                )?;
                self.call_helper(fb, H::StrSubstr)?;
                Ok(Class::Str)
            }
            "str_repeat" | "char_at" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.call_helper(
                    fb,
                    if short == "str_repeat" {
                        H::StrRepeat
                    } else {
                        H::CharAt
                    },
                )?;
                Ok(Class::Str)
            }
            "str_split" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.call_helper(fb, H::StrSplit)?;
                Ok(Class::Ptr)
            }
            "ord" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::Ord)?;
                Ok(Class::I64)
            }
            "chr" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::Chr)?;
                Ok(Class::Str)
            }
            "str_eq" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.call_helper(fb, H::StrEq)?;
                fb.op(op::I64_EXTEND_I32_U);
                Ok(Class::I64)
            }
            // ── 5D2b slice 4, batch 3: string builtins ──
            "str_concat" | "str_replace" | "str_contains" | "str_starts_with" | "str_ends_with"
            | "str_find" | "str_trim" | "str_to_lower" | "str_lower" | "str_to_upper"
            | "str_upper" | "int_to_str" | "str_to_int" | "float_to_str" | "bool_to_str"
            | "u64_to_str" | "u64_to_hex" | "str_to_u64" => match short {
                "str_concat" => {
                    let arg1 = self.arg1(args)?;
                    self.emit_operand(fb, arg0)?;
                    self.emit_operand(fb, arg1)?;
                    self.call_helper(fb, H::StrConcat)?;
                    Ok(Class::Ptr)
                }
                "str_replace" => {
                    let arg1 = self.arg1(args)?;
                    let arg2 = args
                        .get(2)
                        .ok_or_else(|| unsupported("builtin argument missing", Some(self.fn_id)))?;
                    self.emit_operand(fb, arg0)?;
                    self.emit_operand(fb, arg1)?;
                    self.emit_operand(fb, arg2)?;
                    self.call_helper(fb, H::StrReplace)?;
                    Ok(Class::Ptr)
                }
                "str_contains" | "str_starts_with" | "str_ends_with" | "str_find" => {
                    let arg1 = self.arg1(args)?;
                    self.emit_operand(fb, arg0)?;
                    self.emit_operand(fb, arg1)?;
                    match short {
                        "str_contains" => {
                            self.call_helper(fb, H::StrContains)?;
                            Ok(Class::Bool)
                        }
                        "str_starts_with" => {
                            self.call_helper(fb, H::StrStartsWith)?;
                            Ok(Class::Bool)
                        }
                        "str_ends_with" => {
                            self.call_helper(fb, H::StrEndsWith)?;
                            Ok(Class::Bool)
                        }
                        _ => {
                            self.call_helper(fb, H::StrFind)?;
                            Ok(Class::I64)
                        }
                    }
                }
                "str_trim" | "str_to_lower" | "str_lower" | "str_to_upper" | "str_upper"
                | "int_to_str" | "str_to_int" | "float_to_str" | "bool_to_str" | "u64_to_str"
                | "u64_to_hex" | "str_to_u64" => {
                    self.emit_operand(fb, arg0)?;
                    match short {
                        "str_trim" => {
                            self.call_helper(fb, H::StrTrim)?;
                            Ok(Class::Ptr)
                        }
                        "str_to_lower" | "str_lower" => {
                            self.call_helper(fb, H::StrLower)?;
                            Ok(Class::Ptr)
                        }
                        "str_to_upper" | "str_upper" => {
                            self.call_helper(fb, H::StrUpper)?;
                            Ok(Class::Ptr)
                        }
                        "int_to_str" => {
                            self.call_helper(fb, H::IntToStr)?;
                            Ok(Class::Ptr)
                        }
                        "str_to_int" => {
                            self.call_helper(fb, H::StrToInt)?;
                            Ok(Class::I64)
                        }
                        "float_to_str" => {
                            self.call_helper(fb, H::FloatToStr)?;
                            Ok(Class::Ptr)
                        }
                        "bool_to_str" => {
                            self.call_helper(fb, H::BoolToStr)?;
                            Ok(Class::Ptr)
                        }
                        "u64_to_str" => {
                            self.call_helper(fb, H::U64ToStr)?;
                            Ok(Class::Ptr)
                        }
                        "u64_to_hex" => {
                            self.call_helper(fb, H::U64ToHex)?;
                            Ok(Class::Ptr)
                        }
                        _ => {
                            self.call_helper(fb, H::StrToU64)?;
                            Ok(Class::I64)
                        }
                    }
                }
                _ => {
                    unreachable!("string builtin guarded by the outer match")
                }
            },
            _ => Err(unrepresentable(
                builtin,
                "not representable on wasm32-wasip1",
                self.fn_id,
            )),
        }
    }
}
