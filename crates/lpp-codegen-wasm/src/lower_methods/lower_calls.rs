use super::*;

pub(super) fn emit_i64_pow(fb: &mut FB) {
    let base = fb.scratch(Val::I64);
    let exp = fb.scratch(Val::I64);
    let result = fb.scratch(Val::I64);
    fb.s(exp);
    fb.s(base);
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
}

impl<'a> FnLower<'a> {
    pub(super) fn i64_op(&self, fb: &mut FB, op_: BinaryOperator) -> Result<(), CodegenError> {
        if op_ == BinaryOperator::Power {
            emit_i64_pow(fb);
            return Ok(());
        }
        let code = match op_ {
            BinaryOperator::Add => op::I64_ADD,
            BinaryOperator::Subtract => op::I64_SUB,
            BinaryOperator::Multiply => op::I64_MUL,
            BinaryOperator::Divide => op::I64_DIV_S,
            BinaryOperator::Modulo => op::I64_REM_S,
            BinaryOperator::BitAnd => op::I64_AND,
            BinaryOperator::BitOr => op::I64_OR,
            BinaryOperator::BitXor => op::I64_XOR,
            BinaryOperator::ShiftLeft => op::I64_SHL,
            BinaryOperator::ShiftRight => op::I64_SHR_S,
            BinaryOperator::Equal => op::I64_EQ,
            BinaryOperator::NotEqual => op::I64_NE,
            BinaryOperator::Less => op::I64_LT_S,
            BinaryOperator::Greater => op::I64_GT_S,
            BinaryOperator::LessEqual => op::I64_LE_S,
            BinaryOperator::GreaterEqual => op::I64_GE_S,
            BinaryOperator::Power => unreachable!(),
            BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr => {
                return Err(unsupported("logical op on integer", Some(self.fn_id)));
            }
        };
        fb.op(code);
        Ok(())
    }

    pub(super) fn f64_op(&self, fb: &mut FB, op_: BinaryOperator) -> Result<(), CodegenError> {
        if op_ == BinaryOperator::Power {
            self.call_helper(fb, H::Pow)?;
            return Ok(());
        }
        if op_ == BinaryOperator::Modulo {
            // fmod: l - trunc(l / r) * r
            let l = fb.scratch(Val::F64);
            let r = fb.scratch(Val::F64);
            let t = fb.scratch(Val::F64);
            // move [l, r] into scratches (r is on top)
            fb.s(r);
            fb.s(l);
            fb.g(l);
            fb.g(r);
            fb.op(op::F64_DIV);
            fb.op(op::F64_TRUNC);
            fb.g(r);
            fb.op(op::F64_MUL);
            fb.s(t);
            fb.g(l);
            fb.g(t);
            fb.op(op::F64_SUB);
            return Ok(());
        }
        let code = match op_ {
            BinaryOperator::Add => op::F64_ADD,
            BinaryOperator::Subtract => op::F64_SUB,
            BinaryOperator::Multiply => op::F64_MUL,
            BinaryOperator::Divide => op::F64_DIV,
            BinaryOperator::Equal => op::F64_EQ,
            BinaryOperator::NotEqual => op::F64_NE,
            BinaryOperator::Less => op::F64_LT,
            BinaryOperator::Greater => op::F64_GT,
            BinaryOperator::LessEqual => op::F64_LE,
            BinaryOperator::GreaterEqual => op::F64_GE,
            _ => return Err(unsupported("float operator", Some(self.fn_id))),
        };
        fb.op(code);
        Ok(())
    }

    pub(super) fn i32_op(&self, fb: &mut FB, op_: BinaryOperator) -> Result<(), CodegenError> {
        let code = match op_ {
            BinaryOperator::Equal => op::I32_EQ,
            BinaryOperator::NotEqual => op::I32_NE,
            BinaryOperator::Less => op::I32_LT_S,
            BinaryOperator::Greater => op::I32_GT_S,
            BinaryOperator::LessEqual => op::I32_LE_S,
            BinaryOperator::GreaterEqual => op::I32_GE_S,
            BinaryOperator::LogicalAnd => op::I32_AND,
            BinaryOperator::LogicalOr => op::I32_OR,
            _ => return Err(unsupported("boolean/char operator", Some(self.fn_id))),
        };
        fb.op(code);
        Ok(())
    }

    pub(super) fn emit_call(
        &self,
        fb: &mut FB,
        callee: Operand,
        arguments: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        match callee {
            Operand::Function(id) => {
                let callee_fn = self.env.program.function(id).unwrap();
                if matches!(callee_fn.kind, lpp_mir::MirFunctionKind::Async) {
                    // A direct async call constructs a task through the
                    // function's thunk (5D2b slice 3).
                    self.emit_task_construct(fb, None, arguments, callee_fn.return_type, Some(id))
                } else {
                    self.emit_direct_call(fb, id, arguments)
                }
            }
            Operand::Copy(local) => {
                let callee_ty = self.env.program.local(local).unwrap().ty;
                let result = match self.env.types.kind(callee_ty) {
                    TypeKind::Function { result, .. } => result,
                    _ => {
                        return Err(unsupported(
                            "call through a non-function value",
                            Some(self.fn_id),
                        ));
                    }
                };
                if matches!(self.env.types.kind(result), TypeKind::Task(_)) {
                    // A call through an async function value constructs
                    // a task; the capsule's code word is the task
                    // thunk. The result is a Task handle (a managed
                    // pointer) — no unboxing.
                    self.emit_task_construct(fb, Some(local), arguments, result, None)
                } else {
                    self.emit_closure_call(fb, local, arguments)
                }
            }
            _ => Err(unsupported("call through value", Some(self.fn_id))),
        }
    }

    /// Build the task env tuple for `arguments` and the task node
    /// (5D2b slice 3): `tuple(16+8n)` with the argument slots at
    /// `16+8i` (managed arguments retained for the tuple), then
    /// `TaskNew(code, env, managed_flag)`. `capsule` names the local
    /// whose capsule `code` word is the task thunk (a call through an
    /// async function value); otherwise the thunk for `fn_id` is used.
    pub(super) fn emit_task_construct(
        &self,
        fb: &mut FB,
        capsule: Option<MirLocalId>,
        arguments: &lpp_mir::ListRange<Operand>,
        result_ty: TypeId,
        fn_id: Option<MirFunctionId>,
    ) -> Result<Class, CodegenError> {
        let args = self.env.program.operands(*arguments).to_vec();
        let tuple_slot = self
            .env
            .tuple_drop_slot
            .ok_or_else(|| unsupported("missing tuple destroyer", Some(self.fn_id)))?;
        let mut mask = 0i64;
        let mut packed = 0i64;
        for (i, operand) in args.iter().enumerate() {
            if let Operand::Copy(local) = operand {
                let ty = self.env.program.local(*local).unwrap().ty;
                if is_managed(self.env.types, ty) {
                    mask |= 1i64 << i;
                }
            }
            packed |= (16 + 8 * i as i64) << (16 * i as i64);
        }
        // The tuple: the ARC node the arguments' references land in.
        fb.i32c(16 + 8 * args.len() as i64);
        fb.i32c(tuple_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        crate::cycle::maybe_mark_task_collectible(self.env, fb);
        let tuple = fb.scratch(Val::I32);
        fb.t(tuple);
        fb.g(tuple).i64c(mask).store64(0);
        fb.g(tuple).i64c(packed).store64(8);
        for (i, operand) in args.iter().enumerate() {
            let offset = 16 + 8 * i as u32;
            let Operand::Copy(local) = operand else {
                return Err(unsupported(
                    "async call with non-local argument",
                    Some(self.fn_id),
                ));
            };
            let ty = self.env.program.local(*local).unwrap().ty;
            if is_managed(self.env.types, ty) {
                // Retain for the tuple (it owns the reference); the
                // tee addresses the store and the value is dropped.
                self.emit_owned(fb, operand)?;
                let slot = fb.scratch(Val::I32);
                fb.t(slot);
                fb.g(tuple).g(slot).op(op::I64_EXTEND_I32_S).store64(offset);
                fb.g(slot).op(op::DROP);
            } else {
                match class_of_type(self.env.types, ty)? {
                    Class::I64 => {
                        self.emit_operand(fb, operand)?;
                        fb.g(tuple).store64(offset);
                    }
                    Class::F64 => {
                        self.emit_operand(fb, operand)?;
                        fb.op(op::I64_REINTERPRET_F64);
                        fb.g(tuple).store64(offset);
                    }
                    Class::Bool | Class::Char => {
                        self.emit_operand(fb, operand)?;
                        fb.op(op::I64_EXTEND_I32_U);
                        fb.g(tuple).store64(offset);
                    }
                    Class::Str => {
                        self.emit_operand(fb, operand)?;
                        fb.op(op::I64_EXTEND_I32_S);
                        fb.g(tuple).store64(offset);
                    }
                    Class::Ptr => unreachable!("checked above"),
                }
            }
        }
        // TaskNew(code, env, managed): the stack takes (code, env,
        // managed) — code first. The tee'd tuple pointer is no longer
        // needed (every use above re-reads the scratch local); drop it.
        fb.op(op::DROP);
        match (capsule, fn_id) {
            (None, Some(fn_id)) => {
                let code = *self
                    .env
                    .task_thunk_dispatch
                    .get(&fn_id)
                    .ok_or_else(|| unsupported("undispatched task thunk", Some(self.fn_id)))?;
                fb.i32c(code as i64);
            }
            (Some(capsule), None) => {
                let index = *self
                    .local_index
                    .get(&capsule)
                    .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
                fb.g(index).load64(0).op(op::I32_WRAP_I64);
            }
            _ => unreachable!("exactly one code source"),
        }
        fb.g(tuple);
        let managed = if is_managed(self.env.types, result_ty) {
            1
        } else {
            0
        };
        fb.i32c(managed as i64);
        self.call_helper(fb, H::TaskNew)?;
        // TaskNew returns the node pointer as an i64 word; narrow
        // for the wasm ABI.
        fb.op(op::I32_WRAP_I64);
        crate::cycle::maybe_mark_task_collectible(self.env, fb);
        Ok(Class::Ptr)
    }

    pub(super) fn emit_direct_call(
        &self,
        fb: &mut FB,
        fn_id: MirFunctionId,
        arguments: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let callee_fn = self.env.program.function(fn_id).ok_or_else(|| {
            CodegenError::new(
                Some(self.fn_id),
                CodegenErrorKind::ObjectEmissionFailed("unknown callee".to_string()),
            )
        })?;
        // The dispatch ABI: every function takes the env pointer first
        // (NULL for a direct call — plain functions ignore it).
        fb.i32c(0);
        for arg in self.env.program.operands(*arguments) {
            // A managed argument retains for the callee (released by the
            // callee's exit release pass); the source keeps its own.
            self.emit_owned(fb, arg)?;
        }
        let index = *self
            .env
            .user_index
            .get(&fn_id)
            .ok_or_else(|| unsupported("unknown function", Some(self.fn_id)))?;
        fb.call(index);
        let rt = callee_fn.return_type;
        if is_void(self.env.types, rt) {
            fb.i64c(0);
            Ok(Class::I64)
        } else {
            class_of_type(self.env.types, rt)
        }
    }

    /// A call through a capsule: `Call(Copy(local), args)` where the
    /// local holds a closure capsule. Stack: the capsule, the user
    /// arguments (captures come from the env), then the dispatch index
    /// loaded from the capsule's `code` word; `call_indirect` on the
    /// dispatch table (table 1) pops them in reverse.
    pub(super) fn emit_closure_call(
        &self,
        fb: &mut FB,
        capsule: MirLocalId,
        arguments: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let capsule_ty = self.env.program.local(capsule).unwrap().ty;
        let (parameters, result) = match self.env.types.kind(capsule_ty) {
            TypeKind::Function { parameters, result } => (parameters, result),
            _ => {
                return Err(unsupported(
                    "call through a non-function value",
                    Some(self.fn_id),
                ));
            }
        };
        let index = *self
            .local_index
            .get(&capsule)
            .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
        // Stack bottom->top: env, user args..., dispatch index.
        // `call_indirect` pops the index, then the parameters in
        // reverse; the env (first parameter) is the capsule's env word.
        fb.g(index).load64(8).op(op::I32_WRAP_I64);
        // The argument operands are the user parameters of the closure
        // type; the count is checked against the type's parameter list.
        let args = self.env.program.operands(*arguments);
        if args.len() != self.env.types.list(parameters).len() {
            return Err(unsupported(
                "closure call argument count mismatch",
                Some(self.fn_id),
            ));
        }
        for arg in args {
            self.emit_owned(fb, arg)?;
        }
        let call_type = *self
            .env
            .closure_call_type
            .get(&capsule_ty)
            .ok_or_else(|| unsupported("unregistered closure call type", Some(self.fn_id)))?;
        fb.g(index) // dispatch index = the capsule's code word, on top
            .load64(0)
            .op(op::I32_WRAP_I64);
        fb.call_indirect(call_type, self.env.dispatch_table);
        if is_void(self.env.types, result) {
            fb.i64c(0);
            Ok(Class::I64)
        } else {
            class_of_type(self.env.types, result)
        }
    }

    /// Builds a closure capsule: an optional env node (one fixed
    /// 8-byte slot per capture) plus the 16-byte capsule node whose
    /// `code` word is the dispatch-table index and whose `env` word is
    /// the env pointer (0 for a zero-capture closure).
    pub(super) fn emit_make_closure(
        &self,
        fb: &mut FB,
        function: MirFunctionId,
        captures: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let closure = self.env.program.function(function).ok_or_else(|| {
            CodegenError::new(
                Some(self.fn_id),
                CodegenErrorKind::ObjectEmissionFailed("unknown closure".to_string()),
            )
        })?;
        // The env slots are the closure frame's capture slots
        // (`Capture`-kind parameters); `captures` operand `i` feeds
        // slot `i`.
        let capture_locals: Vec<MirLocalId> = self
            .env
            .program
            .function_parameters(closure)
            .iter()
            .copied()
            .filter(|&p| {
                matches!(
                    self.env.program.local(p).unwrap().kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .collect();
        let count = capture_locals.len() as u32;

        // The env node owns one reference per managed capture.
        let env_node: u32 = if count == 0 {
            0
        } else {
            let env_dtor =
                *self.env.env_drop_slot.get(&function).ok_or_else(|| {
                    unsupported("unregistered closure env slot", Some(self.fn_id))
                })?;
            let ptr = fb.scratch(Val::I32);
            fb.i32c((8 * count) as i64);
            fb.i32c(env_dtor as i64);
            self.call_helper(fb, H::ArcAlloc)?;
            crate::cycle::maybe_mark_closure_collectible(self.env, fb);
            fb.s(ptr);
            let values = self.env.program.operands(*captures);
            if values.len() != capture_locals.len() {
                return Err(unsupported("closure capture count", Some(self.fn_id)));
            }
            for (i, operand) in values.iter().enumerate() {
                fb.g(ptr);
                let _class = self.emit_owned(fb, operand)?;
                let ty = self.env.program.local(capture_locals[i]).unwrap().ty;
                store_by_type(fb, self.env.types, 8 * i as u32, ty);
            }
            ptr
        };

        // The capsule node.
        let capsule = fb.scratch(Val::I32);
        let destroy_slot = self
            .env
            .closure_destroy_slot
            .ok_or_else(|| unsupported("missing closure destroyer", Some(self.fn_id)))?;
        fb.i32c(16);
        fb.i32c(destroy_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        crate::cycle::maybe_mark_closure_collectible(self.env, fb);
        fb.s(capsule);
        let code = *self
            .env
            .dispatch
            .get(&function)
            .ok_or_else(|| unsupported("undispatched closure", Some(self.fn_id)))?;
        fb.g(capsule).i64c(code as i64).store64(0);
        // The env word is the env node's pointer (a local value, not an
        // index); zero-capture closures have a NULL env.
        if count == 0 {
            fb.g(capsule).i64c(0).store64(8);
        } else {
            fb.g(capsule)
                .g(env_node)
                .op(op::I64_EXTEND_I32_S)
                .store64(8);
        }
        fb.g(capsule);
        Ok(Class::Ptr)
    }

    /// The second argument of a binary builtin (5D2b slice 4).
    pub(super) fn arg1<'b>(&self, args: &'b [Operand]) -> Result<&'b Operand, CodegenError> {
        args.get(1)
            .ok_or_else(|| unsupported("builtin argument missing", Some(self.fn_id)))
    }
}
