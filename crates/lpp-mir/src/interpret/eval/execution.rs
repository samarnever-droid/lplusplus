use super::*;

impl Interpreter<'_> {
    pub(in crate::interpret) fn execute_entry(
        &mut self,
        function_id: MirFunctionId,
        arguments: &[ExecutionValue],
    ) -> Result<ExecutionValue, InterpreterError> {
        let Some(function) = self.program.function(function_id).copied() else {
            return Err(self.error(
                None,
                None,
                None,
                InterpreterErrorKind::MissingFunction(function_id),
            ));
        };
        let parameters = self.program.function_parameters(&function);
        if arguments.len() != parameters.len() {
            return Err(self.error(
                Some(function_id),
                None,
                Some(function.origin),
                InterpreterErrorKind::ArityMismatch {
                    expected: parameters.len(),
                    actual: arguments.len(),
                },
            ));
        }
        for (parameter, value) in parameters.iter().zip(arguments) {
            let local = self.program.local(*parameter).unwrap();
            if !value_matches_type(value, local.ty, self.types, self.program) {
                return Err(self.error(
                    Some(function_id),
                    None,
                    Some(local.origin),
                    InterpreterErrorKind::ValueTypeMismatch {
                        expected: local.ty,
                        actual: value.kind(),
                    },
                ));
            }
        }
        let mut runtime_arguments = Vec::with_capacity(arguments.len());
        for (parameter, value) in parameters.iter().zip(arguments) {
            let local = self.program.local(*parameter).unwrap();
            runtime_arguments.push(self.import_value(
                function_id,
                function.origin,
                value,
                Some(local.ty),
            )?);
        }
        // An async entry is auto-drained: the top-level task runs to
        // completion and the resolved value is exposed, not the task handle.
        let result = if function.kind == crate::MirFunctionKind::Async {
            let arguments = if self.arc.is_some() {
                // The task node takes the entry's argument references.
                std::mem::take(&mut runtime_arguments)
            } else {
                runtime_arguments.clone()
            };
            let id = self.allocate_heap(
                function_id,
                None,
                function.origin,
                HeapNode::Task {
                    function: function_id,
                    arguments,
                    closure: None,
                    result: None,
                },
                None,
            )?;
            self.mark_task_node(id, function_id);
            self.run_task(id, function_id, function.entry, function.origin, 0)?;
            let value = match &self.heap[id.0 as usize] {
                HeapNode::Task {
                    result: Some(value),
                    ..
                } => value.clone(),
                _ => unreachable!("a drained task holds its result"),
            };
            // The entry result gains its own reference; the task node
            // keeps its share until the internal handle is dropped.
            if self.arc.is_some() {
                self.retain_owned(function_id, None, Some(function.origin), &value)?;
                self.drop_reference(
                    &RuntimeValue::Task(id),
                    Some(function_id),
                    None,
                    Some(function.origin),
                )?;
            }
            value
        } else {
            self.execute_function(function_id, &runtime_arguments, 0, None)?
        };
        let public = self.export_value(function_id, function.origin, &result)?;
        // The execution boundary holds no references: the entry result
        // is the last owned reference and is dropped before the balance
        // proof (the public outcome is a materialized copy).
        if self.arc.is_some() {
            self.drop_reference(&result, Some(function_id), None, Some(function.origin))?;
        }
        if !value_matches_type(&public, function.return_type, self.types, self.program) {
            return Err(self.error(
                Some(function_id),
                None,
                Some(function.origin),
                InterpreterErrorKind::ValueTypeMismatch {
                    expected: function.return_type,
                    actual: public.kind(),
                },
            ));
        }
        Ok(public)
    }

    /// Execute `function_id`. When the entry comes from a closure heap
    /// node (`capture_cell`), the closure's capture slots — which precede
    /// the closure parameters — are written back to that node on return so
    /// the captured cells persist across calls of the same closure value.
    pub(super) fn execute_function(
        &mut self,
        function_id: MirFunctionId,
        arguments: &[RuntimeValue],
        depth: usize,
        capture_cell: Option<HeapId>,
    ) -> Result<RuntimeValue, InterpreterError> {
        let Some(function) = self.program.function(function_id).copied() else {
            return Err(self.error(
                None,
                None,
                None,
                InterpreterErrorKind::MissingFunction(function_id),
            ));
        };
        if depth >= self.limits.max_call_depth {
            return Err(self.error(
                Some(function_id),
                None,
                Some(function.origin),
                InterpreterErrorKind::LimitExceeded(InterpreterLimit::CallDepth),
            ));
        }
        self.calls = self.calls.saturating_add(1);
        self.peak_call_depth = self.peak_call_depth.max(depth.saturating_add(1));
        let parameters = self.program.function_parameters(&function);
        if arguments.len() != parameters.len() {
            return Err(self.error(
                Some(function_id),
                None,
                Some(function.origin),
                InterpreterErrorKind::ArityMismatch {
                    expected: parameters.len(),
                    actual: arguments.len(),
                },
            ));
        }
        let mut frame = Frame::new(self.program.function_locals(&function));
        for (parameter, value) in parameters.iter().zip(arguments) {
            let local = self.program.local(*parameter).unwrap();
            self.reserve_runtime_clone(function_id, None, local.origin, value)?;
            frame.set(*parameter, value.clone());
        }

        let mut current = function.entry;
        loop {
            let block = self
                .program
                .block(current)
                .expect("verified MIR retains every block");
            for instruction_id in self.program.block_instructions(block) {
                let instruction = self.program.instruction(*instruction_id).unwrap();
                self.tick(function_id, current, instruction.origin)?;
                match instruction.kind {
                    InstructionKind::Assign { target, value } => {
                        let target_type = self.program.local(target).unwrap().ty;
                        let value = self.eval_rvalue(
                            function_id,
                            current,
                            instruction.origin,
                            value,
                            &mut frame,
                            depth,
                            Some(target_type),
                        )?;
                        // Cell death on reassignment: the old value's
                        // reference is dropped before the new one lands.
                        if self.arc.is_some() {
                            if let Some(old) = frame.get(target).cloned() {
                                self.drop_reference(
                                    &old,
                                    Some(function_id),
                                    Some(current),
                                    Some(instruction.origin),
                                )?;
                            }
                        }
                        frame.set(target, value);
                    }
                    InstructionKind::Store { place, value } => {
                        let from_local = matches!(value, Operand::Copy(_));
                        let value = self.eval_operand(
                            function_id,
                            current,
                            instruction.origin,
                            value,
                            &frame,
                        )?;
                        // A place store from a local read creates a new
                        // owner: the read retains for the place and the
                        // source keeps its own reference.
                        if self.arc.is_some() && from_local && owns_heap_refs(&value) {
                            self.retain_owned(
                                function_id,
                                Some(current),
                                Some(instruction.origin),
                                &value,
                            )?;
                        }
                        self.store_place(
                            function_id,
                            current,
                            instruction.origin,
                            place,
                            value,
                            &mut frame,
                        )?;
                    }
                }
            }
            self.tick(function_id, current, block.origin)?;
            match block.terminator {
                Terminator::Goto(target) => current = target,
                Terminator::Branch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    let value =
                        self.eval_operand(function_id, current, block.origin, condition, &frame)?;
                    current = match value {
                        RuntimeValue::Bool(true) => then_block,
                        RuntimeValue::Bool(false) => else_block,
                        RuntimeValue::Int(value) if value != 0 => then_block,
                        RuntimeValue::Int(_) => else_block,
                        other => {
                            return Err(self.error(
                                Some(function_id),
                                Some(current),
                                Some(block.origin),
                                InterpreterErrorKind::InvalidCondition(other.kind()),
                            ));
                        }
                    };
                }
                Terminator::SwitchEnum {
                    subject,
                    aggregate,
                    targets,
                } => {
                    let value =
                        self.eval_operand(function_id, current, block.origin, subject, &frame)?;
                    let RuntimeValue::Nominal(id) = value else {
                        return Err(self.error(
                            Some(function_id),
                            Some(current),
                            Some(block.origin),
                            InterpreterErrorKind::InvalidAggregateProjection,
                        ));
                    };
                    let HeapNode::Nominal {
                        aggregate: actual_aggregate,
                        variant: Some(variant),
                        ..
                    } = &self.heap[id.0 as usize]
                    else {
                        return Err(self.error(
                            Some(function_id),
                            Some(current),
                            Some(block.origin),
                            InterpreterErrorKind::InvalidAggregateProjection,
                        ));
                    };
                    if *actual_aggregate != aggregate {
                        return Err(self.error(
                            Some(function_id),
                            Some(current),
                            Some(block.origin),
                            InterpreterErrorKind::InvalidAggregateProjection,
                        ));
                    }
                    let ordinal = self
                        .program
                        .variant(*variant)
                        .expect("verified enum values reference a variant descriptor")
                        .ordinal as usize;
                    current = self.program.switch_targets(targets)[ordinal];
                }
                Terminator::Return(value) => {
                    let value = value
                        .map(|value| {
                            self.eval_operand_moved(
                                function_id,
                                current,
                                block.origin,
                                value,
                                &mut frame,
                            )
                        })
                        .transpose()?
                        .unwrap_or(RuntimeValue::Void);
                    self.writeback_captures(function_id, capture_cell, &mut frame)?;
                    // Cell death at function exit: every slot that still
                    // owns a heap reference releases it; moved-out slots
                    // are already empty and emit nothing (move-out).
                    if self.arc.is_some() {
                        for &local in self.program.function_locals(&function) {
                            if let Some(old) = frame.take(local) {
                                self.drop_reference(
                                    &old,
                                    Some(function_id),
                                    None,
                                    Some(block.origin),
                                )?;
                            }
                        }
                    }
                    return Ok(value);
                }
                Terminator::Unreachable => {
                    return Err(self.error(
                        Some(function_id),
                        Some(current),
                        Some(block.origin),
                        InterpreterErrorKind::ReachedUnreachable,
                    ));
                }
            }
        }
    }

    /// Persist the closure's capture slots into its heap node. The capture
    /// frame slots are the first `captures.len()` parameters of the
    /// closure function; everything after them belongs to the call.
    /// Under ARC the frame's reference moves into the cell (the cell's
    /// old value is released first); if the body moved the capture out,
    /// the frame slot is empty and the cell keeps its own reference.
    pub(super) fn writeback_captures(
        &mut self,
        function_id: MirFunctionId,
        capture_cell: Option<HeapId>,
        frame: &mut Frame,
    ) -> Result<(), InterpreterError> {
        let Some(cell) = capture_cell else {
            return Ok(());
        };
        let Some(function) = self.program.function(function_id).copied() else {
            return Ok(());
        };
        let parameters = self.program.function_parameters(&function);
        if self.arc.is_some() {
            let node = &self.heap[cell.0 as usize];
            let HeapNode::Closure { captures, .. } = node else {
                unreachable!("closure cells reference closure heap nodes");
            };
            let taken: Vec<Option<RuntimeValue>> = parameters
                .iter()
                .take(captures.len())
                .map(|parameter| frame.take(*parameter))
                .collect();
            let old: Vec<RuntimeValue> = taken
                .iter()
                .enumerate()
                .filter(|(_, value)| value.is_some())
                .map(|(position, _)| captures[position].clone())
                .collect();
            for value in &old {
                self.drop_reference(value, None, None, None)?;
            }
            let node = &mut self.heap[cell.0 as usize];
            if let HeapNode::Closure { captures, .. } = node {
                for (position, value) in taken.iter().enumerate() {
                    if let Some(value) = value {
                        captures[position] = value.clone();
                    }
                }
            }
            return Ok(());
        }
        let HeapNode::Closure { captures, .. } = &mut self.heap[cell.0 as usize] else {
            unreachable!("closure cells reference closure heap nodes");
        };
        for (slot, parameter) in captures.iter_mut().zip(parameters.iter()) {
            if let Some(value) = frame.get(*parameter) {
                *slot = value.clone();
            }
        }
        Ok(())
    }

    pub(super) fn eval_rvalue(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        value: Rvalue,
        frame: &mut Frame,
        depth: usize,
        // The type of the local the value is produced into; recorded on
        // heap allocations so the pinned set resolves per node.
        produced: Option<TypeId>,
    ) -> Result<RuntimeValue, InterpreterError> {
        match value {
            Rvalue::Use(operand) => {
                let value = self.eval_operand(function, block, origin, operand, frame)?;
                // A read that creates a new owner retains; the source
                // keeps its own reference.
                if self.arc.is_some()
                    && matches!(operand, Operand::Copy(_))
                    && owns_heap_refs(&value)
                {
                    self.retain_owned(function, Some(block), Some(origin), &value)?;
                }
                Ok(value)
            }
            Rvalue::Unary { operator, operand } => {
                let value = self.eval_operand_borrowed(function, block, origin, operand, frame)?;
                self.eval_unary(function, block, origin, operator, value)
            }
            Rvalue::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.eval_operand_borrowed(function, block, origin, left, frame)?;
                let right = self.eval_operand_borrowed(function, block, origin, right, frame)?;
                self.eval_binary(function, block, origin, operator, left, right)
            }
            Rvalue::Tuple(operands) => {
                // Elements move into the inline tuple.
                let mut values = Vec::new();
                for operand in self.program.operands(operands) {
                    values.push(self.eval_operand_moved(function, block, origin, *operand, frame)?);
                }
                self.reserve_elements(function, Some(block), origin, values.len())?;
                Ok(RuntimeValue::Tuple(values))
            }
            Rvalue::List(operands) => {
                // Elements move into the list node.
                let mut values = Vec::new();
                for operand in self.program.operands(operands) {
                    values.push(self.eval_operand_moved(function, block, origin, *operand, frame)?);
                }
                self.reserve_elements(function, Some(block), origin, values.len())?;
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::List(values),
                    produced,
                )?;
                Ok(RuntimeValue::List(id))
            }
            Rvalue::ConstructStruct { aggregate, fields } => {
                // Fields move into the nominal node.
                let mut values = Vec::new();
                for operand in self.program.operands(fields) {
                    values.push(self.eval_operand_moved(function, block, origin, *operand, frame)?);
                }
                self.reserve_elements(function, Some(block), origin, values.len())?;
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::Nominal {
                        aggregate,
                        variant: None,
                        fields: values,
                    },
                    produced,
                )?;
                Ok(RuntimeValue::Nominal(id))
            }
            Rvalue::ConstructVariant {
                aggregate,
                variant,
                fields,
            } => {
                // Fields move into the nominal node.
                let mut values = Vec::new();
                for operand in self.program.operands(fields) {
                    values.push(self.eval_operand_moved(function, block, origin, *operand, frame)?);
                }
                self.reserve_elements(function, Some(block), origin, values.len())?;
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::Nominal {
                        aggregate,
                        variant: Some(variant),
                        fields: values,
                    },
                    produced,
                )?;
                Ok(RuntimeValue::Nominal(id))
            }
            Rvalue::Load(place) => {
                let value = self.load_place(function, block, origin, place, frame)?;
                // A field read creates a new reference; the container
                // keeps its own.
                if self.arc.is_some() && owns_heap_refs(&value) {
                    self.retain_owned(function, Some(block), Some(origin), &value)?;
                }
                Ok(value)
            }
            Rvalue::ListLen(list) => {
                let list = self.eval_operand(function, block, origin, list, frame)?;
                let RuntimeValue::List(id) = list else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidListProjection,
                    ));
                };
                let HeapNode::List(values) = &self.heap[id.0 as usize] else {
                    unreachable!("list handles reference list heap nodes")
                };
                Ok(RuntimeValue::Int(values.len() as i64))
            }
            Rvalue::Call { callee, arguments } => {
                // The callee operand is an indirection: invoking a
                // closure never consumes it.
                let callee = self.eval_operand(function, block, origin, callee, frame)?;
                // User-call arguments are borrowed transfers: the callee's
                // parameter frame gains an owned reference while a source
                // local keeps its reference. This is the same contract used
                // by both native and WASM code generation. A string literal's
                // evaluation already creates the reader reference that moves
                // into the parameter, so only local copies retain here.
                let mut argument_values = Vec::new();
                for operand in self.program.operands(arguments) {
                    let value = self.eval_operand(function, block, origin, *operand, frame)?;
                    if self.arc.is_some()
                        && matches!(operand, Operand::Copy(_))
                        && owns_heap_refs(&value)
                    {
                        self.retain_owned(function, Some(block), Some(origin), &value)?;
                    }
                    argument_values.push(value);
                }
                match callee {
                    RuntimeValue::Function(target) => {
                        let is_async = self
                            .program
                            .function(target)
                            .is_some_and(|function| function.kind == crate::MirFunctionKind::Async);
                        if is_async {
                            // Async calls yield a task handle; the task runs
                            // lazily, depth-first, when awaited or drained.
                            self.reserve_elements(
                                function,
                                Some(block),
                                origin,
                                argument_values.len(),
                            )?;
                            let id = self.allocate_heap(
                                function,
                                Some(block),
                                origin,
                                HeapNode::Task {
                                    function: target,
                                    arguments: argument_values,
                                    closure: None,
                                    result: None,
                                },
                                produced,
                            )?;
                            self.mark_task_node(id, target);
                            Ok(RuntimeValue::Task(id))
                        } else {
                            self.execute_function(target, &argument_values, depth + 1, None)
                        }
                    }
                    RuntimeValue::Closure(id) => {
                        let HeapNode::Closure {
                            function: target,
                            captures,
                        } = self.heap[id.0 as usize].clone()
                        else {
                            unreachable!("closure handles reference closure heap nodes")
                        };
                        let mut all = captures;
                        // The capture frame slots temporarily own the
                        // references; writeback moves them back.
                        if self.arc.is_some() {
                            for capture in &all {
                                self.retain_owned(function, Some(block), Some(origin), capture)?;
                            }
                        }
                        all.extend(argument_values);
                        self.execute_function(target, &all, depth + 1, Some(id))
                    }
                    other => Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidCallee(other.kind()),
                    )),
                }
            }
            Rvalue::ExternalCall { .. } => Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::UnsupportedExternalCall,
            )),
            Rvalue::Builtin { builtin, arguments } => {
                // The builtin half of the call contract: value arguments
                // that a builtin stores move; receivers and indices are
                // plain reads. A string literal read in a read-only
                // position gains a reader reference that the read does
                // not store, so it is released after the builtin.
                let operands = self.program.operands(arguments).to_vec();
                let mut argument_values = Vec::new();
                for (position, operand) in operands.iter().enumerate() {
                    if self.arc.is_some() && self.builtin_moves_value(builtin, position) {
                        argument_values.push(
                            self.eval_operand_moved(function, block, origin, *operand, frame)?,
                        );
                    } else {
                        let value = self.eval_operand(function, block, origin, *operand, frame)?;
                        if self.arc.is_some()
                            && matches!(operand, Operand::Constant(Constant::String { .. }))
                        {
                            // The read is a borrow of the interned
                            // literal: release the reader reference.
                            self.drop_reference(&value, Some(function), Some(block), Some(origin))?;
                        }
                        argument_values.push(value);
                    }
                }
                self.eval_builtin(function, block, origin, builtin, &argument_values, produced)
            }
            Rvalue::MakeClosure {
                function: target,
                captures,
            } => {
                let captures = self.eval_operands(function, block, origin, captures, frame)?;
                // The closure node retains each capture; the source
                // slots keep their own references (the 4C3B cell
                // semantics).
                if self.arc.is_some() {
                    for capture in &captures {
                        self.retain_owned(function, Some(block), Some(origin), capture)?;
                    }
                }
                self.reserve_elements(function, Some(block), origin, captures.len())?;
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::Closure {
                        function: target,
                        captures,
                    },
                    produced,
                )?;
                Ok(RuntimeValue::Closure(id))
            }
            Rvalue::Cast { value, target } => {
                let evaluated = self.eval_operand(function, block, origin, value, frame)?;
                match (evaluated, self.types.kind(target)) {
                    (RuntimeValue::Int(v), TypeKind::Primitive(PrimitiveType::Float)) => {
                        Ok(RuntimeValue::FloatBits((v as f64).to_bits()))
                    }
                    (RuntimeValue::FloatBits(bits), TypeKind::Primitive(p))
                        if !matches!(
                            p,
                            PrimitiveType::Float | PrimitiveType::Void | PrimitiveType::String
                        ) =>
                    {
                        let f = f64::from_bits(bits);
                        Ok(RuntimeValue::Int(f as i64))
                    }
                    (RuntimeValue::Int(v), TypeKind::Primitive(PrimitiveType::Bool)) => {
                        Ok(RuntimeValue::Bool(v != 0))
                    }
                    (RuntimeValue::Bool(b), _) => Ok(RuntimeValue::Int(i64::from(b))),
                    (RuntimeValue::Char(c), _) => Ok(RuntimeValue::Int(i64::from(c as u32))),
                    (RuntimeValue::Int(v), TypeKind::Primitive(PrimitiveType::Char)) => {
                        Ok(RuntimeValue::Char(char::from_u32(v as u32).unwrap_or('\0')))
                    }
                    (other, _) => Ok(other),
                }
            }
            Rvalue::Intrinsic {
                intrinsic,
                type_argument,
                arguments,
            } => {
                let arg_vals = self.eval_operands(function, block, origin, arguments, frame)?;
                match intrinsic {
                    crate::KernelIntrinsic::SizeOf | crate::KernelIntrinsic::AlignOf => {
                        let (sz, al) =
                            type_argument.map_or((8_i64, 8_i64), |ty| match self.types.kind(ty) {
                                TypeKind::Primitive(
                                    PrimitiveType::Bool | PrimitiveType::U8 | PrimitiveType::I8,
                                ) => (1, 1),
                                TypeKind::Primitive(PrimitiveType::U16 | PrimitiveType::I16) => {
                                    (2, 2)
                                }
                                TypeKind::Primitive(
                                    PrimitiveType::U32 | PrimitiveType::I32 | PrimitiveType::Char,
                                ) => (4, 4),
                                _ => (8, 8),
                            });
                        Ok(RuntimeValue::Int(
                            if intrinsic == crate::KernelIntrinsic::SizeOf {
                                sz
                            } else {
                                al
                            },
                        ))
                    }
                    crate::KernelIntrinsic::PtrNull => Ok(RuntimeValue::Int(0)),
                    crate::KernelIntrinsic::PtrIsNull => {
                        let is_null = matches!(arg_vals.first(), Some(RuntimeValue::Int(0)));
                        Ok(RuntimeValue::Bool(is_null))
                    }
                    crate::KernelIntrinsic::PtrAdd => {
                        let base = match arg_vals.first() {
                            Some(RuntimeValue::Int(v)) => *v,
                            _ => 0,
                        };
                        let off = match arg_vals.get(1) {
                            Some(RuntimeValue::Int(v)) => *v,
                            _ => 0,
                        };
                        Ok(RuntimeValue::Int(base.wrapping_add(off)))
                    }
                    crate::KernelIntrinsic::MemoryBarrier => Ok(RuntimeValue::Void),
                    crate::KernelIntrinsic::SetNew
                    | crate::KernelIntrinsic::SetAdd
                    | crate::KernelIntrinsic::SetHas
                    | crate::KernelIntrinsic::SetRemove
                    | crate::KernelIntrinsic::SetAddStr
                    | crate::KernelIntrinsic::SetHasStr
                    | crate::KernelIntrinsic::SetRemoveStr
                    | crate::KernelIntrinsic::SetLen
                    | crate::KernelIntrinsic::SetClear
                    | crate::KernelIntrinsic::SetFree
                    | crate::KernelIntrinsic::SetUnion
                    | crate::KernelIntrinsic::SetIntersect
                    | crate::KernelIntrinsic::DequeNew
                    | crate::KernelIntrinsic::DequePushBack
                    | crate::KernelIntrinsic::DequePushFront
                    | crate::KernelIntrinsic::DequePopBack
                    | crate::KernelIntrinsic::DequePopFront
                    | crate::KernelIntrinsic::DequePeekFront
                    | crate::KernelIntrinsic::DequePeekBack
                    | crate::KernelIntrinsic::DequeGet
                    | crate::KernelIntrinsic::DequeLen
                    | crate::KernelIntrinsic::DequeClear
                    | crate::KernelIntrinsic::DequeFree => {
                        self.eval_collection_intrinsic(intrinsic, &arg_vals)
                    }
                    _ => Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::UnsupportedBuiltin,
                    )),
                }
            }
            Rvalue::Await(operand) => {
                // The task handle is a receiver: repeated awaits are
                // legal and the handle stays alive.
                let value = self.eval_operand(function, block, origin, operand, frame)?;
                let RuntimeValue::Task(id) = value else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAwaitTarget(value.kind()),
                    ));
                };
                self.run_task(id, function, block, origin, depth)?;
                let value = match &self.heap[id.0 as usize] {
                    HeapNode::Task {
                        result: Some(value),
                        ..
                    } => value.clone(),
                    _ => {
                        return Err(self.error(
                            Some(function),
                            Some(block),
                            Some(origin),
                            InterpreterErrorKind::InvalidTaskProjection,
                        ));
                    }
                };
                self.reserve_runtime_clone(function, Some(block), origin, &value)?;
                // The task keeps its result; the awaiter gains one.
                if self.arc.is_some() && owns_heap_refs(&value) {
                    self.retain_owned(function, Some(block), Some(origin), &value)?;
                }
                Ok(value)
            }
            Rvalue::Spawn(operand) => {
                // The closure operand is a receiver; the task node
                // retains the closure back-reference and the captures it
                // stores, then the internal handle is dropped when the
                // statement completes.
                let value = self.eval_operand(function, block, origin, operand, frame)?;
                let (target, arguments, closure) = match value {
                    RuntimeValue::Closure(id) => {
                        let HeapNode::Closure {
                            function: target,
                            captures,
                        } = self.heap[id.0 as usize].clone()
                        else {
                            unreachable!("closure handles reference closure heap nodes")
                        };
                        (target, captures, Some(id))
                    }
                    RuntimeValue::Function(target) => (target, Vec::new(), None),
                    other => {
                        return Err(self.error(
                            Some(function),
                            Some(block),
                            Some(origin),
                            InterpreterErrorKind::InvalidCallee(other.kind()),
                        ));
                    }
                };
                if self.arc.is_some() {
                    if let Some(closure) = closure {
                        self.retain_value(
                            function,
                            Some(block),
                            Some(origin),
                            &RuntimeValue::Closure(closure),
                        )?;
                    }
                    for argument in &arguments {
                        self.retain_owned(function, Some(block), Some(origin), argument)?;
                    }
                }
                // Detached tasks execute eagerly and deterministically; the
                // spawned work happens before the spawn statement completes.
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::Task {
                        function: target,
                        arguments,
                        closure,
                        result: None,
                    },
                    None,
                )?;
                self.mark_task_node(id, target);
                self.run_task(id, function, block, origin, depth)?;
                if self.arc.is_some() {
                    self.drop_reference(
                        &RuntimeValue::Task(id),
                        Some(function),
                        Some(block),
                        Some(origin),
                    )?;
                }
                Ok(RuntimeValue::Void)
            }
        }
    }

    pub(super) fn run_task(
        &mut self,
        id: HeapId,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        depth: usize,
    ) -> Result<(), InterpreterError> {
        // Awaiting an already-resolved task is a no-op.
        if let HeapNode::Task {
            result: Some(_), ..
        } = &self.heap[id.0 as usize]
        {
            return Ok(());
        }
        let (target, closure) = match &self.heap[id.0 as usize] {
            HeapNode::Task {
                function: target,
                closure,
                ..
            } => (*target, *closure),
            _ => {
                return Err(self.error(
                    Some(function),
                    Some(block),
                    Some(origin),
                    InterpreterErrorKind::InvalidTaskProjection,
                ));
            }
        };
        let arguments = if self.arc.is_some() {
            // The frame takes the node's argument references; the node
            // gives them up (they are released when the frame exits).
            let mut taken = Vec::new();
            if let HeapNode::Task {
                arguments: slot, ..
            } = &mut self.heap[id.0 as usize]
            {
                taken = std::mem::take(slot);
            }
            taken
        } else {
            let HeapNode::Task { arguments, .. } = &self.heap[id.0 as usize] else {
                unreachable!("task handles reference task heap nodes");
            };
            arguments.clone()
        };
        let value = self.execute_function(target, &arguments, depth + 1, closure)?;
        if let HeapNode::Task { result, .. } = &mut self.heap[id.0 as usize] {
            *result = Some(value);
        } else {
            unreachable!("task handles reference task heap nodes");
        }
        Ok(())
    }

    fn eval_collection_intrinsic(
        &mut self,
        intrinsic: crate::KernelIntrinsic,
        args: &[RuntimeValue],
    ) -> Result<RuntimeValue, InterpreterError> {
        let arg_int = |idx: usize| -> i64 {
            match args.get(idx) {
                Some(RuntimeValue::Int(v)) => *v,
                _ => 0,
            }
        };
        let arg_str = |idx: usize| -> Option<String> {
            match args.get(idx) {
                Some(RuntimeValue::String(id)) => match &self.heap[id.0 as usize] {
                    super::HeapNode::String(s) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            }
        };
        use super::super::ExtSetKey;
        use crate::KernelIntrinsic as KI;
        Ok(match intrinsic {
            KI::SetNew => RuntimeValue::Int(self.ext_collections.alloc_set(Default::default())),
            KI::SetAdd => RuntimeValue::Bool(
                self.ext_collections
                    .set_mut(arg_int(0))
                    .is_some_and(|s| s.insert(ExtSetKey::Int(arg_int(1)))),
            ),
            KI::SetHas => RuntimeValue::Bool(
                self.ext_collections
                    .set(arg_int(0))
                    .is_some_and(|s| s.contains(&ExtSetKey::Int(arg_int(1)))),
            ),
            KI::SetRemove => RuntimeValue::Bool(
                self.ext_collections
                    .set_mut(arg_int(0))
                    .is_some_and(|s| s.remove(&ExtSetKey::Int(arg_int(1)))),
            ),
            KI::SetAddStr => {
                let added = arg_str(1).is_some_and(|k| {
                    self.ext_collections
                        .set_mut(arg_int(0))
                        .is_some_and(|s| s.insert(ExtSetKey::Str(k)))
                });
                RuntimeValue::Bool(added)
            }
            KI::SetHasStr => {
                let has = arg_str(1).is_some_and(|k| {
                    self.ext_collections
                        .set(arg_int(0))
                        .is_some_and(|s| s.contains(&ExtSetKey::Str(k)))
                });
                RuntimeValue::Bool(has)
            }
            KI::SetRemoveStr => {
                let removed = arg_str(1).is_some_and(|k| {
                    self.ext_collections
                        .set_mut(arg_int(0))
                        .is_some_and(|s| s.remove(&ExtSetKey::Str(k)))
                });
                RuntimeValue::Bool(removed)
            }
            KI::SetLen => RuntimeValue::Int(
                self.ext_collections
                    .set(arg_int(0))
                    .map_or(0, |s| s.len() as i64),
            ),
            KI::SetClear => {
                if let Some(s) = self.ext_collections.set_mut(arg_int(0)) {
                    s.clear();
                }
                RuntimeValue::Void
            }
            KI::SetFree => {
                self.ext_collections.free_set(arg_int(0));
                RuntimeValue::Void
            }
            KI::SetUnion => {
                let mut out = self
                    .ext_collections
                    .set(arg_int(0))
                    .cloned()
                    .unwrap_or_default();
                if let Some(sb) = self.ext_collections.set(arg_int(1)) {
                    out.extend(sb.iter().cloned());
                }
                RuntimeValue::Int(self.ext_collections.alloc_set(out))
            }
            KI::SetIntersect => {
                let out = match (
                    self.ext_collections.set(arg_int(0)),
                    self.ext_collections.set(arg_int(1)),
                ) {
                    (Some(sa), Some(sb)) => sa.intersection(sb).cloned().collect(),
                    _ => Default::default(),
                };
                RuntimeValue::Int(self.ext_collections.alloc_set(out))
            }
            KI::DequeNew => RuntimeValue::Int(self.ext_collections.alloc_deque()),
            KI::DequePushBack => {
                if let Some(dq) = self.ext_collections.deque_mut(arg_int(0)) {
                    dq.push_back(arg_int(1));
                }
                RuntimeValue::Void
            }
            KI::DequePushFront => {
                if let Some(dq) = self.ext_collections.deque_mut(arg_int(0)) {
                    dq.push_front(arg_int(1));
                }
                RuntimeValue::Void
            }
            KI::DequePopBack => RuntimeValue::Int(
                self.ext_collections
                    .deque_mut(arg_int(0))
                    .and_then(|dq| dq.pop_back())
                    .unwrap_or(0),
            ),
            KI::DequePopFront => RuntimeValue::Int(
                self.ext_collections
                    .deque_mut(arg_int(0))
                    .and_then(|dq| dq.pop_front())
                    .unwrap_or(0),
            ),
            KI::DequePeekFront => RuntimeValue::Int(
                self.ext_collections
                    .deque(arg_int(0))
                    .and_then(|dq| dq.front().copied())
                    .unwrap_or(0),
            ),
            KI::DequePeekBack => RuntimeValue::Int(
                self.ext_collections
                    .deque(arg_int(0))
                    .and_then(|dq| dq.back().copied())
                    .unwrap_or(0),
            ),
            KI::DequeGet => {
                let idx = usize::try_from(arg_int(1)).ok();
                RuntimeValue::Int(
                    idx.and_then(|i| {
                        self.ext_collections
                            .deque(arg_int(0))
                            .and_then(|dq| dq.get(i).copied())
                    })
                    .unwrap_or(0),
                )
            }
            KI::DequeLen => RuntimeValue::Int(
                self.ext_collections
                    .deque(arg_int(0))
                    .map_or(0, |dq| dq.len() as i64),
            ),
            KI::DequeClear => {
                if let Some(dq) = self.ext_collections.deque_mut(arg_int(0)) {
                    dq.clear();
                }
                RuntimeValue::Void
            }
            KI::DequeFree => {
                self.ext_collections.free_deque(arg_int(0));
                RuntimeValue::Void
            }
            _ => unreachable!("filtered by caller"),
        })
    }
}
