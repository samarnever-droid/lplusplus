use super::*;

impl Interpreter<'_> {
    /// Evaluate a transfer operand that consumes the source slot (the
    /// move half of the call contract). On the legacy entries this is a
    /// plain read.
    /// A read-only evaluation: a string literal read in a position
    /// that does not store gains a reader reference, which the read
    /// releases (the v1 expression end). Positions that store —
    /// assignments, place stores, moved arguments, tuple and list
    /// elements — take ownership instead and keep the reference.
    pub(super) fn eval_operand_borrowed(
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

    pub(super) fn eval_operand_moved(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        origin: OriginId,
        operand: Operand,
        frame: &mut Frame,
    ) -> Result<RuntimeValue, InterpreterError> {
        let value = self.eval_operand(function, block, origin, operand, frame)?;
        if self.arc.is_some()
            && let Operand::Copy(local) = operand
            && owns_heap_refs(&value)
        {
            frame.take(local);
        }
        Ok(value)
    }

    /// Whether a builtin moves (consumes) its positional value argument.
    /// The single source of truth for the builtin half of the call
    /// contract; list receivers, string arguments, and index operands
    /// never move.
    pub(super) fn builtin_moves_value(
        &self,
        builtin: lpp_types::BuiltinId,
        position: usize,
    ) -> bool {
        let name = builtin
            .descriptor()
            .name
            .strip_prefix("lpp_")
            .unwrap_or(builtin.descriptor().name);
        matches!(
            (name, position),
            ("list_push", 1) | ("list_set", 2) | ("print", 0) | ("write_str", 0)
        )
    }

    pub(super) fn load_place(
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

    pub(super) fn store_place(
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
            if self.arc.is_some()
                && let Some(old) = frame.get(place.root).cloned()
            {
                self.drop_reference(&old, Some(function), Some(block), Some(origin))?;
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

    pub(super) fn project_value(
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

    pub(super) fn eval_index(
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

    pub(super) fn nominal_field_position(
        &self,
        id: HeapId,
        field: crate::MirFieldId,
    ) -> Option<usize> {
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

    pub(super) fn tick(
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

    pub(super) fn reserve_runtime_clone(
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
                | RuntimeValue::Map(_)
                | RuntimeValue::Slice { .. }
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

    pub(super) fn reserve_elements(
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

    pub(super) fn error(
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
