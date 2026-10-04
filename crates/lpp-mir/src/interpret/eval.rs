use std::collections::BTreeSet;

use lpp_hir::{BinaryOperator, OriginId, UnaryOperator};

use super::*;
use crate::{Constant, InstructionKind, MirPlaceId, Operand, PlaceProjection, Rvalue, Terminator};

impl Interpreter<'_> {
    pub(super) fn execute_entry(
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
    fn execute_function(
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
    fn writeback_captures(
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

    fn eval_rvalue(
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
                // Call arguments move into the callee's frame.
                let mut argument_values = Vec::new();
                for operand in self.program.operands(arguments) {
                    argument_values
                        .push(self.eval_operand_moved(function, block, origin, *operand, frame)?);
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

    fn run_task(
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

    fn eval_builtin(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        builtin: lpp_types::BuiltinId,
        arguments: &[RuntimeValue],
        produced: Option<TypeId>,
    ) -> Result<RuntimeValue, InterpreterError> {
        // v1 exposes every builtin under both its source name and its
        // `lpp_`-prefixed symbol name; both spellings reach this point.
        let name = builtin
            .descriptor()
            .name
            .strip_prefix("lpp_")
            .unwrap_or(builtin.descriptor().name);

        let invalid = |position: usize, actual: ExecutionValueKind| {
            builtin_invalid(function, block, origin, position, actual)
        };
        let invalid_at =
            |position: usize| invalid(position, builtin_argument_kind(arguments, position));
        let int_at = |position: usize| -> Result<i64, InterpreterError> {
            argument_int(arguments, position).ok_or_else(|| invalid_at(position))
        };
        let float_at = |position: usize| -> Result<f64, InterpreterError> {
            Ok(f64::from_bits(
                argument_float_bits(arguments, position).ok_or_else(|| invalid_at(position))?,
            ))
        };
        let bool_at = |position: usize| -> Result<bool, InterpreterError> {
            argument_bool(arguments, position).ok_or_else(|| invalid_at(position))
        };
        let string_at = |position: usize| -> Result<String, InterpreterError> {
            let id = argument_string_id(arguments, position).ok_or_else(|| invalid_at(position))?;
            Ok(heap_string(&self.heap, id).to_owned())
        };
        let list_at = |position: usize| -> Result<HeapId, InterpreterError> {
            argument_list_id(arguments, position).ok_or_else(|| invalid_at(position))
        };
        let clone_at = |position: usize| -> Result<RuntimeValue, InterpreterError> {
            match arguments.get(position) {
                Some(value) => Ok(value.clone()),
                None => Err(invalid(position, ExecutionValueKind::Void)),
            }
        };

        match name {
            // ── Printing: text appended to the deterministic output buffer ──
            "print" => {
                // v1 dispatches `print` on the operand type: string, float,
                // and bool get their own printers; everything else is printed
                // as an integer (characters as their code point).
                let value = clone_at(0)?;
                let text = match value {
                    RuntimeValue::Int(value) => format!("{value}\n"),
                    RuntimeValue::Char(character) => format!("{}\n", character as i64),
                    RuntimeValue::FloatBits(bits) => format!("{:.6}\n", f64::from_bits(bits)),
                    RuntimeValue::Bool(value) => format!("{}\n", u8::from(value)),
                    RuntimeValue::String(id) => {
                        format!("{}\n", heap_string(&self.heap, id))
                    }
                    _ => return Err(invalid(0, value.kind())),
                };
                self.output.push(text);
                // print consumes its value: the reference dies with it.
                if self.arc.is_some() && owns_heap_refs(&value) {
                    self.drop_reference(&value, Some(function), Some(block), Some(origin))?;
                }
                Ok(RuntimeValue::Void)
            }
            "print_str" | "eprint_str" => {
                let text = string_at(0)?;
                self.output.push(format!("{text}\n"));
                Ok(RuntimeValue::Void)
            }
            "print_int" => {
                let value = int_at(0)?;
                self.output.push(format!("{value}\n"));
                Ok(RuntimeValue::Void)
            }
            "print_float" => {
                let value = float_at(0)?;
                self.output.push(format!("{value:.6}\n"));
                Ok(RuntimeValue::Void)
            }
            "print_bool" => {
                let value = bool_at(0)?;
                self.output.push(format!("{}\n", u8::from(value)));
                Ok(RuntimeValue::Void)
            }
            "write_str" => {
                let text = string_at(0)?;
                self.output.push(text);
                // write_str consumes its value: the reference dies with it.
                let value = clone_at(0)?;
                if self.arc.is_some() && owns_heap_refs(&value) {
                    self.drop_reference(&value, Some(function), Some(block), Some(origin))?;
                }
                Ok(RuntimeValue::Void)
            }

            // ── String operations ────────────────────────────────────────────
            "str_concat" => {
                let left = string_at(0)?;
                let right = string_at(1)?;
                builtin_make_string(
                    self,
                    function,
                    block,
                    origin,
                    format!("{left}{right}"),
                    produced,
                )
            }
            "str_len" => {
                let text = string_at(0)?;
                Ok(RuntimeValue::Int(
                    i64::try_from(text.len()).unwrap_or(i64::MAX),
                ))
            }
            "str_contains" => {
                let haystack = string_at(0)?;
                let needle = string_at(1)?;
                Ok(RuntimeValue::Bool(haystack.contains(needle.as_str())))
            }
            "str_starts_with" => {
                let text = string_at(0)?;
                let prefix = string_at(1)?;
                Ok(RuntimeValue::Bool(text.starts_with(prefix.as_str())))
            }
            "str_ends_with" => {
                let text = string_at(0)?;
                let suffix = string_at(1)?;
                Ok(RuntimeValue::Bool(text.ends_with(suffix.as_str())))
            }
            "str_find" => {
                let haystack = string_at(0)?;
                let needle = string_at(1)?;
                let index = haystack
                    .find(needle.as_str())
                    .map(|position| i64::try_from(position).unwrap_or(i64::MAX));
                Ok(RuntimeValue::Int(index.unwrap_or(-1)))
            }
            "str_replace" => {
                let text = string_at(0)?;
                let old = string_at(1)?;
                let new = string_at(2)?;
                let replaced = if old.is_empty() {
                    text
                } else {
                    text.replace(old.as_str(), new.as_str())
                };
                builtin_make_string(self, function, block, origin, replaced, produced)
            }
            "str_trim" => {
                let text = string_at(0)?;
                // v1 trims spaces, tabs, newlines, and carriage returns only.
                let trimmed =
                    text.trim_matches(|character| matches!(character, ' ' | '\t' | '\n' | '\r'));
                builtin_make_string(self, function, block, origin, trimmed.to_owned(), produced)
            }
            "str_to_lower" | "str_lower" => {
                let text = string_at(0)?;
                builtin_make_string(
                    self,
                    function,
                    block,
                    origin,
                    ascii_lowercase(&text),
                    produced,
                )
            }
            "str_to_upper" | "str_upper" => {
                let text = string_at(0)?;
                builtin_make_string(
                    self,
                    function,
                    block,
                    origin,
                    ascii_uppercase(&text),
                    produced,
                )
            }

            // ── Conversions ─────────────────────────────────────────────────
            "int_to_str" => {
                let value = int_at(0)?;
                builtin_make_string(self, function, block, origin, value.to_string(), produced)
            }
            "str_to_int" => Ok(RuntimeValue::Int(parse_i64_decimal(&string_at(0)?))),
            "float_to_str" => {
                let value = float_at(0)?;
                builtin_make_string(
                    self,
                    function,
                    block,
                    origin,
                    format_percent_g(value),
                    produced,
                )
            }
            "bool_to_str" => {
                let value = bool_at(0)?;
                builtin_make_string(self, function, block, origin, value.to_string(), produced)
            }
            "u64_to_str" => {
                let value = int_at(0)?;
                builtin_make_string(
                    self,
                    function,
                    block,
                    origin,
                    (value as u64).to_string(),
                    produced,
                )
            }
            "u64_to_hex" => {
                let value = int_at(0)?;
                builtin_make_string(
                    self,
                    function,
                    block,
                    origin,
                    format!("{:x}", value as u64),
                    produced,
                )
            }
            "str_to_u64" => Ok(RuntimeValue::Int(parse_u64_hex_or_decimal(&string_at(0)?))),

            // ── Integer helpers ─────────────────────────────────────────────
            "abs" => Ok(RuntimeValue::Int(abs_i64(int_at(0)?))),
            "min" => Ok(RuntimeValue::Int(int_at(0)?.min(int_at(1)?))),
            "max" => Ok(RuntimeValue::Int(int_at(0)?.max(int_at(1)?))),
            "min_u" => Ok(RuntimeValue::Int(min_u64(int_at(0)?, int_at(1)?))),
            "max_u" => Ok(RuntimeValue::Int(max_u64(int_at(0)?, int_at(1)?))),
            // Unsigned comparisons return an integer 0/1 (matching v1's
            // `lpp_lt_u` etc.), not a Bool — the corpus uses them C-style
            // (`lt_u(a, b) == 1`), and the ABI `semantic_result` is `i64`.
            "lt_u" => Ok(RuntimeValue::Int(i64::from(
                (int_at(0)? as u64) < (int_at(1)? as u64),
            ))),
            "le_u" => Ok(RuntimeValue::Int(i64::from(
                (int_at(0)? as u64) <= (int_at(1)? as u64),
            ))),
            "gt_u" => Ok(RuntimeValue::Int(i64::from(
                (int_at(0)? as u64) > (int_at(1)? as u64),
            ))),
            "ge_u" => Ok(RuntimeValue::Int(i64::from(
                (int_at(0)? as u64) >= (int_at(1)? as u64),
            ))),
            "shr_u" => {
                let (left, shift) = (int_at(0)?, int_at(1)?);
                Ok(RuntimeValue::Int(if shift < 0 || shift >= 64 {
                    0
                } else {
                    ((left as u64) >> shift) as i64
                }))
            }
            "shl_u" => {
                let (left, shift) = (int_at(0)?, int_at(1)?);
                Ok(RuntimeValue::Int(if shift < 0 || shift >= 64 {
                    0
                } else {
                    ((left as u64) << shift) as i64
                }))
            }
            "div_u" => {
                let (left, right) = (int_at(0)?, int_at(1)?);
                if right == 0 {
                    return Err(builtin_division_by_zero(function, block, origin));
                }
                Ok(RuntimeValue::Int((left as u64 / right as u64) as i64))
            }
            "rem_u" => {
                let (left, right) = (int_at(0)?, int_at(1)?);
                if right == 0 {
                    return Err(builtin_division_by_zero(function, block, origin));
                }
                Ok(RuntimeValue::Int((left as u64 % right as u64) as i64))
            }
            "popcount64" => Ok(RuntimeValue::Int((int_at(0)? as u64).count_ones() as i64)),
            "clz64" => Ok(RuntimeValue::Int((int_at(0)? as u64).leading_zeros() as i64)),
            "ctz64" => Ok(RuntimeValue::Int(
                (int_at(0)? as u64).trailing_zeros() as i64
            )),
            "bswap16" => {
                let value = int_at(0)? as u16;
                Ok(RuntimeValue::Int(value.swap_bytes() as i64))
            }
            "bswap32" => {
                let value = int_at(0)? as u32;
                Ok(RuntimeValue::Int(value.swap_bytes() as i64))
            }
            "bswap64" => {
                let value = int_at(0)? as u64;
                Ok(RuntimeValue::Int(value.swap_bytes() as i64))
            }
            "rotl64" => Ok(RuntimeValue::Int(
                (int_at(0)? as u64).rotate_left((int_at(1)? & 63) as u32) as i64,
            )),
            "rotr64" => Ok(RuntimeValue::Int(
                (int_at(0)? as u64).rotate_right((int_at(1)? & 63) as u32) as i64,
            )),
            "rotl32" => Ok(RuntimeValue::Int(
                (int_at(0)? as u32).rotate_left((int_at(1)? & 31) as u32) as i64,
            )),
            "rotr32" => Ok(RuntimeValue::Int(
                (int_at(0)? as u32).rotate_right((int_at(1)? & 31) as u32) as i64,
            )),
            "trunc_u8" => Ok(RuntimeValue::Int(int_at(0)? as u8 as i64)),
            "trunc_u16" => Ok(RuntimeValue::Int(int_at(0)? as u16 as i64)),
            "trunc_u32" => Ok(RuntimeValue::Int(int_at(0)? as u32 as i64)),
            "trunc_i8" => Ok(RuntimeValue::Int(int_at(0)? as i8 as i64)),
            "trunc_i16" => Ok(RuntimeValue::Int(int_at(0)? as i16 as i64)),
            "trunc_i32" => Ok(RuntimeValue::Int(int_at(0)? as i32 as i64)),
            "add_checked" => {
                let (left, right) = (int_at(0)?, int_at(1)?);
                checked_binary(left.checked_add(right), function, block, origin)
            }
            "sub_checked" => {
                let (left, right) = (int_at(0)?, int_at(1)?);
                checked_binary(left.checked_sub(right), function, block, origin)
            }
            "mul_checked" => {
                let (left, right) = (int_at(0)?, int_at(1)?);
                checked_binary(left.checked_mul(right), function, block, origin)
            }
            "add_wrap" => Ok(RuntimeValue::Int(int_at(0)?.wrapping_add(int_at(1)?))),
            "sub_wrap" => Ok(RuntimeValue::Int(int_at(0)?.wrapping_sub(int_at(1)?))),
            "mul_wrap" => Ok(RuntimeValue::Int(int_at(0)?.wrapping_mul(int_at(1)?))),

            // ── Float math ──────────────────────────────────────────────────
            "floor" => Ok(RuntimeValue::FloatBits(float_at(0)?.floor().to_bits())),
            "ceil" => Ok(RuntimeValue::FloatBits(float_at(0)?.ceil().to_bits())),
            "pow" => Ok(RuntimeValue::FloatBits(
                float_at(0)?.powf(float_at(1)?).to_bits(),
            )),
            "sqrt" => Ok(RuntimeValue::FloatBits(float_at(0)?.sqrt().to_bits())),
            "fmod" => {
                let left = float_at(0)?;
                let right = float_at(1)?;
                // IEEE remainder, same sign as the dividend (C `fmod`).
                let quotient = (left / right).floor();
                Ok(RuntimeValue::FloatBits((left - quotient * right).to_bits()))
            }

            // ── Structural list operations ──────────────────────────────────
            "list_new" => {
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::List(vec![]),
                    produced,
                )?;
                Ok(RuntimeValue::List(id))
            }
            "list_push" => {
                let id = list_at(0)?;
                let value = clone_at(1)?;
                heap_list_mut(&mut self.heap, id).push(value);
                Ok(RuntimeValue::Void)
            }
            "list_get" => {
                let id = list_at(0)?;
                let index = int_at(1)?;
                let entries = heap_list(&self.heap, id);
                let usize_index = usize::try_from(index)
                    .ok()
                    .filter(|candidate| *candidate < entries.len());
                let Some(usize_index) = usize_index else {
                    return Err(builtin_index_out_of_bounds(
                        function,
                        block,
                        origin,
                        index,
                        entries.len(),
                    ));
                };
                let value = entries[usize_index].clone();
                // The list keeps its element; the caller gains one.
                if self.arc.is_some() && owns_heap_refs(&value) {
                    self.retain_owned(function, Some(block), Some(origin), &value)?;
                }
                Ok(value)
            }
            "list_set" => {
                let id = list_at(0)?;
                let index = int_at(1)?;
                let value = clone_at(2)?;
                let entries = heap_list_mut(&mut self.heap, id);
                let usize_index = usize::try_from(index)
                    .ok()
                    .filter(|candidate| *candidate < entries.len());
                let Some(usize_index) = usize_index else {
                    return Err(builtin_index_out_of_bounds(
                        function,
                        block,
                        origin,
                        index,
                        entries.len(),
                    ));
                };
                let old = entries[usize_index].clone();
                entries[usize_index] = value;
                // The replaced element's reference is dropped.
                if self.arc.is_some() && owns_heap_refs(&old) {
                    self.drop_reference(&old, Some(function), Some(block), Some(origin))?;
                }
                Ok(RuntimeValue::Void)
            }
            "list_len" => {
                let id = list_at(0)?;
                let len = heap_list(&self.heap, id).len();
                Ok(RuntimeValue::Int(i64::try_from(len).unwrap_or(i64::MAX)))
            }

            _ => Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::UnsupportedBuiltin,
            )),
        }
    }

    fn eval_operands(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        operands: crate::ListRange<Operand>,
        frame: &Frame,
    ) -> Result<Vec<RuntimeValue>, InterpreterError> {
        self.program
            .operands(operands)
            .iter()
            .map(|operand| self.eval_operand(function, block, origin, *operand, frame))
            .collect()
    }

    fn eval_operand(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        operand: Operand,
        frame: &Frame,
    ) -> Result<RuntimeValue, InterpreterError> {
        match operand {
            Operand::Copy(local) => {
                let Some(value) = frame.get(local) else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::UninitializedLocal(local),
                    ));
                };
                self.reserve_runtime_clone(function, Some(block), origin, value)?;
                Ok(value.clone())
            }
            Operand::Function(function) => Ok(RuntimeValue::Function(function)),
            Operand::Constant(constant) => match constant {
                Constant::Integer(value) => Ok(RuntimeValue::Int(value)),
                Constant::FloatBits(value) => Ok(RuntimeValue::FloatBits(value)),
                Constant::Bool(value) => Ok(RuntimeValue::Bool(value)),
                Constant::Character {
                    origin: _,
                    character,
                } => Ok(RuntimeValue::Char(character)),
                // Literal string constants are interned per program ID, so
                // every reference to the same literal aliases one node.
                // Under ARC the cache entry holds one reference for the
                // run; every reader gains its own.
                Constant::String { origin: _, string } => {
                    let cached = self.string_cache.get(&string).copied();
                    if let Some(id) = cached {
                        if self.arc.is_some() {
                            self.retain_value(
                                function,
                                Some(block),
                                Some(origin),
                                &RuntimeValue::String(id),
                            )?;
                        }
                        return Ok(RuntimeValue::String(id));
                    }
                    let text = self
                        .program
                        .string(string)
                        .cloned()
                        .expect("verified MIR retains every string");
                    let id = self.allocate_heap(
                        function,
                        Some(block),
                        origin,
                        HeapNode::String(text),
                        None,
                    )?;
                    self.mark_ephemeral(id);
                    self.string_cache.insert(string, id);
                    if self.arc.is_some() {
                        self.retain_value(
                            function,
                            Some(block),
                            Some(origin),
                            &RuntimeValue::String(id),
                        )?;
                    }
                    Ok(RuntimeValue::String(id))
                }
            },
        }
    }

    fn eval_unary(
        &self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        operator: UnaryOperator,
        value: RuntimeValue,
    ) -> Result<RuntimeValue, InterpreterError> {
        match (operator, value) {
            (UnaryOperator::Not, RuntimeValue::Bool(value)) => Ok(RuntimeValue::Bool(!value)),
            (UnaryOperator::Not, RuntimeValue::Int(value)) => Ok(RuntimeValue::Bool(value == 0)),
            (UnaryOperator::Negate, RuntimeValue::Int(value)) => {
                Ok(RuntimeValue::Int(value.wrapping_neg()))
            }
            (UnaryOperator::Negate, RuntimeValue::FloatBits(value)) => {
                Ok(RuntimeValue::FloatBits((-f64::from_bits(value)).to_bits()))
            }
            _ => Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::InvalidUnaryOperands,
            )),
        }
    }

    fn eval_binary(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        operator: BinaryOperator,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<RuntimeValue, InterpreterError> {
        if matches!(
            (&left, &right, operator),
            (
                RuntimeValue::Int(_),
                RuntimeValue::Int(0),
                BinaryOperator::Divide | BinaryOperator::Modulo,
            )
        ) {
            return Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::DivisionByZero,
            ));
        }
        let result = match (left, right) {
            (RuntimeValue::Int(left), RuntimeValue::Int(right)) => {
                self.eval_integer(operator, left, right)
            }
            (RuntimeValue::FloatBits(left), RuntimeValue::FloatBits(right)) => {
                self.eval_float(operator, f64::from_bits(left), f64::from_bits(right))
            }
            (RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => match operator {
                BinaryOperator::LogicalAnd => Some(RuntimeValue::Bool(left && right)),
                BinaryOperator::LogicalOr => Some(RuntimeValue::Bool(left || right)),
                BinaryOperator::Equal => Some(RuntimeValue::Bool(left == right)),
                BinaryOperator::NotEqual => Some(RuntimeValue::Bool(left != right)),
                _ => None,
            },
            (RuntimeValue::Char(left), RuntimeValue::Char(right)) => {
                self.eval_char(operator, left, right)
            }
            (RuntimeValue::String(left), RuntimeValue::String(right)) => {
                let (HeapNode::String(left), HeapNode::String(right)) =
                    (&self.heap[left.0 as usize], &self.heap[right.0 as usize])
                else {
                    unreachable!("string handles reference string heap nodes")
                };
                match operator {
                    BinaryOperator::Add => {
                        let id = self.allocate_heap(
                            function,
                            Some(block),
                            origin,
                            HeapNode::String(format!("{left}{right}")),
                            None,
                        )?;
                        Some(RuntimeValue::String(id))
                    }
                    BinaryOperator::Equal => Some(RuntimeValue::Bool(left == right)),
                    BinaryOperator::NotEqual => Some(RuntimeValue::Bool(left != right)),
                    _ => None,
                }
            }
            // String `+` with a scalar right operand: stringify the scalar and
            // concatenate, mirroring codegen's *_to_str + str_concat path.
            (RuntimeValue::String(left_id), right)
                if operator == BinaryOperator::Add
                    && matches!(
                        right,
                        RuntimeValue::Int(_) | RuntimeValue::FloatBits(_) | RuntimeValue::Bool(_)
                    ) =>
            {
                let HeapNode::String(left_text) = &self.heap[left_id.0 as usize] else {
                    unreachable!("string handle references a string node")
                };
                let mut combined = left_text.clone();
                match right {
                    RuntimeValue::Int(value) => combined.push_str(&value.to_string()),
                    RuntimeValue::Bool(value) => {
                        combined.push_str(if value { "true" } else { "false" })
                    }
                    // NOTE: codegen stringifies floats with C `%g`
                    // (lpp_float_to_str); this matches for typical values but can
                    // differ for very large/small magnitudes.
                    RuntimeValue::FloatBits(bits) => {
                        combined.push_str(&f64::from_bits(bits).to_string())
                    }
                    _ => unreachable!("guarded to scalar right operands"),
                }
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::String(combined),
                    None,
                )?;
                Some(RuntimeValue::String(id))
            }
            (left, right) => match operator {
                BinaryOperator::Equal => {
                    Some(RuntimeValue::Bool(self.runtime_equal(&left, &right)))
                }
                BinaryOperator::NotEqual => {
                    Some(RuntimeValue::Bool(!self.runtime_equal(&left, &right)))
                }
                _ => None,
            },
        };
        result.ok_or_else(|| {
            self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::InvalidBinaryOperands,
            )
        })
    }

    fn runtime_equal(&self, left: &RuntimeValue, right: &RuntimeValue) -> bool {
        let mut pending = vec![(left, right)];
        let mut visited = std::collections::BTreeSet::new();
        while let Some((left, right)) = pending.pop() {
            match (left, right) {
                (RuntimeValue::Void, RuntimeValue::Void) => {}
                (RuntimeValue::Bool(left), RuntimeValue::Bool(right)) if left == right => {}
                (RuntimeValue::Int(left), RuntimeValue::Int(right)) if left == right => {}
                (RuntimeValue::FloatBits(left), RuntimeValue::FloatBits(right))
                    if left == right => {}
                (RuntimeValue::Char(left), RuntimeValue::Char(right)) if left == right => {}
                (RuntimeValue::String(left), RuntimeValue::String(right)) => {
                    let (HeapNode::String(left), HeapNode::String(right)) =
                        (&self.heap[left.0 as usize], &self.heap[right.0 as usize])
                    else {
                        unreachable!("string handles reference string heap nodes")
                    };
                    if left != right {
                        return false;
                    }
                }
                (RuntimeValue::Function(left), RuntimeValue::Function(right)) if left == right => {}
                (RuntimeValue::Closure(left), RuntimeValue::Closure(right)) => {
                    if left == right {
                        continue;
                    }
                    let HeapNode::Closure {
                        function: left_function,
                        captures: left_captures,
                    } = &self.heap[left.0 as usize]
                    else {
                        unreachable!("closure handles reference closure heap nodes")
                    };
                    let HeapNode::Closure {
                        function: right_function,
                        captures: right_captures,
                    } = &self.heap[right.0 as usize]
                    else {
                        unreachable!("closure handles reference closure heap nodes")
                    };
                    if left_function != right_function
                        || left_captures.len() != right_captures.len()
                    {
                        return false;
                    }
                    pending.extend(left_captures.iter().zip(right_captures));
                }
                (RuntimeValue::Tuple(left), RuntimeValue::Tuple(right)) => {
                    if left.len() != right.len() {
                        return false;
                    }
                    pending.extend(left.iter().zip(right));
                }
                (RuntimeValue::List(left), RuntimeValue::List(right)) => {
                    if left == right || !visited.insert((*left, *right)) {
                        continue;
                    }
                    let (HeapNode::List(left), HeapNode::List(right)) =
                        (&self.heap[left.0 as usize], &self.heap[right.0 as usize])
                    else {
                        return false;
                    };
                    if left.len() != right.len() {
                        return false;
                    }
                    pending.extend(left.iter().zip(right));
                }
                (RuntimeValue::Nominal(left), RuntimeValue::Nominal(right)) => {
                    if left == right || !visited.insert((*left, *right)) {
                        continue;
                    }
                    let (
                        HeapNode::Nominal {
                            aggregate: left_aggregate,
                            variant: left_variant,
                            fields: left_fields,
                        },
                        HeapNode::Nominal {
                            aggregate: right_aggregate,
                            variant: right_variant,
                            fields: right_fields,
                        },
                    ) = (&self.heap[left.0 as usize], &self.heap[right.0 as usize])
                    else {
                        return false;
                    };
                    if left_aggregate != right_aggregate
                        || left_variant != right_variant
                        || left_fields.len() != right_fields.len()
                    {
                        return false;
                    }
                    pending.extend(left_fields.iter().zip(right_fields));
                }
                _ => return false,
            }
        }
        true
    }

    fn eval_integer(
        &self,
        operator: BinaryOperator,
        left: i64,
        right: i64,
    ) -> Option<RuntimeValue> {
        match operator {
            BinaryOperator::Add => Some(RuntimeValue::Int(left.wrapping_add(right))),
            BinaryOperator::Subtract => Some(RuntimeValue::Int(left.wrapping_sub(right))),
            BinaryOperator::Multiply => Some(RuntimeValue::Int(left.wrapping_mul(right))),
            BinaryOperator::Divide if right != 0 => {
                Some(RuntimeValue::Int(left.wrapping_div(right)))
            }
            BinaryOperator::Modulo if right != 0 => {
                Some(RuntimeValue::Int(left.wrapping_rem(right)))
            }
            BinaryOperator::BitAnd => Some(RuntimeValue::Int(left & right)),
            BinaryOperator::BitOr => Some(RuntimeValue::Int(left | right)),
            BinaryOperator::BitXor => Some(RuntimeValue::Int(left ^ right)),
            BinaryOperator::ShiftLeft => Some(RuntimeValue::Int(left.wrapping_shl(right as u32))),
            BinaryOperator::ShiftRight => Some(RuntimeValue::Int(left.wrapping_shr(right as u32))),
            BinaryOperator::Equal => Some(RuntimeValue::Bool(left == right)),
            BinaryOperator::NotEqual => Some(RuntimeValue::Bool(left != right)),
            BinaryOperator::Less => Some(RuntimeValue::Bool(left < right)),
            BinaryOperator::Greater => Some(RuntimeValue::Bool(left > right)),
            BinaryOperator::LessEqual => Some(RuntimeValue::Bool(left <= right)),
            BinaryOperator::GreaterEqual => Some(RuntimeValue::Bool(left >= right)),
            _ => None,
        }
    }

    fn eval_char(&self, operator: BinaryOperator, left: char, right: char) -> Option<RuntimeValue> {
        match operator {
            BinaryOperator::Equal => Some(RuntimeValue::Bool(left == right)),
            BinaryOperator::NotEqual => Some(RuntimeValue::Bool(left != right)),
            BinaryOperator::Less => Some(RuntimeValue::Bool(left < right)),
            BinaryOperator::Greater => Some(RuntimeValue::Bool(left > right)),
            BinaryOperator::LessEqual => Some(RuntimeValue::Bool(left <= right)),
            BinaryOperator::GreaterEqual => Some(RuntimeValue::Bool(left >= right)),
            _ => None,
        }
    }

    fn eval_float(&self, operator: BinaryOperator, left: f64, right: f64) -> Option<RuntimeValue> {
        match operator {
            BinaryOperator::Add => Some(RuntimeValue::FloatBits((left + right).to_bits())),
            BinaryOperator::Subtract => Some(RuntimeValue::FloatBits((left - right).to_bits())),
            BinaryOperator::Multiply => Some(RuntimeValue::FloatBits((left * right).to_bits())),
            BinaryOperator::Divide => Some(RuntimeValue::FloatBits((left / right).to_bits())),
            BinaryOperator::Modulo => Some(RuntimeValue::FloatBits((left % right).to_bits())),
            BinaryOperator::Equal => Some(RuntimeValue::Bool(left == right)),
            BinaryOperator::NotEqual => Some(RuntimeValue::Bool(left != right)),
            BinaryOperator::Less => Some(RuntimeValue::Bool(left < right)),
            BinaryOperator::Greater => Some(RuntimeValue::Bool(left > right)),
            BinaryOperator::LessEqual => Some(RuntimeValue::Bool(left <= right)),
            BinaryOperator::GreaterEqual => Some(RuntimeValue::Bool(left >= right)),
            _ => None,
        }
    }

    fn import_value(
        &mut self,
        function: MirFunctionId,
        origin: OriginId,
        value: &ExecutionValue,
        produced: Option<TypeId>,
    ) -> Result<RuntimeValue, InterpreterError> {
        Ok(match value {
            ExecutionValue::Void => RuntimeValue::Void,
            ExecutionValue::Bool(value) => RuntimeValue::Bool(*value),
            ExecutionValue::Int(value) => RuntimeValue::Int(*value),
            ExecutionValue::FloatBits(value) => RuntimeValue::FloatBits(*value),
            ExecutionValue::String(value) => RuntimeValue::String(self.allocate_heap(
                function,
                None,
                origin,
                HeapNode::String(value.clone()),
                produced,
            )?),
            ExecutionValue::Char(value) => RuntimeValue::Char(*value),
            ExecutionValue::Closure {
                function: target,
                captures,
            } => {
                let imported_function = self.program.function(*target).copied();
                let parameters = imported_function
                    .map(|function| self.program.function_parameters(&function).to_vec())
                    .unwrap_or_default();
                let mut imported = Vec::with_capacity(captures.len());
                for (position, value) in captures.iter().enumerate() {
                    let child_ty = parameters
                        .get(position)
                        .copied()
                        .and_then(|parameter| self.program.local(parameter).map(|local| local.ty));
                    imported.push(self.import_value(function, origin, value, child_ty)?);
                }
                let produced = imported_function.map(|function| function.ty);
                RuntimeValue::Closure(self.allocate_heap(
                    function,
                    None,
                    origin,
                    HeapNode::Closure {
                        function: *target,
                        captures: imported,
                    },
                    produced,
                )?)
            }
            ExecutionValue::Task {
                function: target,
                arguments,
            } => {
                let mut imported = Vec::with_capacity(arguments.len());
                for value in arguments {
                    imported.push(self.import_value(function, origin, value, None)?);
                }
                let id = self.allocate_heap(
                    function,
                    None,
                    origin,
                    HeapNode::Task {
                        function: *target,
                        arguments: imported,
                        closure: None,
                        result: None,
                    },
                    None,
                )?;
                self.mark_ephemeral(id);
                RuntimeValue::Task(id)
            }
            ExecutionValue::Function(value) => RuntimeValue::Function(*value),
            ExecutionValue::Tuple(values) => {
                let mut imported = Vec::with_capacity(values.len());
                for value in values {
                    imported.push(self.import_value(function, origin, value, None)?);
                }
                RuntimeValue::Tuple(imported)
            }
            ExecutionValue::List(values) => {
                let element = produced.and_then(|ty| match self.types.kind(ty) {
                    TypeKind::List(element) => Some(element),
                    _ => None,
                });
                let mut imported = Vec::with_capacity(values.len());
                for value in values {
                    imported.push(self.import_value(function, origin, value, element)?);
                }
                RuntimeValue::List(self.allocate_heap(
                    function,
                    None,
                    origin,
                    HeapNode::List(imported),
                    produced,
                )?)
            }
            ExecutionValue::Nominal {
                aggregate,
                variant,
                fields,
            } => {
                let mut imported = Vec::with_capacity(fields.len());
                for value in fields {
                    imported.push(self.import_value(function, origin, value, None)?);
                }
                let produced = self
                    .program
                    .aggregate(*aggregate)
                    .map(|aggregate| aggregate.ty);
                RuntimeValue::Nominal(self.allocate_heap(
                    function,
                    None,
                    origin,
                    HeapNode::Nominal {
                        aggregate: *aggregate,
                        variant: *variant,
                        fields: imported,
                    },
                    produced,
                )?)
            }
        })
    }

    fn export_value(
        &self,
        function: MirFunctionId,
        origin: OriginId,
        value: &RuntimeValue,
    ) -> Result<ExecutionValue, InterpreterError> {
        Ok(match value {
            RuntimeValue::Void => ExecutionValue::Void,
            RuntimeValue::Bool(value) => ExecutionValue::Bool(*value),
            RuntimeValue::Int(value) => ExecutionValue::Int(*value),
            RuntimeValue::FloatBits(value) => ExecutionValue::FloatBits(*value),
            RuntimeValue::String(id) => {
                let HeapNode::String(text) = &self.heap[id.0 as usize] else {
                    unreachable!("string handles reference string heap nodes")
                };
                ExecutionValue::String(text.clone())
            }
            RuntimeValue::Char(value) => ExecutionValue::Char(*value),
            RuntimeValue::Closure(id) => {
                let HeapNode::Closure {
                    function: target,
                    captures,
                } = &self.heap[id.0 as usize]
                else {
                    unreachable!("closure handles reference closure heap nodes")
                };
                ExecutionValue::Closure {
                    function: *target,
                    captures: captures
                        .iter()
                        .map(|value| self.export_value(function, origin, value))
                        .collect::<Result<_, _>>()?,
                }
            }
            RuntimeValue::Task(id) => {
                let HeapNode::Task {
                    function: target,
                    arguments,
                    ..
                } = &self.heap[id.0 as usize]
                else {
                    unreachable!("task handles reference task heap nodes")
                };
                ExecutionValue::Task {
                    function: *target,
                    arguments: arguments
                        .iter()
                        .map(|value| self.export_value(function, origin, value))
                        .collect::<Result<_, _>>()?,
                }
            }
            RuntimeValue::Function(value) => ExecutionValue::Function(*value),
            RuntimeValue::Tuple(values) => ExecutionValue::Tuple(
                values
                    .iter()
                    .map(|value| self.export_value(function, origin, value))
                    .collect::<Result<_, _>>()?,
            ),
            RuntimeValue::List(id) => {
                let HeapNode::List(values) = &self.heap[id.0 as usize] else {
                    return Err(self.error(
                        Some(function),
                        None,
                        Some(origin),
                        InterpreterErrorKind::InvalidListProjection,
                    ));
                };
                ExecutionValue::List(
                    values
                        .iter()
                        .map(|value| self.export_value(function, origin, value))
                        .collect::<Result<_, _>>()?,
                )
            }
            RuntimeValue::Nominal(id) => {
                let HeapNode::Nominal {
                    aggregate,
                    variant,
                    fields,
                } = &self.heap[id.0 as usize]
                else {
                    return Err(self.error(
                        Some(function),
                        None,
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    ));
                };
                ExecutionValue::Nominal {
                    aggregate: *aggregate,
                    variant: *variant,
                    fields: fields
                        .iter()
                        .map(|value| self.export_value(function, origin, value))
                        .collect::<Result<_, _>>()?,
                }
            }
        })
    }

    fn allocate_heap(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        origin: OriginId,
        node: HeapNode,
        produced: Option<TypeId>,
    ) -> Result<HeapId, InterpreterError> {
        if self.heap.len() >= self.limits.max_heap_nodes {
            return Err(self.error(
                Some(function),
                block,
                Some(origin),
                InterpreterErrorKind::LimitExceeded(InterpreterLimit::HeapNodes),
            ));
        }
        let id = u32::try_from(self.heap.len()).map_err(|_| {
            self.error(
                Some(function),
                block,
                Some(origin),
                InterpreterErrorKind::LimitExceeded(InterpreterLimit::HeapNodes),
            )
        })?;
        if let Some(arc) = self.arc.as_mut() {
            arc.refcounts.push(1);
            let pin = match produced {
                Some(ty) => NodePin::Of(ty),
                None => NodePin::Unknown,
            };
            arc.node_types.push(pin);
        }
        self.heap.push(node);
        Ok(HeapId(id))
    }

    /// The children a deallocation must release, in declaration order.
    fn children_of(&self, id: HeapId) -> Vec<RuntimeValue> {
        let mut children: Vec<RuntimeValue> = Vec::new();
        match &self.heap[id.0 as usize] {
            HeapNode::List(elements) => children.extend(elements.iter().cloned()),
            HeapNode::Nominal { fields, .. } => children.extend(fields.iter().cloned()),
            HeapNode::String(_) => {}
            HeapNode::Closure { captures, .. } => children.extend(captures.iter().cloned()),
            HeapNode::Task {
                arguments,
                result,
                closure,
                ..
            } => {
                children.extend(arguments.iter().cloned());
                if let Some(result) = result {
                    children.push(result.clone());
                }
                if let Some(closure) = closure {
                    children.push(RuntimeValue::Closure(*closure));
                }
            }
        }
        children
    }

    /// Whether the node's own type is a 4D cycle member (pinned). Nodes
    /// without a recorded type are treated conservatively as pinned:
    /// they are reported, never reported as leaks.
    pub(super) fn node_is_pinned(&self, id: HeapId) -> bool {
        let Some(arc) = self.arc.as_ref() else {
            return false;
        };
        match arc.node_types.get(id.0 as usize) {
            None | Some(NodePin::Unknown) => true,
            Some(NodePin::Ephemeral) => false,
            Some(NodePin::Of(ty)) => arc.pinned.contains(ty),
        }
    }

    /// Classify a task node's pin: a task node can join a 4D cycle only
    /// through its contents, so it pins through the result type or the
    /// spawned closure's function type, and is ephemeral otherwise
    /// (detached nodes must be able to die with their last owner).
    fn mark_task_node(&mut self, id: HeapId, target: MirFunctionId) {
        if self.arc.is_none() {
            return;
        }
        let Some(function) = self.program.function(target).copied() else {
            return;
        };
        let closure_ty = match &self.heap[id.0 as usize] {
            HeapNode::Task {
                closure: Some(closure),
                ..
            } => self
                .program
                .function(function_id_of_closure(self, *closure))
                .map(|f| f.ty),
            _ => None,
        };
        let pin = if is_heap_type(self.types, function.return_type) {
            NodePin::Of(function.return_type)
        } else if let Some(ty) = closure_ty {
            NodePin::Of(ty)
        } else {
            NodePin::Ephemeral
        };
        let index = id.0 as usize;
        self.arc.as_mut().unwrap().node_types[index] = pin;
    }

    /// Mark a node that cannot be a cycle member (string literals).
    fn mark_ephemeral(&mut self, id: HeapId) {
        if let Some(arc) = self.arc.as_mut() {
            arc.node_types[id.0 as usize] = NodePin::Ephemeral;
        }
    }

    /// Gain one reference for a heap value (the borrow half of the call
    /// contract). A no-op for primitives and on the legacy entries.
    fn retain_value(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
        value: &RuntimeValue,
    ) -> Result<(), InterpreterError> {
        if self.arc.is_none() || !is_heap_value(value) {
            return Ok(());
        }
        let index = heap_id_of(value).0 as usize;
        let Some(count) = self.arc.as_mut().unwrap().refcounts.get_mut(index) else {
            return Err(self.error(
                Some(function),
                block,
                origin,
                InterpreterErrorKind::OwnershipUnderflow,
            ));
        };
        *count = count.saturating_add(1);
        let arc = self.arc.as_mut().unwrap();
        arc.retains = arc.retains.saturating_add(1);
        Ok(())
    }

    /// Like `retain_value`, but looks through inline tuples: every heap
    /// element of a copied tuple gains a reference.
    fn retain_owned(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
        value: &RuntimeValue,
    ) -> Result<(), InterpreterError> {
        if let RuntimeValue::Tuple(elements) = value {
            for element in elements {
                self.retain_owned(function, block, origin, element)?;
            }
            return Ok(());
        }
        self.retain_value(function, block, origin, value)
    }

    /// Drop one reference for a value that owns heap references (looking
    /// through inline tuples); when a node's count reaches zero it is
    /// deallocated and its children released, depth-first in declaration
    /// order (iteratively). Pinned nodes absorb the release. A release
    /// with no live reference is an underflow.
    fn drop_reference(
        &mut self,
        value: &RuntimeValue,
        function: Option<MirFunctionId>,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
    ) -> Result<(), InterpreterError> {
        if self.arc.is_none() || !owns_heap_refs(value) {
            return Ok(());
        }
        let mut pending: Vec<RuntimeValue> = vec![value.clone()];
        while let Some(current) = pending.pop() {
            if let RuntimeValue::Tuple(elements) = &current {
                for element in elements.iter().rev() {
                    pending.push(element.clone());
                }
                continue;
            }
            // Primitive children own no references: skip them.
            if !is_heap_value(&current) {
                continue;
            }
            let id = heap_id_of(&current);
            let index = id.0 as usize;
            let count = *self
                .arc
                .as_ref()
                .unwrap()
                .refcounts
                .get(index)
                .ok_or_else(|| {
                    self.error(
                        function,
                        block,
                        origin,
                        InterpreterErrorKind::OwnershipUnderflow,
                    )
                })?;
            if count == 0 {
                return Err(self.error(
                    function,
                    block,
                    origin,
                    InterpreterErrorKind::OwnershipUnderflow,
                ));
            }
            if self.node_is_pinned(id) {
                continue;
            }
            let children = self.children_of(id);
            let arc = self.arc.as_mut().unwrap();
            let next = count - 1;
            arc.refcounts[index] = next;
            arc.releases = arc.releases.saturating_add(1);
            if next > 0 {
                continue;
            }
            arc.frees = arc.frees.saturating_add(1);
            for child in children.into_iter().rev() {
                pending.push(child);
            }
        }
        Ok(())
    }

    /// End-of-execution balance proof for `execute_mir_arc`: the string
    /// cache holds one reference per interned literal and releases them,
    /// then every non-pinned heap node must be dead. A node held —
    /// transitively — by a live pinned node is part of that 4D-approved
    /// cycle's retained set: it survives the program together with the
    /// cycle and is not an accidental leak.
    pub(super) fn finish_arc(&mut self) -> Result<(), InterpreterError> {
        if self.arc.is_none() {
            return Ok(());
        }
        let cached: Vec<HeapId> = self.string_cache.values().copied().collect();
        for id in cached {
            self.drop_reference(&RuntimeValue::String(id), None, None, None)?;
        }
        let arc = self.arc.as_ref().unwrap();
        let mut held: BTreeSet<usize> = BTreeSet::new();
        let mut stack: Vec<usize> = arc
            .refcounts
            .iter()
            .enumerate()
            .filter(|(index, count)| **count > 0 && self.node_is_pinned(HeapId(*index as u32)))
            .map(|(index, _)| index)
            .collect();
        while let Some(index) = stack.pop() {
            if !held.insert(index) {
                continue;
            }
            for child in self.children_of(HeapId(index as u32)) {
                if is_heap_value(&child) {
                    stack.push(heap_id_of(&child).0 as usize);
                }
            }
        }
        let leaked = arc
            .refcounts
            .iter()
            .enumerate()
            .filter(|(index, count)| {
                **count > 0 && !self.node_is_pinned(HeapId(*index as u32)) && !held.contains(index)
            })
            .count();
        if leaked > 0 {
            return Err(InterpreterError {
                function: None,
                block: None,
                origin: None,
                kind: InterpreterErrorKind::OwnershipLeak { nodes: leaked },
            });
        }
        Ok(())
    }

    /// Evaluate a transfer operand that consumes the source slot (the
    /// move half of the call contract). On the legacy entries this is a
    /// plain read.
    /// A read-only evaluation: a string literal read in a position
    /// that does not store gains a reader reference, which the read
    /// releases (the v1 expression end). Positions that store —
    /// assignments, place stores, moved arguments, tuple and list
    /// elements — take ownership instead and keep the reference.
    fn eval_operand_borrowed(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        operand: Operand,
        frame: &Frame,
    ) -> Result<RuntimeValue, InterpreterError> {
        let value = self.eval_operand(function, block, origin, operand, frame)?;
        if self.arc.is_some() && matches!(operand, Operand::Constant(Constant::String { .. })) {
            self.drop_reference(&value, Some(function), Some(block), Some(origin))?;
        }
        Ok(value)
    }

    fn eval_operand_moved(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        operand: Operand,
        frame: &mut Frame,
    ) -> Result<RuntimeValue, InterpreterError> {
        let value = self.eval_operand(function, block, origin, operand, frame)?;
        if self.arc.is_some() {
            if let Operand::Copy(local) = operand {
                if owns_heap_refs(&value) {
                    frame.take(local);
                }
            }
        }
        Ok(value)
    }

    /// Whether a builtin moves (consumes) its positional value argument.
    /// The single source of truth for the builtin half of the call
    /// contract; list receivers, string arguments, and index operands
    /// never move.
    fn builtin_moves_value(&self, builtin: lpp_types::BuiltinId, position: usize) -> bool {
        let name = builtin
            .descriptor()
            .name
            .strip_prefix("lpp_")
            .unwrap_or(builtin.descriptor().name);
        match (name, position) {
            ("list_push", 1) | ("list_set", 2) => true,
            ("print", 0) | ("write_str", 0) => true,
            _ => false,
        }
    }

    fn load_place(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        place: MirPlaceId,
        frame: &Frame,
    ) -> Result<RuntimeValue, InterpreterError> {
        let place = self
            .program
            .place(place)
            .expect("verified MIR retains every place");
        let Some(mut current) = frame.get(place.root).cloned() else {
            return Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::UninitializedLocal(place.root),
            ));
        };
        let projections = self.program.place_projections(place).to_vec();
        for projection in projections {
            current = self.project_value(function, block, origin, &current, projection, frame)?;
        }
        self.reserve_runtime_clone(function, Some(block), origin, &current)?;
        Ok(current)
    }

    fn store_place(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        place: MirPlaceId,
        value: RuntimeValue,
        frame: &mut Frame,
    ) -> Result<(), InterpreterError> {
        let place = self
            .program
            .place(place)
            .expect("verified MIR retains every place");
        let projections = self.program.place_projections(place).to_vec();
        if projections.is_empty() {
            // Cell death on reassignment: the old value's reference is
            // dropped before the new one lands.
            if self.arc.is_some() {
                if let Some(old) = frame.get(place.root).cloned() {
                    self.drop_reference(&old, Some(function), Some(block), Some(origin))?;
                }
            }
            frame.set(place.root, value);
            return Ok(());
        }
        let Some(mut current) = frame.get(place.root).cloned() else {
            return Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::UninitializedLocal(place.root),
            ));
        };
        for projection in &projections[..projections.len() - 1] {
            current = self.project_value(function, block, origin, &current, *projection, frame)?;
        }
        match *projections.last().expect("non-empty projections") {
            PlaceProjection::Downcast(_) => {
                return Err(self.error(
                    Some(function),
                    Some(block),
                    Some(origin),
                    InterpreterErrorKind::InvalidAggregateProjection,
                ));
            }
            PlaceProjection::Field(field) => {
                let RuntimeValue::Nominal(id) = current else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    ));
                };
                let position = self.nominal_field_position(id, field).ok_or_else(|| {
                    self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    )
                })?;
                let HeapNode::Nominal { fields, .. } = &mut self.heap[id.0 as usize] else {
                    unreachable!("nominal handles reference nominal heap nodes")
                };
                let old = fields[position].clone();
                fields[position] = value;
                // The replaced field's reference is dropped.
                if self.arc.is_some() && owns_heap_refs(&old) {
                    self.drop_reference(&old, Some(function), Some(block), Some(origin))?;
                }
            }
            PlaceProjection::ListIndex(index) => {
                let RuntimeValue::List(id) = current else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidListProjection,
                    ));
                };
                let index = self.eval_index(function, block, origin, index, frame)?;
                let HeapNode::List(values) = &self.heap[id.0 as usize] else {
                    unreachable!("list handles reference list heap nodes")
                };
                let len = values.len();
                if index < 0 || index as usize >= len {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::IndexOutOfBounds { index, len },
                    ));
                }
                let old = values[index as usize].clone();
                let HeapNode::List(values) = &mut self.heap[id.0 as usize] else {
                    unreachable!("list handles reference list heap nodes")
                };
                values[index as usize] = value;
                // The replaced element's reference is dropped.
                if self.arc.is_some() && owns_heap_refs(&old) {
                    self.drop_reference(&old, Some(function), Some(block), Some(origin))?;
                }
            }
            PlaceProjection::TupleField(_) => {
                return Err(self.error(
                    Some(function),
                    Some(block),
                    Some(origin),
                    InterpreterErrorKind::InvalidAggregateProjection,
                ));
            }
        }
        Ok(())
    }

    fn project_value(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        value: &RuntimeValue,
        projection: PlaceProjection,
        frame: &Frame,
    ) -> Result<RuntimeValue, InterpreterError> {
        match projection {
            PlaceProjection::Downcast(variant) => {
                let RuntimeValue::Nominal(id) = value else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    ));
                };
                let HeapNode::Nominal {
                    variant: Some(actual),
                    ..
                } = &self.heap[id.0 as usize]
                else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    ));
                };
                if *actual != variant {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    ));
                }
                Ok(value.clone())
            }
            PlaceProjection::Field(field) => {
                let RuntimeValue::Nominal(id) = value else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    ));
                };
                let Some(position) = self.nominal_field_position(*id, field) else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    ));
                };
                let HeapNode::Nominal { fields, .. } = &self.heap[id.0 as usize] else {
                    unreachable!("nominal handles reference nominal heap nodes")
                };
                Ok(fields[position].clone())
            }
            PlaceProjection::TupleField(index) => {
                let RuntimeValue::Tuple(values) = value else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    ));
                };
                values.get(index as usize).cloned().ok_or_else(|| {
                    self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidAggregateProjection,
                    )
                })
            }
            PlaceProjection::ListIndex(index) => {
                let RuntimeValue::List(id) = value else {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::InvalidListProjection,
                    ));
                };
                let index = self.eval_index(function, block, origin, index, frame)?;
                let HeapNode::List(values) = &self.heap[id.0 as usize] else {
                    unreachable!("list handles reference list heap nodes")
                };
                if index < 0 || index as usize >= values.len() {
                    return Err(self.error(
                        Some(function),
                        Some(block),
                        Some(origin),
                        InterpreterErrorKind::IndexOutOfBounds {
                            index,
                            len: values.len(),
                        },
                    ));
                }
                Ok(values[index as usize].clone())
            }
        }
    }

    fn eval_index(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        index: Operand,
        frame: &Frame,
    ) -> Result<i64, InterpreterError> {
        let value = self.eval_operand(function, block, origin, index, frame)?;
        let RuntimeValue::Int(index) = value else {
            return Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::InvalidListProjection,
            ));
        };
        Ok(index)
    }

    fn nominal_field_position(&self, id: HeapId, field: crate::MirFieldId) -> Option<usize> {
        let HeapNode::Nominal {
            aggregate, variant, ..
        } = &self.heap[id.0 as usize]
        else {
            return None;
        };
        let fields = if let Some(variant) = variant {
            let descriptor = self.program.variant(*variant)?;
            if descriptor.aggregate != *aggregate {
                return None;
            }
            self.program.variant_fields(descriptor)
        } else {
            let descriptor = self.program.aggregate(*aggregate)?;
            self.program.aggregate_fields(descriptor)
        };
        fields.iter().position(|candidate| *candidate == field)
    }

    fn tick(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
    ) -> Result<(), InterpreterError> {
        if self.steps >= self.limits.max_steps {
            return Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::LimitExceeded(InterpreterLimit::Steps),
            ));
        }
        self.steps += 1;
        Ok(())
    }

    fn reserve_runtime_clone(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        origin: OriginId,
        value: &RuntimeValue,
    ) -> Result<(), InterpreterError> {
        let mut elements = 0usize;
        let mut pending = vec![value];
        // Cyclic structures (4E) must count each node once: the walk
        // is bounded by the number of reachable heap nodes.
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
        while let Some(value) = pending.pop() {
            let children: Vec<&RuntimeValue> = match value {
                RuntimeValue::Tuple(children) => children.iter().collect(),
                RuntimeValue::List(id) => {
                    if !seen.insert(id.0) {
                        continue;
                    }
                    let HeapNode::List(children) = &self.heap[id.0 as usize] else {
                        unreachable!("list handles reference list heap nodes")
                    };
                    children.iter().collect()
                }
                RuntimeValue::Nominal(id) => {
                    if !seen.insert(id.0) {
                        continue;
                    }
                    let HeapNode::Nominal { fields, .. } = &self.heap[id.0 as usize] else {
                        unreachable!("nominal handles reference nominal heap nodes")
                    };
                    fields.iter().collect()
                }
                RuntimeValue::Closure(id) => {
                    if !seen.insert(id.0) {
                        continue;
                    }
                    let HeapNode::Closure { captures, .. } = &self.heap[id.0 as usize] else {
                        unreachable!("closure handles reference closure heap nodes")
                    };
                    captures.iter().collect()
                }
                RuntimeValue::Task(id) => {
                    if !seen.insert(id.0) {
                        continue;
                    }
                    let HeapNode::Task {
                        arguments, result, ..
                    } = &self.heap[id.0 as usize]
                    else {
                        unreachable!("task handles reference task heap nodes")
                    };
                    let mut children: Vec<&RuntimeValue> = arguments.iter().collect();
                    if let Some(result) = result {
                        children.push(result);
                    }
                    children
                }
                RuntimeValue::Void
                | RuntimeValue::Bool(_)
                | RuntimeValue::Int(_)
                | RuntimeValue::FloatBits(_)
                | RuntimeValue::String(_)
                | RuntimeValue::Char(_)
                | RuntimeValue::Function(_) => continue,
            };
            let Some(next) = elements.checked_add(children.len()) else {
                return Err(self.error(
                    Some(function),
                    block,
                    Some(origin),
                    InterpreterErrorKind::LimitExceeded(InterpreterLimit::AggregateElements),
                ));
            };
            elements = next;
            if self.aggregate_elements.saturating_add(elements) > self.limits.max_aggregate_elements
            {
                return Err(self.error(
                    Some(function),
                    block,
                    Some(origin),
                    InterpreterErrorKind::LimitExceeded(InterpreterLimit::AggregateElements),
                ));
            }
            pending.extend(children);
        }
        self.reserve_elements(function, block, origin, elements)
    }

    fn reserve_elements(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        origin: OriginId,
        elements: usize,
    ) -> Result<(), InterpreterError> {
        let Some(next) = self.aggregate_elements.checked_add(elements) else {
            return Err(self.error(
                Some(function),
                block,
                Some(origin),
                InterpreterErrorKind::LimitExceeded(InterpreterLimit::AggregateElements),
            ));
        };
        if next > self.limits.max_aggregate_elements {
            return Err(self.error(
                Some(function),
                block,
                Some(origin),
                InterpreterErrorKind::LimitExceeded(InterpreterLimit::AggregateElements),
            ));
        }
        self.aggregate_elements = next;
        Ok(())
    }

    fn error(
        &self,
        function: Option<MirFunctionId>,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
        kind: InterpreterErrorKind,
    ) -> InterpreterError {
        InterpreterError {
            function,
            block,
            origin,
            kind,
        }
    }
}

// ── Deterministic builtin helpers ──────────────────────────────────────────
// The helpers below mirror the v1 host runtime (runtime/lpp_str.c and
// runtime/lpp_int.c) so interpreter output matches legacy execution for the
// deterministic pure subset.

fn builtin_invalid(
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
    argument: usize,
    actual: ExecutionValueKind,
) -> InterpreterError {
    InterpreterError {
        function: Some(function),
        block: Some(block),
        origin: Some(origin),
        kind: InterpreterErrorKind::InvalidBuiltinArgument { argument, actual },
    }
}

fn builtin_division_by_zero(
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
) -> InterpreterError {
    InterpreterError {
        function: Some(function),
        block: Some(block),
        origin: Some(origin),
        kind: InterpreterErrorKind::DivisionByZero,
    }
}

fn builtin_index_out_of_bounds(
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
    index: i64,
    len: usize,
) -> InterpreterError {
    InterpreterError {
        function: Some(function),
        block: Some(block),
        origin: Some(origin),
        kind: InterpreterErrorKind::IndexOutOfBounds { index, len },
    }
}

fn builtin_argument_kind(arguments: &[RuntimeValue], position: usize) -> ExecutionValueKind {
    match arguments.get(position) {
        Some(value) => value.kind(),
        None => ExecutionValueKind::Void,
    }
}

fn argument_int(arguments: &[RuntimeValue], position: usize) -> Option<i64> {
    match arguments.get(position)? {
        RuntimeValue::Int(value) => Some(*value),
        _ => None,
    }
}

fn argument_float_bits(arguments: &[RuntimeValue], position: usize) -> Option<u64> {
    match arguments.get(position)? {
        RuntimeValue::FloatBits(bits) => Some(*bits),
        _ => None,
    }
}

fn argument_bool(arguments: &[RuntimeValue], position: usize) -> Option<bool> {
    match arguments.get(position)? {
        RuntimeValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn argument_string_id(arguments: &[RuntimeValue], position: usize) -> Option<HeapId> {
    match arguments.get(position)? {
        RuntimeValue::String(id) => Some(*id),
        _ => None,
    }
}

fn argument_list_id(arguments: &[RuntimeValue], position: usize) -> Option<HeapId> {
    match arguments.get(position)? {
        RuntimeValue::List(id) => Some(*id),
        _ => None,
    }
}

/// Whether the value is a heap node (string, list, nominal, closure,
/// or task).
fn is_heap_value(value: &RuntimeValue) -> bool {
    matches!(
        value,
        RuntimeValue::String(_)
            | RuntimeValue::List(_)
            | RuntimeValue::Nominal(_)
            | RuntimeValue::Closure(_)
            | RuntimeValue::Task(_)
    )
}

/// Whether the value owns at least one heap reference, looking through
/// inline tuples (a tuple slot's death releases its heap elements).
fn owns_heap_refs(value: &RuntimeValue) -> bool {
    match value {
        RuntimeValue::Tuple(elements) => elements.iter().any(owns_heap_refs),
        _ => is_heap_value(value),
    }
}

/// Whether the type names a heap value (a runtime heap node, or an
/// inline tuple that can own one).
fn is_heap_type(types: &TypeInterner, ty: TypeId) -> bool {
    match types.kind(ty) {
        TypeKind::Function { .. }
        | TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Task(_)
        | TypeKind::Tuple(_)
        | TypeKind::Map { .. }
        | TypeKind::Nominal { .. } => true,
        TypeKind::Primitive(PrimitiveType::String) => true,
        _ => false,
    }
}

fn function_id_of_closure(interpreter: &Interpreter<'_>, id: HeapId) -> MirFunctionId {
    match &interpreter.heap[id.0 as usize] {
        HeapNode::Closure { function, .. } => *function,
        _ => unreachable!("closure handles reference closure heap nodes"),
    }
}

fn heap_id_of(value: &RuntimeValue) -> HeapId {
    match value {
        RuntimeValue::String(id)
        | RuntimeValue::List(id)
        | RuntimeValue::Nominal(id)
        | RuntimeValue::Closure(id)
        | RuntimeValue::Task(id) => *id,
        other => unreachable!("heap values carry heap ids: {other:?}"),
    }
}

fn heap_string(heap: &[HeapNode], id: HeapId) -> &str {
    match &heap[id.0 as usize] {
        HeapNode::String(text) => text,
        _ => unreachable!("string handles reference string heap nodes"),
    }
}

fn heap_list<'heap>(heap: &'heap [HeapNode], id: HeapId) -> &'heap Vec<RuntimeValue> {
    match &heap[id.0 as usize] {
        HeapNode::List(entries) => entries,
        _ => unreachable!("list handles reference list heap nodes"),
    }
}

fn heap_list_mut<'heap>(heap: &'heap mut [HeapNode], id: HeapId) -> &'heap mut Vec<RuntimeValue> {
    match &mut heap[id.0 as usize] {
        HeapNode::List(entries) => entries,
        _ => unreachable!("list handles reference list heap nodes"),
    }
}

fn builtin_make_string(
    interpreter: &mut Interpreter<'_>,
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
    text: String,
    produced: Option<TypeId>,
) -> Result<RuntimeValue, InterpreterError> {
    let id = interpreter.allocate_heap(
        function,
        Some(block),
        origin,
        HeapNode::String(text),
        produced,
    )?;
    Ok(RuntimeValue::String(id))
}

fn checked_binary(
    result: Option<i64>,
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
) -> Result<RuntimeValue, InterpreterError> {
    match result {
        Some(value) => Ok(RuntimeValue::Int(value)),
        None => Err(InterpreterError {
            function: Some(function),
            block: Some(block),
            origin: Some(origin),
            kind: InterpreterErrorKind::IntegerOverflow,
        }),
    }
}

fn abs_i64(value: i64) -> i64 {
    if value < 0 {
        value.wrapping_neg()
    } else {
        value
    }
}

fn min_u64(left: i64, right: i64) -> i64 {
    if (left as u64) < (right as u64) {
        left
    } else {
        right
    }
}

fn max_u64(left: i64, right: i64) -> i64 {
    if (left as u64) > (right as u64) {
        left
    } else {
        right
    }
}

/// v1 case folding touches ASCII letters only.
fn ascii_lowercase(text: &str) -> String {
    text.chars()
        .map(|character| {
            (if matches!(character, 'A'..='Z') {
                character as u8 + 32
            } else {
                character as u8
            }) as char
        })
        .collect()
}

/// v1 case folding touches ASCII letters only.
fn ascii_uppercase(text: &str) -> String {
    text.chars()
        .map(|character| {
            (if matches!(character, 'a'..='z') {
                character as u8 - 32
            } else {
                character as u8
            }) as char
        })
        .collect()
}

/// C `strtoll(s, NULL, 10)`: skip whitespace, optional sign, leading decimal
/// digits, clamp on overflow, and return 0 when no conversion is possible.
fn parse_i64_decimal(text: &str) -> i64 {
    let mut characters = text.chars().peekable();
    while matches!(
        characters.peek(),
        Some(' ') | Some('\t') | Some('\n') | Some('\u{0B}') | Some('\u{0C}') | Some('\r')
    ) {
        characters.next();
    }
    let sign = match characters.peek() {
        Some('+') => {
            characters.next();
            1
        }
        Some('-') => {
            characters.next();
            -1
        }
        _ => 1,
    };
    let mut value: i64 = 0;
    let mut converted = false;
    // Track true overflow: `saturating` arithmetic alone cannot tell
    // "exactly i64::MAX" from "past it", and a negative input past
    // i64::MAX must clamp to i64::MIN, not i64::MIN + 1.
    let mut overflow = false;
    for character in characters {
        let Some(digit) = (character as u8).checked_sub(b'0') else {
            break;
        };
        if digit > 9 {
            break;
        }
        let digit = digit as i64;
        if value > (i64::MAX - digit) / 10 {
            overflow = true;
            value = i64::MAX;
        } else {
            value = value * 10 + digit;
        }
        converted = true;
    }
    if !converted {
        return 0;
    }
    if sign < 0 {
        if overflow { i64::MIN } else { -value }
    } else {
        value
    }
}

/// v1 `lpp_str_to_u64`: skip whitespace, optional `0x` prefix, then hex or
/// decimal digits collected with wrapping shifts; stops at the first
/// non-digit.
fn parse_u64_hex_or_decimal(text: &str) -> i64 {
    let mut characters = text.chars().peekable();
    while matches!(
        characters.peek(),
        Some(' ') | Some('\t') | Some('\n') | Some('\r')
    ) {
        characters.next();
    }
    let mut hexadecimal = false;
    if characters.peek() == Some(&'0') {
        let second = characters.clone().nth(1);
        if matches!(second, Some('x') | Some('X')) {
            characters.next();
            characters.next();
            hexadecimal = true;
        }
    }
    let mut value: u64 = 0;
    for character in characters {
        if hexadecimal {
            match character.to_ascii_lowercase().to_digit(16) {
                Some(digit) => value = value.wrapping_shl(4) | digit as u64,
                None => break,
            }
        } else {
            match character {
                '0'..='9' => value = value.wrapping_mul(10) + character as u64 - b'0' as u64,
                _ => break,
            }
        }
    }
    value as i64
}

/// C `%g` with the default precision (six significant digits): trailing
/// zeros are stripped, and the exponent form appears when the exponent is
/// below -4 or at/above 6.
fn format_percent_g(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    let negative = value.is_sign_negative();
    let magnitude = value.abs();
    if magnitude == 0.0 {
        return if negative { "-0".into() } else { "0".into() };
    }
    let scientific = format!("{magnitude:.5e}");
    let (mantissa, exponent_part) = scientific.split_once('e').expect("scientific form");
    let exponent = i32::from_str_radix(exponent_part, 10).unwrap_or(0);
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let trimmed = digits.trim_end_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
    let sign = if negative { "-" } else { "" };
    let significant = i32::try_from(trimmed.len()).unwrap_or(i32::MAX);
    if exponent < -4 || exponent >= 6 {
        // C `%g` exponent form: `d[.ddd]e±EE` with trailing zeros stripped.
        let mantissa = if significant == 1 {
            trimmed.to_string()
        } else {
            format!("{}.{}", &trimmed[..1], &trimmed[1..])
        };
        format!("{sign}{mantissa}e{:+03}", exponent)
    } else if exponent >= significant - 1 {
        // Whole number: remaining positions are zeros.
        let zeros = "0".repeat((exponent - (significant - 1)) as usize);
        format!("{sign}{trimmed}{zeros}")
    } else if exponent >= 0 {
        let split_at = (exponent as usize) + 1;
        format!("{sign}{}.{}", &trimmed[..split_at], &trimmed[split_at..])
    } else {
        let zeros = "0".repeat((-exponent - 1) as usize);
        format!("{sign}0.{zeros}{trimmed}")
    }
}
