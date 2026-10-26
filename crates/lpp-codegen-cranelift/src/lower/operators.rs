use super::*;

impl<'m> Lowering<'m> {
    /// Integer `sdiv`/`srem` with L++ semantics: a zero divisor is
    /// the defined runtime panic (the oracle's `DivisionByZero`,
    /// exit 101 via `lpp_panic_div_zero`), never a hardware trap;
    /// `INT_MIN / -1` wraps like the oracle's
    /// `wrapping_div`/`wrapping_rem` — substituting divisor 1 for
    /// that single case yields `INT_MIN` and `0` respectively, so
    /// the hardware op never sees the overflowing pair either.
    pub(super) fn int_divmod(
        &mut self,
        builder: &mut FunctionBuilder,
        ty: CLType,
        divide: bool,
        l: Value,
        r: Value,
    ) -> Value {
        let zero = builder.ins().iconst(ty, 0);
        let divisor_ok = builder.ins().icmp(IntCC::NotEqual, r, zero);
        let panic_block = builder.create_block();
        let div_block = builder.create_block();
        let merge_block = builder.create_block();
        let result = builder.append_block_param(merge_block, ty);
        builder
            .ins()
            .brif(divisor_ok, div_block, &[], panic_block, &[]);

        builder.seal_block(panic_block);
        builder.switch_to_block(panic_block);
        let func_ref = self.import_ref(builder, IMP_PANIC_DIV_ZERO);
        builder.ins().call(func_ref, &[]);
        builder.ins().trap(TrapCode::unwrap_user(1));

        builder.seal_block(div_block);
        builder.switch_to_block(div_block);
        let int_min = builder.ins().iconst(
            ty,
            match ty {
                cltypes::I8 => i64::from(i8::MIN),
                cltypes::I16 => i64::from(i16::MIN),
                cltypes::I32 => i64::from(i32::MIN),
                _ => i64::MIN,
            },
        );
        let neg_one = builder.ins().iconst(ty, -1);
        let is_min = builder.ins().icmp(IntCC::Equal, l, int_min);
        let is_neg_one = builder.ins().icmp(IntCC::Equal, r, neg_one);
        let one = builder.ins().iconst(ty, 1);
        let if_min = builder.ins().select(is_neg_one, one, r);
        let safe_r = builder.ins().select(is_min, if_min, r);
        let quotient = if divide {
            builder.ins().sdiv(l, safe_r)
        } else {
            builder.ins().srem(l, safe_r)
        };
        builder.ins().jump(merge_block, &[quotient]);

        builder.seal_block(merge_block);
        builder.switch_to_block(merge_block);
        result
    }

    pub(super) fn lower_binary(
        &mut self,
        builder: &mut FunctionBuilder,
        operator: BinaryOperator,
        ty: CLType,
        string_eq: bool,
        l: Value,
        r: Value,
    ) -> Result<Value, CodegenError> {
        let mut compare = |cc_int: IntCC, cc_float: FloatCC| -> Value {
            let bool_val = match ty {
                cltypes::F64 => builder.ins().fcmp(cc_float, l, r),
                _ => builder.ins().icmp(cc_int, l, r),
            };
            let one = builder.ins().iconst(cltypes::I8, 1);
            let zero = builder.ins().iconst(cltypes::I8, 0);
            builder.ins().select(bool_val, one, zero)
        };
        Ok(match operator {
            BinaryOperator::Add => match ty {
                cltypes::F64 => builder.ins().fadd(l, r),
                _ => builder.ins().iadd(l, r),
            },
            BinaryOperator::Subtract => match ty {
                cltypes::F64 => builder.ins().fsub(l, r),
                _ => builder.ins().isub(l, r),
            },
            BinaryOperator::Multiply => match ty {
                cltypes::F64 => builder.ins().fmul(l, r),
                _ => builder.ins().imul(l, r),
            },
            BinaryOperator::Divide => match ty {
                cltypes::F64 => builder.ins().fdiv(l, r),
                _ => self.int_divmod(builder, ty, true, l, r),
            },
            BinaryOperator::Modulo if ty == cltypes::F64 => {
                let func_ref = self.import_ref(builder, IMP_FMOD);
                let call_inst = builder.ins().call(func_ref, &[l, r]);
                builder.func.dfg.first_result(call_inst)
            }
            BinaryOperator::Modulo => self.int_divmod(builder, ty, false, l, r),
            BinaryOperator::BitAnd => builder.ins().band(l, r),
            BinaryOperator::BitOr => builder.ins().bor(l, r),
            BinaryOperator::BitXor => builder.ins().bxor(l, r),
            BinaryOperator::ShiftLeft => builder.ins().ishl(l, r),
            BinaryOperator::ShiftRight => builder.ins().sshr(l, r),
            BinaryOperator::Equal if string_eq => {
                let func_ref = self.import_ref(builder, IMP_STR_EQ);
                let call_inst = builder.ins().call(func_ref, &[l, r]);
                let raw = builder.func.dfg.first_result(call_inst);
                builder.ins().ireduce(cltypes::I8, raw)
            }
            BinaryOperator::Equal => compare(IntCC::Equal, FloatCC::Equal),
            BinaryOperator::NotEqual if string_eq => {
                // `lpp_str_eq` returns 1 when the contents match, so
                // inequality is `eq_result == 0`.
                let func_ref = self.import_ref(builder, IMP_STR_EQ);
                let call_inst = builder.ins().call(func_ref, &[l, r]);
                let raw = builder.func.dfg.first_result(call_inst);
                let zero = builder.ins().iconst(cltypes::I64, 0);
                // `icmp` already yields the I8 condition value.
                builder.ins().icmp(IntCC::Equal, raw, zero)
            }
            BinaryOperator::NotEqual => compare(IntCC::NotEqual, FloatCC::NotEqual),
            BinaryOperator::Less => compare(IntCC::SignedLessThan, FloatCC::LessThan),
            BinaryOperator::Greater => compare(IntCC::SignedGreaterThan, FloatCC::GreaterThan),
            BinaryOperator::LessEqual => {
                compare(IntCC::SignedLessThanOrEqual, FloatCC::LessThanOrEqual)
            }
            BinaryOperator::GreaterEqual => {
                compare(IntCC::SignedGreaterThanOrEqual, FloatCC::GreaterThanOrEqual)
            }
            BinaryOperator::LogicalAnd => builder.ins().band(l, r),
            BinaryOperator::LogicalOr => builder.ins().bor(l, r),
        })
    }

    pub(super) fn lower_terminator(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        terminator: &Terminator,
        cl_blocks: &HashMap<BasicBlockId, Block>,
    ) -> Result<(), CodegenError> {
        match terminator {
            Terminator::Goto(target) => {
                let block = *cl_blocks
                    .get(target)
                    .expect("validated CFG retains jump targets");
                builder.ins().jump(block, &[]);
                Ok(())
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let cond = self.operand_value(inst, builder, condition)?;
                let cond_bool = builder.ins().icmp_imm(IntCC::NotEqual, cond, 0);
                let then_block = *cl_blocks
                    .get(then_block)
                    .expect("validated CFG retains branch targets");
                let else_block = *cl_blocks
                    .get(else_block)
                    .expect("validated CFG retains branch targets");
                builder
                    .ins()
                    .brif(cond_bool, then_block, &[], else_block, &[]);
                Ok(())
            }
            Terminator::SwitchEnum {
                subject,
                aggregate,
                targets,
            } => {
                let _ = aggregate;
                // The subject is a plain read (no retain — the
                // interpreter's rule); the tag is the I64 at offset 0.
                let subject_value = self.operand_value(inst, builder, subject)?;
                let tag = builder
                    .ins()
                    .load(cltypes::I64, MemFlags::new(), subject_value, 0);
                let target_blocks: Vec<Block> = inst
                    .program
                    .switch_targets(*targets)
                    .iter()
                    .map(|&target| {
                        *cl_blocks
                            .get(&target)
                            .expect("validated CFG retains switch targets")
                    })
                    .collect();
                emit_tag_cascade(builder, tag, &target_blocks);
                Ok(())
            }
            Terminator::Return(operand) => {
                let return_value = if let Some(operand) = operand {
                    let value = self.operand_value(inst, builder, operand)?;
                    // A managed return retains for the caller: the
                    // exit pass below releases the slot's own
                    // reference, not the caller's.
                    if is_managed(inst.types, inst.function.return_type) {
                        self.arc_retain(builder, value);
                    }
                    Some(value)
                } else {
                    None
                };
                // The exit release pass: every managed slot that still
                // owns a reference releases it, in declaration order.
                self.exit_release_pass(inst, builder);
                let values: Vec<Value> = return_value.into_iter().collect();
                builder.ins().return_(&values);
                Ok(())
            }
            Terminator::Unreachable => {
                builder.ins().trap(TrapCode::unwrap_user(1));
                Ok(())
            }
        }
    }

    // ── operand values ────────────────────────────────────────────────────

    /// An operand in a value position: variable loads, constants, and
    /// deduplicated string data.
    pub(super) fn operand_value(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        operand: &Operand,
    ) -> Result<Value, CodegenError> {
        match operand {
            Operand::Copy(local) => {
                let variable = *inst
                    .local_vars
                    .get(local)
                    .expect("validated MIR retains local variables");
                Ok(builder.use_var(variable))
            }
            Operand::Constant(constant) => Ok(match constant {
                Constant::Integer(value) => builder.ins().iconst(cltypes::I64, *value),
                Constant::FloatBits(bits) => builder.ins().f64const(f64::from_bits(*bits)),
                Constant::Bool(value) => builder
                    .ins()
                    .iconst(cltypes::I8, i64::from(u8::from(*value))),
                Constant::Character {
                    origin: _,
                    character,
                } => builder.ins().iconst(cltypes::I32, *character as i64),
                Constant::String { origin: _, string } => {
                    self.string_constant(inst, builder, *string)?
                }
            }),
            Operand::Function(_) => Err(unsupported(inst.mir_id, "inst.function value")),
        }
    }

    /// String constants use the v1 runtime layout, deduplicated per
    /// `MirStringId` (the interpreter interns literals per inst.program, so
    /// pointer identity of equal literals matches between the object
    /// and the oracle).
    pub(super) fn string_constant(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        string: MirStringId,
    ) -> Result<Value, CodegenError> {
        if let Some(&data_id) = self.emitted_strings.get(&string) {
            let local_id = self.module.declare_data_in_func(data_id, builder.func);
            let base = builder.ins().symbol_value(cltypes::I64, local_id);
            return Ok(builder.ins().iadd_imm(base, STRING_HEADER_OFFSET));
        }

        let text = inst
            .program
            .string(string)
            .cloned()
            .expect("validated MIR retains every string");
        let symbol_name = format!("lpp_str_{}", string.raw());

        let data_id = self
            .module
            .declare_data(&symbol_name, Linkage::Export, false, false)
            .map_err(|e| {
                emission_failed(format!(
                    "declare string data {symbol_name} in {:?}: {e:?}",
                    inst.mir_id
                ))
            })?;

        let mut bytes = Vec::with_capacity(STRING_HEADER_OFFSET as usize + text.len() + 1);
        bytes.extend_from_slice(&LPP_ARC_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&LPP_ARC_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(0);

        let mut data_ctx = DataDescription::new();
        data_ctx.define(bytes.into_boxed_slice());
        data_ctx.set_align(16);
        self.module.define_data(data_id, &data_ctx).map_err(|e| {
            emission_failed(format!(
                "define string data {symbol_name} in {:?}: {e:?}",
                inst.mir_id
            ))
        })?;
        self.emitted_strings.insert(string, data_id);

        let local_id = self.module.declare_data_in_func(data_id, builder.func);
        let base = builder.ins().symbol_value(cltypes::I64, local_id);
        Ok(builder.ins().iadd_imm(base, STRING_HEADER_OFFSET))
    }
}
