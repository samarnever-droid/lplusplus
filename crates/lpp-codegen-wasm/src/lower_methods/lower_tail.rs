use super::*;

impl<'a> FnLower<'a> {
    /// The element type of a list operand (a `Copy` of a list local).
    pub(super) fn list_element_of(&self, operand: &Operand) -> Result<TypeId, CodegenError> {
        let Operand::Copy(local) = operand else {
            return Err(unsupported(
                "list argument must be a local",
                Some(self.fn_id),
            ));
        };
        let ty = self.env.program.local(*local).unwrap().ty;
        match self.env.types.kind(ty) {
            TypeKind::List(element) => Ok(element),
            _ => Err(unsupported("list argument expected", Some(self.fn_id))),
        }
    }

    /// `list_new` with the target's element class (the is-ARC word).
    pub(super) fn emit_list_new(
        &self,
        fb: &mut FB,
        target_ty: TypeId,
    ) -> Result<Class, CodegenError> {
        let TypeKind::List(element) = self.env.types.kind(target_ty) else {
            return Err(unsupported(
                "list_new outside a list target",
                Some(self.fn_id),
            ));
        };
        fb.i32c(
            if element_class(self.env.types, element) == ElementClass::Ptr {
                1
            } else {
                0
            },
        );
        self.call_helper(fb, H::ListNew)?;
        Ok(Class::Ptr)
    }

    pub(super) fn emit_map_new(
        &self,
        fb: &mut FB,
        target_ty: TypeId,
    ) -> Result<Class, CodegenError> {
        if !matches!(
            self.env.types.kind(target_ty),
            TypeKind::Primitive(PrimitiveType::Int)
        ) {
            return Err(unsupported(
                "map_new outside its opaque Int handle target",
                Some(self.fn_id),
            ));
        }
        fb.i32c(0);
        self.call_helper(fb, H::MapNew)?;
        fb.op(op::I64_EXTEND_I32_U);
        Ok(Class::I64)
    }

    pub(super) fn emit_terminator(
        &self,
        fb: &mut FB,
        terminator: &Terminator,
        positions: &HashMap<BasicBlockId, u32>,
    ) -> Result<(), CodegenError> {
        match terminator {
            Terminator::Goto(target) => {
                fb.i32c(positions[target] as i64);
                fb.s(self.cur_local);
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                match self.emit_operand(fb, condition)? {
                    Class::Bool => {}
                    Class::I64 => {
                        // L++ permits integer truthiness. WebAssembly `if`
                        // needs an i32 condition, so lower `x` to `x != 0`.
                        fb.op(op::I64_EQZ);
                        fb.op(op::I32_EQZ);
                    }
                    _ => {
                        return Err(unsupported("branch condition type", Some(self.fn_id)));
                    }
                }
                fb.if_();
                fb.i32c(positions[then_block] as i64);
                fb.s(self.cur_local);
                fb.else_();
                fb.i32c(positions[else_block] as i64);
                fb.s(self.cur_local);
                fb.end();
            }
            Terminator::Return(operand) => {
                let function = self.env.program.function(self.fn_id).unwrap();
                if let Some(op) = operand {
                    // A plain read: the return retains for the caller
                    // below, and the exit pass releases the slot's own
                    // reference — not the caller's.
                    self.emit_operand(fb, op)?;
                    if is_managed(self.env.types, function.return_type) {
                        self.call_helper(fb, H::Retain)?;
                    }
                }
                self.exit_release_pass(fb)?;
                fb.op(op::RETURN);
            }
            Terminator::Unreachable => {
                // Mirrors the 5C lowering: a trap ends the process; the
                // unreleased references are reclaimed with the memory.
                fb.op(op::UNREACHABLE);
            }
            Terminator::SwitchEnum {
                subject, targets, ..
            } => {
                // The subject is a plain read (no retain — the
                // interpreter's rule); the tag is the i32 at offset 0.
                // The load consumes the subject pointer, so no tee.
                self.emit_operand(fb, subject)?;
                let tag = fb.scratch(Val::I32);
                fb.load32(ENUM_TAG as u32);
                fb.s(tag);
                let targets: &[BasicBlockId] = self.env.program.switch_targets(*targets);
                // `if0 {…} else { if1 {…} else { if2 {…} } }`: n arms open
                // n `if`s (n−1 `else`s), so n `end`s close the chain.
                for (i, &target) in targets.iter().enumerate() {
                    fb.g(tag);
                    fb.i32c(i as i64);
                    fb.op(op::I32_EQ);
                    fb.if_();
                    fb.i32c(positions[&target] as i64);
                    fb.s(self.cur_local);
                    if i + 1 < targets.len() {
                        fb.else_();
                    }
                }
                for _ in 0..targets.len() {
                    fb.end();
                }
            }
        }
        Ok(())
    }

    /// At every `Return`/trap: every managed local that still owns a
    /// reference releases it, in declaration order. Transfers never
    /// null their source, so the pass is unconditional — each retain
    /// balances against exactly one release (overwrite or exit).
    pub(super) fn exit_release_pass(&self, fb: &mut FB) -> Result<(), CodegenError> {
        let function = self.env.program.function(self.fn_id).unwrap();
        for &local in self.env.program.function_locals(function) {
            let descriptor = self.env.program.local(local).unwrap();
            // 5D2b: the closure frame's capture slots are views of the
            // env (the env slot owns the reference) — releasing them
            // here would double-free at env destruction.
            if matches!(descriptor.kind, lpp_mir::MirLocalKind::Capture) {
                continue;
            }
            let ty = descriptor.ty;
            if is_managed(self.env.types, ty) {
                let index = *self
                    .local_index
                    .get(&local)
                    .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
                fb.g(index);
                self.call_helper(fb, H::Release)?;
            }
        }
        Ok(())
    }

    pub(super) fn call_helper(&self, fb: &mut FB, h: H) -> Result<(), CodegenError> {
        let index = *self.env.helper_index.get(&h).ok_or_else(|| {
            CodegenError::new(
                Some(self.fn_id),
                CodegenErrorKind::ObjectEmissionFailed(format!("helper not registered: {h:?}")),
            )
        })?;
        fb.call(index);
        Ok(())
    }
}
