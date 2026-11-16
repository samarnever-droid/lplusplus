use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_operands(
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

    pub(super) fn eval_operand(
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

    pub(super) fn eval_unary(
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
            (UnaryOperator::Plus, RuntimeValue::Int(value)) => Ok(RuntimeValue::Int(value)),
            (UnaryOperator::Plus, RuntimeValue::FloatBits(value)) => {
                Ok(RuntimeValue::FloatBits(value))
            }
            (UnaryOperator::BitNot, RuntimeValue::Int(value)) => Ok(RuntimeValue::Int(!value)),
            _ => Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::InvalidUnaryOperands,
            )),
        }
    }

    pub(super) fn eval_binary(
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
                        RuntimeValue::Int(_)
                            | RuntimeValue::FloatBits(_)
                            | RuntimeValue::Bool(_)
                            | RuntimeValue::Char(_)
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
                    RuntimeValue::Char(ch) => combined.push(ch),
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

    pub(super) fn runtime_equal(&self, left: &RuntimeValue, right: &RuntimeValue) -> bool {
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
                (RuntimeValue::Map(left), RuntimeValue::Map(right)) => {
                    if left == right {
                        continue;
                    }
                    let (HeapNode::Map(left), HeapNode::Map(right)) =
                        (&self.heap[left.0 as usize], &self.heap[right.0 as usize])
                    else {
                        return false;
                    };
                    if left != right {
                        return false;
                    }
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

    pub(super) fn eval_integer(
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
            BinaryOperator::Power => {
                if right < 0 {
                    Some(RuntimeValue::Int(0))
                } else {
                    let mut res = 1i64;
                    let mut base = left;
                    let mut exp = right as u64;
                    while exp > 0 {
                        if (exp & 1) != 0 {
                            res = res.wrapping_mul(base);
                        }
                        base = base.wrapping_mul(base);
                        exp >>= 1;
                    }
                    Some(RuntimeValue::Int(res))
                }
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

    pub(super) fn eval_char(
        &self,
        operator: BinaryOperator,
        left: char,
        right: char,
    ) -> Option<RuntimeValue> {
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

    pub(super) fn eval_float(
        &self,
        operator: BinaryOperator,
        left: f64,
        right: f64,
    ) -> Option<RuntimeValue> {
        match operator {
            BinaryOperator::Add => Some(RuntimeValue::FloatBits((left + right).to_bits())),
            BinaryOperator::Subtract => Some(RuntimeValue::FloatBits((left - right).to_bits())),
            BinaryOperator::Multiply => Some(RuntimeValue::FloatBits((left * right).to_bits())),
            BinaryOperator::Divide => Some(RuntimeValue::FloatBits((left / right).to_bits())),
            BinaryOperator::Modulo => Some(RuntimeValue::FloatBits((left % right).to_bits())),
            BinaryOperator::Power => Some(RuntimeValue::FloatBits(left.powf(right).to_bits())),
            BinaryOperator::Equal => Some(RuntimeValue::Bool(left == right)),
            BinaryOperator::NotEqual => Some(RuntimeValue::Bool(left != right)),
            BinaryOperator::Less => Some(RuntimeValue::Bool(left < right)),
            BinaryOperator::Greater => Some(RuntimeValue::Bool(left > right)),
            BinaryOperator::LessEqual => Some(RuntimeValue::Bool(left <= right)),
            BinaryOperator::GreaterEqual => Some(RuntimeValue::Bool(left >= right)),
            _ => None,
        }
    }

    pub(super) fn import_value(
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

    pub(super) fn export_value(
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
            // Maps are still represented as opaque Int handles by the
            // frontend. Expose a deterministic nonzero handle if one crosses
            // the execution boundary; normal map programs return scalars.
            RuntimeValue::Map(id) => ExecutionValue::Int(i64::from(id.0) + 1),
            RuntimeValue::Slice {
                source,
                start,
                len,
                string,
            } => {
                if *string {
                    let bytes = &heap_string(&self.heap, *source).as_bytes()[*start..*start + *len];
                    ExecutionValue::String(String::from_utf8_lossy(bytes).into_owned())
                } else {
                    let values = &heap_list(&self.heap, *source)[*start..*start + *len];
                    ExecutionValue::List(
                        values
                            .iter()
                            .map(|value| self.export_value(function, origin, value))
                            .collect::<Result<_, _>>()?,
                    )
                }
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
}
