use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_builtin(
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
        let map_at = |position: usize| -> Result<HeapId, InterpreterError> {
            argument_map_id(arguments, position).ok_or_else(|| invalid_at(position))
        };
        let slice_at = |position: usize| -> Result<(HeapId, usize, usize, bool), InterpreterError> {
            match arguments.get(position) {
                Some(RuntimeValue::Slice {
                    source,
                    start,
                    len,
                    string,
                }) => Ok((*source, *start, *len, *string)),
                _ => Err(invalid_at(position)),
            }
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
            "str_substr" => {
                let text = string_at(0)?;
                let start = int_at(1)?;
                let len = int_at(2)?;
                let bytes = text.as_bytes();
                let start = usize::try_from(start).ok();
                let len = usize::try_from(len).ok().filter(|len| *len > 0);
                let result = start
                    .zip(len)
                    .filter(|(start, _)| *start < bytes.len())
                    .map(|(start, len)| {
                        let end = start.saturating_add(len).min(bytes.len());
                        String::from_utf8_lossy(&bytes[start..end]).into_owned()
                    })
                    .unwrap_or_default();
                builtin_make_string(self, function, block, origin, result, produced)
            }
            "str_repeat" => {
                let text = string_at(0)?;
                let count = int_at(1)?;
                let result = usize::try_from(count)
                    .ok()
                    .filter(|count| *count > 0)
                    .map(|count| text.repeat(count))
                    .unwrap_or_default();
                builtin_make_string(self, function, block, origin, result, produced)
            }
            "char_at" => {
                let text = string_at(0)?;
                let index = usize::try_from(int_at(1)?).ok();
                let result = index
                    .and_then(|index| text.as_bytes().get(index).copied())
                    .map(|byte| String::from_utf8_lossy(&[byte]).into_owned())
                    .unwrap_or_default();
                builtin_make_string(self, function, block, origin, result, produced)
            }
            "ord" => {
                let text = string_at(0)?;
                Ok(RuntimeValue::Int(
                    text.chars()
                        .next()
                        .map_or(0, |character| i64::from(character as u32)),
                ))
            }
            "chr" => {
                let code = u32::try_from(int_at(0)?).ok();
                let text = code
                    .and_then(char::from_u32)
                    .map_or_else(String::new, |character| character.to_string());
                builtin_make_string(self, function, block, origin, text, produced)
            }
            "str_eq" => {
                let left = string_at(0)?;
                let right = string_at(1)?;
                Ok(RuntimeValue::Int(i64::from(left == right)))
            }
            "str_split" => {
                let text = string_at(0)?;
                let delimiter = int_at(1)? as u8;
                let pieces: Vec<String> = text
                    .as_bytes()
                    .split(|byte| *byte == delimiter)
                    .map(|piece| String::from_utf8_lossy(piece).into_owned())
                    .collect();
                self.reserve_elements(function, Some(block), origin, pieces.len())?;
                let mut values = Vec::with_capacity(pieces.len());
                for piece in pieces {
                    let id = self.allocate_heap(
                        function,
                        Some(block),
                        origin,
                        HeapNode::String(piece),
                        None,
                    )?;
                    values.push(RuntimeValue::String(id));
                }
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::List(values),
                    produced,
                )?;
                Ok(RuntimeValue::List(id))
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
                Ok(RuntimeValue::Int(if !(0..64).contains(&shift) {
                    0
                } else {
                    ((left as u64) >> shift) as i64
                }))
            }
            "shl_u" => {
                let (left, shift) = (int_at(0)?, int_at(1)?);
                Ok(RuntimeValue::Int(if !(0..64).contains(&shift) {
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
            "sin" => Ok(RuntimeValue::FloatBits(float_at(0)?.sin().to_bits())),
            "cos" => Ok(RuntimeValue::FloatBits(float_at(0)?.cos().to_bits())),
            "int_pow" => {
                let base = int_at(0)?;
                let exponent = int_at(1)?;
                if exponent < 0 {
                    Ok(RuntimeValue::Int(0))
                } else {
                    let mut base = base;
                    let mut exponent = exponent as u64;
                    let mut result = 1i64;
                    while exponent > 0 {
                        if exponent & 1 != 0 {
                            result = result.wrapping_mul(base);
                        }
                        base = base.wrapping_mul(base);
                        exponent >>= 1;
                    }
                    Ok(RuntimeValue::Int(result))
                }
            }
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

            // ── Deterministic integer/string-key maps ──────────────────────
            "map_new" | "map_new_arc" => {
                let id = self.allocate_heap(
                    function,
                    Some(block),
                    origin,
                    HeapNode::Map(Vec::new()),
                    produced,
                )?;
                Ok(RuntimeValue::Map(id))
            }
            "map_put" | "map_put_str" | "map_put_float" | "map_put_str_float" => {
                let id = map_at(0)?;
                let key = if name.contains("_str") {
                    MapKey::String(string_at(1)?)
                } else {
                    MapKey::Int(int_at(1)?)
                };
                let value = if name.ends_with("_float") {
                    float_at(2)?.to_bits() as i64
                } else {
                    int_at(2)?
                };
                let position = heap_map(&self.heap, id)
                    .iter()
                    .position(|(candidate, _)| *candidate == key);
                if let Some(position) = position {
                    heap_map_mut(&mut self.heap, id)[position].1 = value;
                } else {
                    self.reserve_elements(function, Some(block), origin, 1)?;
                    heap_map_mut(&mut self.heap, id).push((key, value));
                }
                Ok(RuntimeValue::Void)
            }
            "map_get" | "map_get_str" | "map_get_float" | "map_get_str_float" => {
                let id = map_at(0)?;
                let key = if name.contains("_str") {
                    MapKey::String(string_at(1)?)
                } else {
                    MapKey::Int(int_at(1)?)
                };
                let value = heap_map(&self.heap, id)
                    .iter()
                    .find(|(candidate, _)| *candidate == key)
                    .map_or(0, |(_, value)| *value);
                if name.ends_with("_float") {
                    Ok(RuntimeValue::FloatBits(value as u64))
                } else {
                    Ok(RuntimeValue::Int(value))
                }
            }
            "map_has" | "map_has_str" => {
                let id = map_at(0)?;
                let key = if name.ends_with("_str") {
                    MapKey::String(string_at(1)?)
                } else {
                    MapKey::Int(int_at(1)?)
                };
                Ok(RuntimeValue::Bool(
                    heap_map(&self.heap, id)
                        .iter()
                        .any(|(candidate, _)| *candidate == key),
                ))
            }
            "map_remove" | "map_remove_str" => {
                let id = map_at(0)?;
                let key = if name.ends_with("_str") {
                    MapKey::String(string_at(1)?)
                } else {
                    MapKey::Int(int_at(1)?)
                };
                let position = heap_map(&self.heap, id)
                    .iter()
                    .position(|(candidate, _)| *candidate == key);
                if let Some(position) = position {
                    heap_map_mut(&mut self.heap, id).swap_remove(position);
                }
                Ok(RuntimeValue::Void)
            }
            "map_len" => {
                let id = map_at(0)?;
                let len = heap_map(&self.heap, id).len();
                Ok(RuntimeValue::Int(i64::try_from(len).unwrap_or(i64::MAX)))
            }

            // ── Borrowed list/string slices ────────────────────────────────
            "slice" | "str_slice" => {
                let start_raw = int_at(1)?;
                let len_raw = int_at(2)?;
                let start = usize::try_from(start_raw).ok();
                let len = usize::try_from(len_raw).ok();
                let string = name == "str_slice";
                let source = if string {
                    argument_string_id(arguments, 0).ok_or_else(|| invalid_at(0))?
                } else {
                    list_at(0)?
                };
                let source_len = if string {
                    heap_string(&self.heap, source).len()
                } else {
                    heap_list(&self.heap, source).len()
                };
                let valid = start
                    .zip(len)
                    .and_then(|(start, len)| start.checked_add(len).map(|end| (start, len, end)))
                    .filter(|(_, _, end)| *end <= source_len);
                let Some((start, len, _)) = valid else {
                    return Err(builtin_index_out_of_bounds(
                        function,
                        block,
                        origin,
                        start_raw.saturating_add(len_raw),
                        source_len,
                    ));
                };
                Ok(RuntimeValue::Slice {
                    source,
                    start,
                    len,
                    string,
                })
            }
            "slice_len" => {
                let (_, _, len, _) = slice_at(0)?;
                Ok(RuntimeValue::Int(i64::try_from(len).unwrap_or(i64::MAX)))
            }
            "slice_get" | "slice_get_bool" => {
                let (source, start, len, string) = slice_at(0)?;
                let index_raw = int_at(1)?;
                let index = usize::try_from(index_raw)
                    .ok()
                    .filter(|index| *index < len)
                    .ok_or_else(|| {
                        builtin_index_out_of_bounds(function, block, origin, index_raw, len)
                    })?;
                let absolute = start + index;
                if string {
                    let byte = heap_string(&self.heap, source).as_bytes()[absolute];
                    builtin_make_string(
                        self,
                        function,
                        block,
                        origin,
                        String::from_utf8_lossy(&[byte]).into_owned(),
                        produced,
                    )
                } else {
                    let value = heap_list(&self.heap, source)[absolute].clone();
                    if name == "slice_get_bool" && !matches!(value, RuntimeValue::Bool(_)) {
                        return Err(invalid(0, value.kind()));
                    }
                    if self.arc.is_some() && owns_heap_refs(&value) {
                        self.retain_owned(function, Some(block), Some(origin), &value)?;
                    }
                    Ok(value)
                }
            }
            "slice_to_str" | "str_slice_to_str" => {
                let (source, start, len, string) = slice_at(0)?;
                if !string {
                    return Err(invalid_at(0));
                }
                let bytes = &heap_string(&self.heap, source).as_bytes()[start..start + len];
                builtin_make_string(
                    self,
                    function,
                    block,
                    origin,
                    String::from_utf8_lossy(bytes).into_owned(),
                    produced,
                )
            }

            _ => Err(self.error(
                Some(function),
                Some(block),
                Some(origin),
                InterpreterErrorKind::UnsupportedBuiltin,
            )),
        }
    }
}
