use super::*;

impl<'a> FnLower<'a> {
    pub(super) fn emit_operand(
        &self,
        fb: &mut FB,
        operand: &Operand,
    ) -> Result<Class, CodegenError> {
        let class = operand_class(self.env.program, self.env.types, operand)?;
        match operand {
            Operand::Copy(local) => {
                let index = *self
                    .local_index
                    .get(local)
                    .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
                fb.g(index);
            }
            Operand::Constant(c) => {
                match c {
                    Constant::Integer(v) => fb.i64c(*v),
                    Constant::FloatBits(bits) => fb.f64c(f64::from_bits(*bits)),
                    Constant::Bool(b) => fb.i32c(if *b { 1 } else { 0 }),
                    Constant::Character { character, .. } => fb.i32c(*character as i64),
                    Constant::String { string, .. } => {
                        let content = self
                            .env
                            .program
                            .string(*string)
                            .ok_or_else(|| {
                                CodegenError::new(
                                    Some(self.fn_id),
                                    CodegenErrorKind::ObjectEmissionFailed(
                                        "unknown string constant".to_string(),
                                    ),
                                )
                            })?
                            .clone();
                        let ptr = *self.env.string_ptr.get(&content).ok_or_else(|| {
                            CodegenError::new(
                                Some(self.fn_id),
                                CodegenErrorKind::ObjectEmissionFailed(
                                    "string not pooled".to_string(),
                                ),
                            )
                        })?;
                        fb.i32c(ptr as i64)
                    }
                };
            }
            Operand::Function(function_id) => {
                // A bare function value is a 16-byte capsule
                // `[code, env]` on the ARC heap (5D2b slice 2); the
                // capsule starts with rc = 1 — ownership transfers to
                // the use site (no retain). A sync value points at the
                // function with a NULL env; an async value (slice 3)
                // points at its task thunk with an empty task-env
                // tuple (spawnable values need a non-NULL env).
                let callee = self.env.program.function(*function_id).unwrap();
                let is_async = matches!(callee.kind, lpp_mir::MirFunctionKind::Async);
                let dispatch_index = if is_async {
                    *self
                        .env
                        .task_thunk_dispatch
                        .get(function_id)
                        .ok_or_else(|| unsupported("undispatched task thunk", Some(self.fn_id)))?
                } else {
                    *self.env.dispatch.get(function_id).ok_or_else(|| {
                        unsupported("undispatched function value", Some(self.fn_id))
                    })?
                };
                let destroy_slot = self
                    .env
                    .closure_destroy_slot
                    .ok_or_else(|| unsupported("missing closure destroyer", Some(self.fn_id)))?;
                let capsule = fb.scratch(Val::I32);
                fb.i32c(16);
                fb.i32c(destroy_slot as i64);
                self.call_helper(fb, H::ArcAlloc)?;
                // Keep the capsule on the stack (the caller owns it);
                // the scratch only addresses the fill-in stores.
                fb.t(capsule);
                fb.g(capsule).i64c(dispatch_index as i64).store64(0);
                if is_async {
                    let tuple_slot = self
                        .env
                        .tuple_drop_slot
                        .ok_or_else(|| unsupported("missing tuple destroyer", Some(self.fn_id)))?;
                    fb.i32c(16);
                    fb.i32c(tuple_slot as i64);
                    self.call_helper(fb, H::ArcAlloc)?;
                    let tuple = fb.scratch(Val::I32);
                    fb.t(tuple);
                    fb.g(tuple).i64c(0).store64(0); // mask
                    fb.g(tuple).i64c(0).store64(8); // packed offsets
                    // The empty tuple is a fresh node (rc = 1); its
                    // reference transfers to the capsule. The tee's
                    // stack copy is consumed by the store.
                    fb.g(capsule).op(op::I64_EXTEND_I32_S).store64(8);
                } else {
                    fb.g(capsule).i64c(0).store64(8);
                }
            }
        }
        Ok(class)
    }

    /// An operand that creates a new owner: a plain read, plus a retain
    /// when it is a `Copy` of a managed local (retain-on-transfer).
    /// `Retain` consumes the pointer, so the local is re-pushed after —
    /// the caller keeps its value and gains one reference count.
    pub(super) fn emit_owned(&self, fb: &mut FB, operand: &Operand) -> Result<Class, CodegenError> {
        let class = self.emit_operand(fb, operand)?;
        if let Operand::Copy(local) = operand {
            let ty = self.env.program.local(*local).unwrap().ty;
            if is_managed(self.env.types, ty) {
                // Retain passes the value through; the caller keeps the
                // copy on the stack. Strings are managed too even though
                // their lowering class is `Str` rather than the generic
                // pointer class.
                self.call_helper(fb, H::Retain)?;
            }
        }
        Ok(class)
    }

    /// `target = rvalue`: produce the value (transfers retain inside),
    /// release the slot's old reference (if any), then store the new
    /// value — the interpreter's order (a self-assignment retains before
    /// it releases).
    pub(super) fn emit_assign(
        &self,
        fb: &mut FB,
        target: MirLocalId,
        rvalue: &Rvalue,
    ) -> Result<(), CodegenError> {
        let target_ty = self.env.program.local(target).unwrap().ty;
        let _class = if let Rvalue::List(items) = rvalue {
            self.emit_list_literal(fb, target_ty, items)?
        } else if let Rvalue::Tuple(items) = rvalue {
            self.emit_tuple_literal(fb, target_ty, items)?
        } else if let Rvalue::Use(Operand::Constant(Constant::Integer(value))) = rvalue
            && matches!(self.env.types.kind(target_ty), TypeKind::Nominal { .. })
        {
            // Legacy aggregate null is spelled integer `0` in source. MIR
            // materializes it as a nominally typed local, while wasm pointers
            // are i32, so emit the context-typed zero directly instead of the
            // canonical i64 integer constant.
            fb.i32c(*value);
            Class::Ptr
        } else if let Rvalue::Use(operand) = rvalue {
            // `Use(Copy(managed))` transfers a new reference to the
            // target: retain on copy (a plain `emit_operand` would leave
            // the target a dangling alias, double-freed at exit).
            self.emit_owned(fb, operand)?
        } else if let Rvalue::Builtin { builtin, .. } = rvalue
            && builtin
                .descriptor()
                .name
                .strip_prefix("lpp_")
                .unwrap_or(builtin.descriptor().name)
                == "list_new"
        {
            // The element class (hence the is-ARC word) comes from the
            // target's list type.
            self.emit_list_new(fb, target_ty)?
        } else if let Rvalue::Builtin { builtin, .. } = rvalue
            && matches!(
                builtin
                    .descriptor()
                    .name
                    .strip_prefix("lpp_")
                    .unwrap_or(builtin.descriptor().name),
                "map_new" | "map_new_arc"
            )
        {
            self.emit_map_new(fb, target_ty)?
        } else {
            self.emit_rvalue(fb, rvalue)?
        };
        let index = *self
            .local_index
            .get(&target)
            .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
        match self.captures.get(&target) {
            // 5D2b: a capture local is a view of its env slot (local 0
            // holds the env). The env slot owns the managed reference,
            // so the old value is released through the slot — never
            // through the local — then the new value lands in both.
            Some(&slot) => {
                if is_managed(self.env.types, target_ty) {
                    fb.g(0);
                    match local_val(self.env.types, target_ty)? {
                        Val::I32 => fb.load32(8 * slot),
                        Val::I64 => fb.load64(8 * slot),
                        Val::F64 => fb.loadf64(8 * slot),
                    };
                    self.call_helper(fb, H::Release)?;
                }
                fb.s(index);
                fb.g(0);
                fb.g(index);
                match local_val(self.env.types, target_ty)? {
                    Val::I32 => fb.store32(8 * slot),
                    Val::I64 => fb.store64(8 * slot),
                    Val::F64 => fb.storef64(8 * slot),
                };
                Ok(())
            }
            None => {
                if is_managed(self.env.types, target_ty) {
                    fb.g(index);
                    self.call_helper(fb, H::Release)?;
                }
                fb.s(index);
                Ok(())
            }
        }
    }

    /// Emit an operand as a value of the destination language type. The only
    /// contextual representation exception is legacy nominal null (`0`): an
    /// integer constant is normally i64, while a wasm aggregate pointer is i32.
    pub(super) fn emit_owned_for_type(
        &self,
        fb: &mut FB,
        value: &Operand,
        destination: TypeId,
    ) -> Result<Class, CodegenError> {
        if matches!(value, Operand::Constant(Constant::Integer(0)))
            && matches!(self.env.types.kind(destination), TypeKind::Nominal { .. })
        {
            fb.i32c(0);
            Ok(Class::Ptr)
        } else {
            self.emit_owned(fb, value)
        }
    }

    /// `place = operand`: a bare local is an assign; a projected store
    /// walks the chain, releases the old field (managed fields only),
    /// and writes the element/field in place.
    pub(super) fn emit_store(
        &self,
        fb: &mut FB,
        place: lpp_mir::MirPlaceId,
        value: &Operand,
    ) -> Result<(), CodegenError> {
        let resolution = resolve_place(
            self.env.program,
            self.env.types,
            place,
            &self.env.layouts,
            &self.env.aggregates,
            true,
            self.fn_id,
        )?;
        if resolution.steps.is_empty() {
            let _class = self.emit_owned_for_type(fb, value, resolution.ty)?;
            let index = *self
                .local_index
                .get(&resolution.root)
                .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
            if is_managed(self.env.types, resolution.ty) {
                fb.g(index);
                self.call_helper(fb, H::Release)?;
            }
            fb.s(index);
            return Ok(());
        }
        let last = resolution.steps.last().unwrap();
        // The chain leaves exactly one base pointer on the stack. `tee`
        // parks it in a scratch local without duplicating it, so any
        // re-addressing must `get` the scratch (that `get` is the only
        // extra stack copy, and the final store/consumer pops it).
        let base_scratch = fb.scratch(Val::I32);
        self.emit_place_chain(fb, &resolution)?;
        fb.t(base_scratch);
        match last {
            PlaceStep::Field {
                offset,
                ty,
                managed,
            } => {
                // Stack: [base]. The i* store pops (value, addr): the
                // value goes on top of the single base copy.
                if *managed {
                    // Release the old field: re-fetch the base, load the
                    // field, release. The re-fetch is consumed by the
                    // load, leaving the original base on the stack.
                    fb.g(base_scratch);
                    fb.load32(*offset);
                    self.call_helper(fb, H::Release)?;
                }
                let _class = self.emit_owned_for_type(fb, value, *ty)?;
                store_by_type(fb, self.env.types, *offset, *ty);
                // Stack: [] — the store consumed the base and the value.
            }
            PlaceStep::TupleField {
                offset,
                ty,
                managed,
            } => {
                if *managed {
                    fb.g(base_scratch);
                    fb.load32(*offset);
                    self.call_helper(fb, H::Release)?;
                }
                self.emit_owned_for_type(fb, value, *ty)?;
                store_by_type(fb, self.env.types, *offset, *ty);
            }
            PlaceStep::ListIndex { element, index } => {
                // [base] -> get scratch -> [base, base] -> idx -> value
                // -> the ListSet* helper pops (value, idx, ptr), leaving
                // the chain's base copy to drop.
                fb.g(base_scratch);
                self.emit_operand(fb, index)?;
                let h = match element_class(self.env.types, *element) {
                    ElementClass::I64 => {
                        let _class = self.emit_operand(fb, value)?;
                        widen_to_i64(fb, self.env.types, *element);
                        H::ListSetI64
                    }
                    ElementClass::F64 => {
                        self.emit_operand(fb, value)?;
                        H::ListSetF64
                    }
                    ElementClass::Ptr => {
                        // The helper releases the replaced element and
                        // retains the new one (runtime-internal). Null must be
                        // emitted as the list element's i32 pointer type.
                        if matches!(value, Operand::Constant(Constant::Integer(0)))
                            && matches!(self.env.types.kind(*element), TypeKind::Nominal { .. })
                        {
                            fb.i32c(0);
                        } else {
                            self.emit_operand(fb, value)?;
                        }
                        H::ListSetPtr
                    }
                };
                self.call_helper(fb, h)?;
                // Stack: [base] — drop the chain's copy (the root local
                // still owns its reference).
                fb.op(op::DROP);
            }
            PlaceStep::Downcast => {
                return Err(unsupported("store through downcast", Some(self.fn_id)));
            }
        }
        Ok(())
    }

    /// Evaluate the projection chain up to (excluding) the final step;
    /// leaves the final base pointer on the stack. Intermediate steps
    /// are plain borrows (no retain): a field load or an ARC-element
    /// read the caller does not release.
    pub(super) fn emit_place_chain(
        &self,
        fb: &mut FB,
        resolution: &PlaceResolution,
    ) -> Result<(), CodegenError> {
        let root_index = *self
            .local_index
            .get(&resolution.root)
            .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
        fb.g(root_index);
        for step in resolution.steps.iter().take(resolution.steps.len() - 1) {
            match step {
                PlaceStep::Downcast => {}
                PlaceStep::Field { offset, .. } | PlaceStep::TupleField { offset, .. } => {
                    // Intermediate fields are managed (verified), i32 slots.
                    fb.load32(*offset);
                }
                PlaceStep::ListIndex { element, index } => {
                    // Stack: [base] → emit the index → [base, idx] →
                    // the plain element read → [element].
                    self.emit_operand(fb, index)?;
                    match element_class(self.env.types, *element) {
                        // Intermediate elements are managed (verified).
                        ElementClass::Ptr => self.call_helper(fb, H::ListGetPtr)?,
                        ElementClass::I64 | ElementClass::F64 => {
                            return Err(unsupported(
                                "projection through scalar list element",
                                Some(self.fn_id),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// `Load(place)`: a bare local is a plain read (no retain); a
    /// projected read loads the field/element and retains when managed
    /// (the container keeps its own reference).
    pub(super) fn emit_load_place(
        &self,
        fb: &mut FB,
        place: lpp_mir::MirPlaceId,
    ) -> Result<Class, CodegenError> {
        let resolution = resolve_place(
            self.env.program,
            self.env.types,
            place,
            &self.env.layouts,
            &self.env.aggregates,
            false,
            self.fn_id,
        )?;
        if resolution.steps.is_empty() {
            return self.emit_operand(fb, &Operand::Copy(resolution.root));
        }
        let last = resolution.steps.last().unwrap();
        // The chain leaves exactly one base pointer on the stack; the final
        // step consumes it (field load pops it, list getter pops it with the
        // index), so nothing leaks.
        self.emit_place_chain(fb, &resolution)?;
        match last {
            PlaceStep::Field {
                offset,
                ty,
                managed,
            } => {
                load_by_type(fb, self.env.types, *offset, *ty);
                if *managed {
                    // The loaded pointer is a fresh owner: Retain passes
                    // the value through, so the caller keeps it and the
                    // reference count is bumped. (No tee: scratch is not
                    // part of the exit-release set — parking a copy there
                    // would leak the reference.)
                    self.call_helper(fb, H::Retain)?;
                }
                class_of_type(self.env.types, *ty)
            }
            PlaceStep::TupleField {
                offset,
                ty,
                managed,
            } => {
                load_by_type(fb, self.env.types, *offset, *ty);
                if *managed {
                    self.call_helper(fb, H::Retain)?;
                }
                class_of_type(self.env.types, *ty)
            }
            PlaceStep::ListIndex { element, index } => {
                self.emit_operand(fb, index)?;
                match element_class(self.env.types, *element) {
                    ElementClass::I64 => {
                        self.call_helper(fb, H::ListGetI64)?;
                        if !matches!(
                            self.env.types.kind(*element),
                            TypeKind::Primitive(PrimitiveType::Int)
                        ) {
                            fb.op(op::I32_WRAP_I64);
                        }
                        class_of_type(self.env.types, *element)
                    }
                    ElementClass::F64 => {
                        self.call_helper(fb, H::ListGetF64)?;
                        Ok(Class::F64)
                    }
                    ElementClass::Ptr => {
                        // The getter returns the element's pointer without
                        // retaining; the load creates a fresh owner, so
                        // Retain (pass-through) bumps it. No tee — scratch
                        // is not released by the exit pass.
                        self.call_helper(fb, H::ListGetPtr)?;
                        self.call_helper(fb, H::Retain)?;
                        Ok(Class::Ptr)
                    }
                }
            }
            PlaceStep::Downcast => Err(unsupported("load through downcast", Some(self.fn_id))),
        }
    }

    /// A list literal `[e1, e2, …]` of element type `element`: `ListNew`
    /// then one push per element (a `Copy` element retains for the
    /// list; the source keeps its own reference).
    pub(super) fn emit_list_literal(
        &self,
        fb: &mut FB,
        target_ty: TypeId,
        items: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let TypeKind::List(element) = self.env.types.kind(target_ty) else {
            return Err(unsupported("list literal of non-list", Some(self.fn_id)));
        };
        let ec = element_class(self.env.types, element);
        let list = fb.scratch(Val::I32);
        fb.i32c(if ec == ElementClass::Ptr { 1 } else { 0 });
        self.call_helper(fb, H::ListNew)?;
        fb.s(list);
        for operand in self.env.program.operands(*items) {
            fb.g(list);
            let _class = self.emit_owned(fb, operand)?;
            match ec {
                ElementClass::I64 => {
                    widen_to_i64(fb, self.env.types, element);
                    self.call_helper(fb, H::ListPushI64)?;
                }
                ElementClass::F64 => {
                    self.call_helper(fb, H::ListPushF64)?;
                }
                ElementClass::Ptr => {
                    self.call_helper(fb, H::ListPushPtr)?;
                }
            }
        }
        fb.g(list);
        Ok(Class::Ptr)
    }

    pub(super) fn emit_tuple_literal(
        &self,
        fb: &mut FB,
        target_ty: TypeId,
        items: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let TypeKind::Tuple(elements) = self.env.types.kind(target_ty) else {
            return Err(unsupported("tuple literal of non-tuple", Some(self.fn_id)));
        };
        let element_types = self.env.types.list(elements);
        let values = self.env.program.operands(*items);
        if values.len() != element_types.len() || values.len() > 64 {
            return Err(unsupported("tuple arity", Some(self.fn_id)));
        }
        let drop_slot = self
            .env
            .tuple_drop_slot
            .ok_or_else(|| unsupported("missing tuple destroyer", Some(self.fn_id)))?;
        let tuple = fb.scratch(Val::I32);
        fb.i32c((16 + values.len() * 8) as i64);
        fb.i32c(drop_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        fb.s(tuple);
        let mut managed_mask = 0u64;
        for (index, (&ty, value)) in element_types.iter().zip(values.iter()).enumerate() {
            if is_managed(self.env.types, ty) {
                managed_mask |= 1u64 << index;
            }
            fb.g(tuple);
            self.emit_owned(fb, value)?;
            store_by_type(fb, self.env.types, 16 + 8 * index as u32, ty);
        }
        fb.g(tuple).i64c(managed_mask as i64).store64(0);
        fb.g(tuple).i64c(values.len() as i64).store64(8);
        fb.g(tuple);
        Ok(Class::Ptr)
    }

    pub(super) fn emit_rvalue(&self, fb: &mut FB, rvalue: &Rvalue) -> Result<Class, CodegenError> {
        match rvalue {
            Rvalue::Use(operand) => self.emit_operand(fb, operand),
            Rvalue::Unary { operator, operand } => self.emit_unary(fb, *operator, operand),
            Rvalue::Binary {
                left,
                operator,
                right,
            } => self.emit_binary(fb, left, *operator, right),
            Rvalue::Call { callee, arguments } => self.emit_call(fb, *callee, arguments),
            Rvalue::ExternalCall { .. } => Err(unsupported("external call", Some(self.fn_id))),
            Rvalue::Builtin { builtin, arguments } => self.emit_builtin(fb, *builtin, arguments),
            Rvalue::Load(place) => self.emit_load_place(fb, *place),
            // List literals are produced by `emit_assign` (the target
            // type supplies the element class of `[]`).
            Rvalue::List(_) => Err(unsupported("list literal outside assign", Some(self.fn_id))),
            Rvalue::Tuple(_) => Err(unsupported("tuple", Some(self.fn_id))),
            Rvalue::ConstructStruct { aggregate, fields } => {
                self.emit_construct(fb, *aggregate, None, fields)
            }
            Rvalue::ConstructVariant {
                aggregate,
                variant,
                fields,
            } => {
                let ordinal = self.env.program.variant(*variant).unwrap().ordinal;
                self.emit_construct(fb, *aggregate, Some(ordinal), fields)
            }
            Rvalue::ListLen(operand) => {
                self.emit_operand(fb, operand)?;
                self.call_helper(fb, H::ListLen)?;
                Ok(Class::I64)
            }
            Rvalue::MakeClosure { function, captures } => {
                self.emit_make_closure(fb, *function, captures)
            }
            Rvalue::Await(operand) => self.emit_await(fb, operand),
            Rvalue::Spawn(operand) => self.emit_spawn(fb, operand),
        }
    }

    /// `x.await` (5D2b slice 3): `TaskAwait(x)` — polls when fresh and
    /// retains a managed result — then unboxes the result word to the
    /// type behind the `Task`.
    pub(super) fn emit_await(&self, fb: &mut FB, operand: &Operand) -> Result<Class, CodegenError> {
        let Operand::Copy(local) = operand else {
            return Err(unsupported("await of non-local task", Some(self.fn_id)));
        };
        let ty = self.env.program.local(*local).unwrap().ty;
        let inner = match self.env.types.kind(ty) {
            TypeKind::Task(inner) => inner,
            _ => return Err(unsupported("await of non-task", Some(self.fn_id))),
        };
        self.emit_operand(fb, operand)?;
        self.call_helper(fb, H::TaskAwait)?;
        if is_void(self.env.types, inner) {
            fb.op(op::DROP);
            fb.i64c(0);
            return Ok(Class::I64);
        }
        match local_val(self.env.types, inner)? {
            Val::I64 => Ok(Class::I64),
            Val::F64 => {
                fb.op(op::F64_REINTERPRET_I64);
                Ok(Class::F64)
            }
            Val::I32 => {
                fb.op(op::I32_WRAP_I64);
                if matches!(
                    self.env.types.kind(inner),
                    TypeKind::List(_) | TypeKind::Nominal { .. } | TypeKind::Task(_)
                ) {
                    Ok(Class::Ptr)
                } else {
                    Ok(Class::Bool)
                }
            }
        }
    }

    /// `spawn(x)` (5D2b slice 3): eager — build the task (code from
    /// the capsule, env a wrapper tuple holding the retained closure
    /// env or an empty tuple for a bare function), poll it to
    /// completion, and destroy the node. The statement yields nothing.
    pub(super) fn emit_spawn(&self, fb: &mut FB, operand: &Operand) -> Result<Class, CodegenError> {
        let Operand::Copy(local) = operand else {
            return Err(unsupported("spawn of non-local value", Some(self.fn_id)));
        };
        let ty = self.env.program.local(*local).unwrap().ty;
        let TypeKind::Function { .. } = self.env.types.kind(ty) else {
            return Err(unsupported("spawn of non-function", Some(self.fn_id)));
        };
        let origin = self
            .env
            .value_origins
            .get(&(self.fn_id, *local))
            .copied()
            .ok_or_else(|| {
                unsupported(
                    "spawn of a function value of unknown origin",
                    Some(self.fn_id),
                )
            })?;
        let origin_fn = self.env.program.function(origin).unwrap();
        let index = *self
            .local_index
            .get(local)
            .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
        // The task env: a 1-slot wrapper tuple whose slot 16 holds the
        // capsule env (NULL for a zero-capture closure or a bare
        // function — the slot is then simply 0).
        let tuple_slot = self
            .env
            .tuple_drop_slot
            .ok_or_else(|| unsupported("missing tuple destroyer", Some(self.fn_id)))?;
        let is_closure = matches!(origin_fn.kind, lpp_mir::MirFunctionKind::Closure);
        let is_sync_bare = matches!(origin_fn.kind, lpp_mir::MirFunctionKind::Function);
        if !is_closure && !is_sync_bare {
            return Err(unsupported("spawn of an async value", Some(self.fn_id)));
        }
        let code = if is_closure {
            *self
                .env
                .closure_thunk_dispatch
                .get(&origin)
                .ok_or_else(|| unsupported("undispatched closure thunk", Some(self.fn_id)))?
        } else {
            *self
                .env
                .task_thunk_dispatch
                .get(&origin)
                .ok_or_else(|| unsupported("undispatched task thunk", Some(self.fn_id)))?
        };
        fb.i32c(24);
        fb.i32c(tuple_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        let tuple = fb.scratch(Val::I32);
        fb.t(tuple);
        fb.g(tuple).i64c(if is_closure { 1 } else { 0 }).store64(0);
        fb.g(tuple).i64c(if is_closure { 16 } else { 0 }).store64(8);
        // The capsule pointer, retained for the wrapper tuple: the
        // closure thunk reads the closure env through it (a
        // bare-function wrapper stores 0 instead). The local already
        // HOLDS the capsule pointer (an i32 heap offset) — retain it
        // directly; no dereference.
        fb.g(index);
        self.call_helper(fb, H::Retain)?;
        let cap = fb.scratch(Val::I32);
        fb.t(cap);
        fb.g(tuple).g(cap).op(op::I64_EXTEND_I32_S).store64(16);
        fb.g(cap).op(op::DROP);
        // TaskNew(code, env, 0) — a spawned closure/function is void.
        fb.i32c(code as i64);
        fb.g(tuple);
        fb.i32c(0);
        self.call_helper(fb, H::TaskNew)?;
        // TaskNew returns the node pointer as an i64 word; narrow
        // for the wasm ABI before the scratch tee.
        let task = fb.scratch(Val::I32);
        fb.op(op::I32_WRAP_I64);
        fb.t(task);
        // Eager: poll to completion, then release the node.
        fb.g(task)
            .call(self.env.helper_index[&H::TaskPoll])
            .op(op::DROP);
        fb.g(task).call(self.env.helper_index[&H::TaskDestroy]);
        fb.i64c(0);
        Ok(Class::I64)
    }

    /// `ArcAlloc(total, drop-slot)` then one field store per slot: a
    /// `Copy` field retains for the new owner (the source keeps its own
    /// reference); a constructed field lands with its initial reference.
    pub(super) fn emit_construct(
        &self,
        fb: &mut FB,
        aggregate: MirAggregateId,
        tag: Option<u32>,
        fields: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let layout = self
            .env
            .layouts
            .get(&aggregate)
            .ok_or_else(|| unsupported("unlaid-out nominal", Some(self.fn_id)))?
            .clone();
        let drop_slot = *self
            .env
            .drop_slot
            .get(&aggregate)
            .ok_or_else(|| unsupported("unregistered drop slot", Some(self.fn_id)))?;
        let ptr = fb.scratch(Val::I32);
        fb.i32c(layout.total_size as i64);
        fb.i32c(drop_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        fb.s(ptr);
        if let Some(ordinal) = tag {
            // The i32 tag at offset 0 (bytes 4..8 stay the padding of
            // the native 8-byte tag — the shared layout is identical).
            fb.g(ptr);
            fb.i32c(ordinal as i64);
            fb.store32(ENUM_TAG as u32);
        }
        let slots = match tag {
            None => &layout.struct_fields,
            Some(ordinal) => &layout.variant_fields[ordinal as usize],
        };
        let values = self.env.program.operands(*fields);
        if values.len() != slots.len() {
            return Err(unsupported("aggregate field count", Some(self.fn_id)));
        }
        for (operand, slot) in values.iter().zip(slots.iter()) {
            // wasm stores pop (value, addr): address first, value on top.
            fb.g(ptr);
            if matches!(operand, Operand::Constant(Constant::Integer(0)))
                && matches!(self.env.types.kind(slot.ty), TypeKind::Nominal { .. })
            {
                // Context-typed legacy null: nominal slots are i32 pointers,
                // while a standalone integer constant would otherwise emit i64.
                fb.i32c(0);
            } else {
                let _class = self.emit_owned(fb, operand)?;
            }
            store_by_type(fb, self.env.types, slot.offset, slot.ty);
        }
        fb.g(ptr);
        Ok(Class::Ptr)
    }

    pub(super) fn emit_unary(
        &self,
        fb: &mut FB,
        operator: UnaryOperator,
        operand: &Operand,
    ) -> Result<Class, CodegenError> {
        let class = operand_class(self.env.program, self.env.types, operand)?;
        match (operator, class) {
            (UnaryOperator::Not, Class::Bool) => {
                self.emit_operand(fb, operand)?;
                fb.op(op::I32_EQZ);
                Ok(Class::Bool)
            }
            (UnaryOperator::Not, Class::I64) => {
                self.emit_operand(fb, operand)?;
                fb.op(op::I64_EQZ);
                Ok(Class::Bool)
            }
            (UnaryOperator::Negate, Class::I64) => {
                fb.i64c(0);
                self.emit_operand(fb, operand)?;
                fb.op(op::I64_SUB); // 0 - value
                Ok(Class::I64)
            }
            (UnaryOperator::Negate, Class::F64) => {
                self.emit_operand(fb, operand)?;
                fb.op(op::F64_NEG);
                Ok(Class::F64)
            }
            _ => Err(unsupported(
                "unary operator on this operand type",
                Some(self.fn_id),
            )),
        }
    }

    pub(super) fn emit_binary(
        &self,
        fb: &mut FB,
        left: &Operand,
        operator: BinaryOperator,
        right: &Operand,
    ) -> Result<Class, CodegenError> {
        let left_static = operand_class(self.env.program, self.env.types, left)?;
        let right_static = operand_class(self.env.program, self.env.types, right)?;
        let left_is_null = matches!(left, Operand::Constant(Constant::Integer(0)));
        let right_is_null = matches!(right, Operand::Constant(Constant::Integer(0)));
        let lc = if left_is_null && right_static == Class::Ptr {
            fb.i32c(0);
            Class::Ptr
        } else {
            self.emit_operand(fb, left)?
        };
        let rc = if right_is_null && left_static == Class::Ptr {
            fb.i32c(0);
            Class::Ptr
        } else {
            self.emit_operand(fb, right)?
        }; // stack: [l, r]
        if lc != rc {
            return Err(unsupported(
                "binary operand type mismatch",
                Some(self.fn_id),
            ));
        }
        if lc == Class::I64 && matches!(operator, BinaryOperator::Divide | BinaryOperator::Modulo) {
            // WASM traps on i64.div_s / i64.rem_s for zero; the L++
            // contract is the same typed runtime panic as the MIR oracle
            // (WASI exit 101). `INT_MIN / -1` is also handled explicitly:
            // use divisor 1 for that pair, yielding the oracle's wrapped
            // quotient (INT_MIN) and remainder (0), never a host trap.
            let left = fb.scratch(Val::I64);
            let right = fb.scratch(Val::I64);
            let overflow = fb.scratch(Val::I32);
            let safe_right = fb.scratch(Val::I64);
            fb.s(right);
            fb.s(left);
            fb.g(right).i64c(0).op(op::I64_EQ).if_();
            fb.i32c(101).call(self.env.proc_exit_import_idx);
            fb.op(op::UNREACHABLE);
            fb.end();
            fb.g(left)
                .i64c(i64::MIN)
                .op(op::I64_EQ)
                .g(right)
                .i64c(-1)
                .op(op::I64_EQ)
                .op(op::I32_AND)
                .s(overflow);
            fb.i64c(1).g(right).g(overflow).op(op::SELECT).s(safe_right);
            fb.g(left).g(safe_right);
            fb.op(if operator == BinaryOperator::Divide {
                op::I64_DIV_S
            } else {
                op::I64_REM_S
            });
            return Ok(Class::I64);
        }
        match (operator, lc) {
            (BinaryOperator::Equal, Class::Ptr) => {
                fb.op(op::I32_EQ);
                Ok(Class::Bool)
            }
            (BinaryOperator::NotEqual, Class::Ptr) => {
                fb.op(op::I32_NE);
                Ok(Class::Bool)
            }
            (_, Class::Ptr) => Err(unsupported(
                "binary operator on managed type",
                Some(self.fn_id),
            )),
            // --- String operations ---
            (BinaryOperator::Add, Class::Str) => {
                self.call_helper(fb, H::StrConcat)?;
                Ok(Class::Str)
            }
            (BinaryOperator::Equal, Class::Str) => {
                self.call_helper(fb, H::StrEq)?;
                Ok(Class::Bool)
            }
            (BinaryOperator::NotEqual, Class::Str) => {
                self.call_helper(fb, H::StrEq)?;
                fb.op(op::I32_EQZ);
                Ok(Class::Bool)
            }
            (_, Class::Str) => Err(unsupported("string operator", Some(self.fn_id))),
            // --- Int (i64) ---
            (op_, Class::I64) => {
                self.i64_op(fb, op_)?;
                Ok(match op_ {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEqual
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::LogicalAnd
                    | BinaryOperator::LogicalOr => Class::Bool,
                    _ => Class::I64,
                })
            }
            // --- Float (f64) ---
            (op_, Class::F64) => {
                self.f64_op(fb, op_)?;
                Ok(match op_ {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEqual
                    | BinaryOperator::GreaterEqual => Class::Bool,
                    _ => Class::F64,
                })
            }
            // --- Bool / Char (i32) ---
            (op_, Class::Bool | Class::Char) => {
                self.i32_op(fb, op_)?;
                Ok(match op_ {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEqual
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::LogicalAnd
                    | BinaryOperator::LogicalOr => Class::Bool,
                    _ => Class::I64,
                })
            }
        }
    }
}
