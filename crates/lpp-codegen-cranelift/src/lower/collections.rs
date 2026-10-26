use super::*;

impl<'m> Lowering<'m> {
    /// Family B: the SIMD entries lowered to native 128-bit
    /// instructions (the 5B deferral lifted) — the v1 cranelift
    /// backend's `lower_vector_builtin` semantics. Only the scalar
    /// checksum imports the deterministic v1 loop.
    pub(super) fn vector_from_lanes(
        &mut self,
        builder: &mut FunctionBuilder,
        lanes: [Value; 2],
    ) -> Value {
        let value = builder.ins().splat(cltypes::I64X2, lanes[0]);
        builder.ins().insertlane(value, lanes[1], 1u8)
    }

    pub(super) fn lower_simd(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        builtin: BuiltinId,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        let value = match name {
            "vec_i64x2" => {
                let lanes = [
                    self.operand_value(inst, builder, &args[0])?,
                    self.operand_value(inst, builder, &args[1])?,
                ];
                Some(self.vector_from_lanes(builder, lanes))
            }
            "vec_i64x2_splat" => {
                let lane = self.operand_value(inst, builder, &args[0])?;
                Some(self.vector_from_lanes(builder, [lane, lane]))
            }
            "vec_i64x2_not" => {
                let val = self.operand_value(inst, builder, &args[0])?;
                Some(builder.ins().bnot(val))
            }
            "vec_i64x2_add" | "vec_i64x2_sub" | "vec_i64x2_mul" | "vec_i64x2_xor"
            | "vec_i64x2_and" | "vec_i64x2_or" => {
                let left = self.operand_value(inst, builder, &args[0])?;
                let right = self.operand_value(inst, builder, &args[1])?;
                Some(match name {
                    "vec_i64x2_add" => builder.ins().iadd(left, right),
                    "vec_i64x2_sub" => builder.ins().isub(left, right),
                    "vec_i64x2_mul" => builder.ins().imul(left, right),
                    "vec_i64x2_and" => builder.ins().band(left, right),
                    "vec_i64x2_or" => builder.ins().bor(left, right),
                    _ => builder.ins().bxor(left, right),
                })
            }
            "vec_i64x2_shr" => {
                let shift = match &args[1] {
                    Operand::Constant(Constant::Integer(value)) => *value,
                    _ => {
                        return Err(unsupported(
                            inst.mir_id,
                            "vector shift amount must be a constant integer",
                        ));
                    }
                };
                let left = self.operand_value(inst, builder, &args[0])?;
                let mut lanes = [builder.ins().iconst(cltypes::I64, 0); 2];
                for lane in 0..2u8 {
                    let item = builder.ins().extractlane(left, lane);
                    lanes[lane as usize] = builder.ins().sshr_imm(item, shift);
                }
                Some(self.vector_from_lanes(builder, lanes))
            }
            "vec_i64x2_shr_var" => {
                let left = self.operand_value(inst, builder, &args[0])?;
                let right = self.operand_value(inst, builder, &args[1])?;
                let left0 = builder.ins().extractlane(left, 0);
                let left1 = builder.ins().extractlane(left, 1);
                let right0 = builder.ins().extractlane(right, 0);
                let right1 = builder.ins().extractlane(right, 1);
                let lanes = [
                    builder.ins().sshr(left0, right0),
                    builder.ins().sshr(left1, right1),
                ];
                Some(self.vector_from_lanes(builder, lanes))
            }
            "vec_i64x2_extract" => {
                let lane = match &args[1] {
                    Operand::Constant(Constant::Integer(index)) if (0..2).contains(index) => {
                        *index as u8
                    }
                    _ => {
                        return Err(unsupported(
                            inst.mir_id,
                            "vector extract lane must be a constant 0 or 1",
                        ));
                    }
                };
                let value = self.operand_value(inst, builder, &args[0])?;
                Some(builder.ins().extractlane(value, lane))
            }
            "vec_i64x2_sum" => {
                let value = self.operand_value(inst, builder, &args[0])?;
                let mut result = builder.ins().extractlane(value, 0);
                for lane in 1..2u8 {
                    let item = builder.ins().extractlane(value, lane);
                    result = builder.ins().iadd(result, item);
                }
                Some(result)
            }
            "vec_u8x16_splat" => {
                let byte_val = self.operand_value(inst, builder, &args[0])?;
                let byte_val = builder.ins().ireduce(cltypes::I8, byte_val);
                let byte_vec = builder.ins().splat(cltypes::I8X16, byte_val);
                let flags =
                    MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little);
                Some(builder.ins().bitcast(cltypes::I64X2, flags, byte_vec))
            }
            "vec_u8x16_eq" => {
                let left = self.operand_value(inst, builder, &args[0])?;
                let right = self.operand_value(inst, builder, &args[1])?;
                let flags =
                    MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little);
                let left_u8 = builder.ins().bitcast(cltypes::I8X16, flags, left);
                let right_u8 = builder.ins().bitcast(cltypes::I8X16, flags, right);
                let cmp = builder.ins().icmp(IntCC::Equal, left_u8, right_u8);
                Some(builder.ins().bitcast(cltypes::I64X2, flags, cmp))
            }
            // Both movemask spellings share the v1 symbol and the
            // native lowering.
            "vec_u8x16_movemask" | "vec_movemask8" => {
                let val = self.operand_value(inst, builder, &args[0])?;
                let flags =
                    MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little);
                let val_u8 = builder.ins().bitcast(cltypes::I8X16, flags, val);
                let mut mask = builder.ins().iconst(cltypes::I64, 0);
                for lane in 0..16u8 {
                    let b = builder.ins().extractlane(val_u8, lane);
                    let bit = builder.ins().ushr_imm(b, 7);
                    let bit64 = builder.ins().uextend(cltypes::I64, bit);
                    let shifted = builder.ins().ishl_imm(bit64, lane as i64);
                    mask = builder.ins().bor(mask, shifted);
                }
                Some(mask)
            }
            "vec_i64_checksum" => {
                let n = self.operand_value(inst, builder, &args[0])?;
                let func_ref = self.import_ref(builder, IMP_VEC_CHECKSUM);
                let call = builder.ins().call(func_ref, &[n]);
                Some(builder.func.dfg.first_result(call))
            }
            _ => {
                return Err(CodegenError::new(
                    Some(inst.mir_id),
                    CodegenErrorKind::UnrepresentableBuiltin {
                        builtin,
                        reason: "unknown simd builtin",
                    },
                ));
            }
        };
        Ok(value)
    }

    /// Family C: slice handles — views over list/string storage,
    /// allocated in a 40-byte explicit stack slot (the v1
    /// `ExplicitSlot` layout), validated by the v1 slice runtime.
    pub(super) fn lower_slice(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        builtin: BuiltinId,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        match name {
            "slice" | "str_slice" => {
                let (base, start, length) = (
                    self.operand_value(inst, builder, &args[0])?,
                    self.operand_value(inst, builder, &args[1])?,
                    self.operand_value(inst, builder, &args[2])?,
                );
                let slot = builder.func.create_sized_stack_slot(StackSlotData {
                    kind: StackSlotKind::ExplicitSlot,
                    size: 40,
                    align_shift: 3,
                });
                let storage = builder.ins().stack_addr(cltypes::I64, slot, 0);
                let kind = if name == "str_slice" { 0 } else { 1 };
                let kind_value = builder.ins().iconst(cltypes::I64, kind);
                let func_ref = self.import_ref(builder, "lpp_slice_init");
                let call = builder
                    .ins()
                    .call(func_ref, &[storage, base, start, length, kind_value]);
                Ok(Some(builder.func.dfg.first_result(call)))
            }
            "slice_len" | "slice_get" | "lpp_slice_get_bool" | "slice_to_str"
            | "str_slice_to_str" => {
                // `slice_get` on a `StrSlice` reads a character and returns a
                // fresh 1-char ARC string, so it dispatches to the string-slice
                // runtime entry point instead of the numeric one. The v1 ABI is
                // frozen (no dedicated builtin), so this overload is resolved
                // here from the receiver's language type.
                let symbol = if name == "slice_get"
                    && matches!(
                        args.first().and_then(|operand| {
                            operand_language_kind(inst.program, inst.types, operand)
                        }),
                        Some(TypeKind::Primitive(PrimitiveType::StrSlice))
                    ) {
                    "lpp_str_slice_get"
                } else {
                    builtin.descriptor().symbol
                };
                let mut call_args: Vec<Value> = Vec::new();
                for operand in args {
                    call_args.push(self.operand_value(inst, builder, operand)?);
                }
                let func_ref = self.import_ref(builder, symbol);
                let call = builder.ins().call(func_ref, &call_args);
                let has_result = !matches!(
                    builtin.descriptor().result,
                    lpp_runtime_abi::generated::AbiType::Void
                );
                Ok(has_result.then(|| builder.func.dfg.first_result(call)))
            }
            _ => Err(unsupported(inst.mir_id, "unknown slice builtin")),
        }
    }

    /// The oracle empties a moved value argument's source local (the
    /// reference dies with the move), so the frame-exit pass never
    /// releases it twice. Non-local and unmanaged operands stay.
    pub(super) fn empty_moved_local(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        operand: &Operand,
    ) {
        let Operand::Copy(local) = operand else {
            return;
        };
        let ty = inst
            .program
            .local(*local)
            .expect("validated MIR retains locals")
            .ty;
        if !is_managed(inst.types, ty) {
            return;
        }
        let variable = *inst.local_vars.get(local).expect("local variable");
        let zero = builder.ins().iconst(cltypes::I64, 0);
        builder.def_var(variable, zero);
    }

    /// `list_push(list, value)` / `list_set(list, index, value)`:
    /// element-class dispatch on the list's element, the value
    /// argument moving into the list (the list owns the reference,
    /// the source local is emptied).
    ///
    /// The runtime's `lpp_list_push`/`lpp_list_set` retain the stored
    /// element (`retain_element`) for ARC lists, so a move is: push
    /// (runtime creates the list's reference), then release the
    /// source's reference and empty its local. Omitting the release
    /// leaks exactly one reference per stored element — the oracle's
    /// `builtin_moves_value` move consumes the source outright.
    pub(super) fn lower_list_mutation(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        let list = self.operand_value(inst, builder, &args[0])?;
        let TypeKind::List(element) = inst.types.kind(
            inst.program
                .local(match &args[0] {
                    Operand::Copy(local) => *local,
                    _ => return Err(unsupported(inst.mir_id, "list mutation of non-local list")),
                })
                .expect("validated MIR retains locals")
                .ty,
        ) else {
            return Err(unsupported(inst.mir_id, "list mutation of non-list"));
        };
        let value_operand = if name == "list_push" {
            &args[1]
        } else {
            &args[2]
        };
        match element_class(inst.types, element) {
            ElementClass::Managed => {
                let value = self.operand_value(inst, builder, value_operand)?;
                let import = if name == "list_push" {
                    IMP_LIST_PUSH_ARC
                } else {
                    IMP_LIST_SET_ARC
                };
                let func_ref = self.import_ref(builder, import);
                if name == "list_push" {
                    builder.ins().call(func_ref, &[list, value]);
                } else {
                    let index = self.operand_value(inst, builder, &args[1])?;
                    builder.ins().call(func_ref, &[list, index, value]);
                }
                // The runtime retained its own reference; consume the
                // source's (the move), then empty the source local.
                self.arc_release(builder, value);
                self.empty_moved_local(inst, builder, value_operand);
            }
            ElementClass::Scalar => {
                let value = self.operand_value(inst, builder, value_operand)?;
                let stored = coerce_to(builder, value, cltypes::I64);
                let import = if name == "list_push" {
                    IMP_LIST_PUSH
                } else {
                    IMP_LIST_SET
                };
                let func_ref = self.import_ref(builder, import);
                if name == "list_push" {
                    builder.ins().call(func_ref, &[list, stored]);
                } else {
                    let index = self.operand_value(inst, builder, &args[1])?;
                    builder.ins().call(func_ref, &[list, index, stored]);
                }
            }
            ElementClass::Float => {
                let value = self.operand_value(inst, builder, value_operand)?;
                let stored = coerce_to(builder, value, cltypes::F64);
                let import = if name == "list_push" {
                    IMP_LIST_PUSH_FLOAT
                } else {
                    IMP_LIST_SET_FLOAT
                };
                let func_ref = self.import_ref(builder, import);
                if name == "list_push" {
                    builder.ins().call(func_ref, &[list, stored]);
                } else {
                    let index = self.operand_value(inst, builder, &args[1])?;
                    builder.ins().call(func_ref, &[list, index, stored]);
                }
            }
            ElementClass::Bool => {
                let value = self.operand_value(inst, builder, value_operand)?;
                let stored = coerce_to(builder, value, cltypes::I8);
                let import = if name == "list_push" {
                    IMP_LIST_PUSH_BOOL
                } else {
                    IMP_LIST_SET_BOOL
                };
                let func_ref = self.import_ref(builder, import);
                if name == "list_push" {
                    builder.ins().call(func_ref, &[list, stored]);
                } else {
                    let index = self.operand_value(inst, builder, &args[1])?;
                    builder.ins().call(func_ref, &[list, index, stored]);
                }
            }
        }
        Ok(None)
    }

    /// `list_get(list, index)`: element-class dispatch; an element
    /// read gains a reference (the list keeps its own).
    pub(super) fn lower_list_get(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        let list = self.operand_value(inst, builder, &args[0])?;
        let index = self.operand_value(inst, builder, &args[1])?;
        let TypeKind::List(element) = inst.types.kind(
            inst.program
                .local(match &args[0] {
                    Operand::Copy(local) => *local,
                    _ => return Err(unsupported(inst.mir_id, "list get of non-local list")),
                })
                .expect("validated MIR retains locals")
                .ty,
        ) else {
            return Err(unsupported(inst.mir_id, "list get of non-list"));
        };
        let value = match element_class(inst.types, element) {
            ElementClass::Managed => {
                let func_ref = self.import_ref(builder, IMP_LIST_GET_ARC);
                let call = builder.ins().call(func_ref, &[list, index]);
                let v = builder.func.dfg.first_result(call);
                self.arc_retain(builder, v);
                v
            }
            ElementClass::Scalar => {
                let func_ref = self.import_ref(builder, IMP_LIST_GET);
                let call = builder.ins().call(func_ref, &[list, index]);
                let v = builder.func.dfg.first_result(call);
                coerce_to(
                    builder,
                    v,
                    machine_type(inst.types, element).expect("element type"),
                )
            }
            ElementClass::Float => {
                let func_ref = self.import_ref(builder, IMP_LIST_GET_FLOAT);
                let call = builder.ins().call(func_ref, &[list, index]);
                builder.func.dfg.first_result(call)
            }
            ElementClass::Bool => {
                let func_ref = self.import_ref(builder, IMP_LIST_GET_BOOL);
                let call = builder.ins().call(func_ref, &[list, index]);
                builder.func.dfg.first_result(call)
            }
        };
        Ok(Some(value))
    }
}
