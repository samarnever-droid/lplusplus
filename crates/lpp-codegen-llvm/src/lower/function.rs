use super::*;

/// Emit one function definition (non-SSA, alloca-based).
pub(super) fn emit_function(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
    function: &MirFunction,
    text: &mut String,
) -> Result<(), CodegenError> {
    let is_main = plan.symbols[&fn_id] == "main";
    let locals = program.function_locals(function).to_vec();
    let params = program.function_parameters(function).to_vec();

    // Return type: main returns the C `int` (i32); a void function is
    // `define void`; others the natural type.
    let ret_val = if is_main {
        Val::I32
    } else if matches!(
        types.kind(function.return_type),
        TypeKind::Primitive(PrimitiveType::Void)
    ) {
        Val::Void
    } else {
        val_of_type(types, function.return_type).unwrap_or(Val::I64)
    };

    // Parameter signature.
    let mut param_types: Vec<String> = Vec::with_capacity(params.len());
    for (i, &p) in params.iter().enumerate() {
        let ty = val_of_type(types, program.local(p).unwrap().ty).unwrap_or(Val::I64);
        param_types.push(format!("{} %a{i}", ty.ty()));
    }

    let name = &plan.symbols[&fn_id];
    let _ = std::fmt::Write::write_fmt(
        text,
        format_args!(
            "define {} @{name}({}) {{\n",
            ret_val.ty(),
            param_types.join(", ")
        ),
    );

    // The entry block is the first RPO block; allocas live there.
    let order = rpo(program, function);

    // Local slots (in function-local order) and parameter stores.
    let mut slot_of: HashMap<MirLocalId, String> = HashMap::new();
    let mut body = String::new();
    let entry = order[0];
    let entry_label = label(entry);
    let _ = std::fmt::Write::write_str(&mut body, &format!("{entry_label}:\n"));
    for (i, &local) in locals.iter().enumerate() {
        let ty = val_of_type(types, program.local(local).unwrap().ty).unwrap_or(Val::I64);
        let slot = format!("%slot{i}");
        let _ = std::fmt::Write::write_str(
            &mut body,
            &format!("{slot} = alloca {}, align {}\n", ty.ty(), ty.align()),
        );
        slot_of.insert(local, slot);
    }
    for (i, &p) in params.iter().enumerate() {
        let ty = val_of_type(types, program.local(p).unwrap().ty).unwrap_or(Val::I64);
        if let Some(slot) = slot_of.get(&p) {
            let _ = std::fmt::Write::write_str(
                &mut body,
                &format!(
                    "store {} %a{i}, ptr {slot}, align {}\n",
                    ty.ty(),
                    ty.align()
                ),
            );
        }
    }

    // Lower each block's instructions + terminator, in RPO order. The entry
    // block already carries its label and allocas; the rest get a label.
    let mut ssa: u32 = 0;
    let mut out = String::new();
    for (position, &block_id) in order.iter().enumerate() {
        let block = program.block(block_id).unwrap();
        if position > 0 {
            push(&mut body, &format!("{}:", label(block_id)));
        }
        for &instr in program.block_instructions(block) {
            let ir = &program.instruction(instr).unwrap().kind;
            match ir {
                InstructionKind::Assign { target, value } => {
                    let target_type = program.local(*target).unwrap().ty;
                    let void = matches!(
                        types.kind(target_type),
                        TypeKind::Primitive(PrimitiveType::Void)
                    );
                    let (val, expr) = emit_value(
                        program, types, plan, fn_id, value, &slot_of, &mut ssa, &mut out,
                    )?;
                    if void {
                        // A void expression (a void builtin): the target is a
                        // void temp that is never read, so there is no store.
                        continue;
                    }
                    let slot = &slot_of[target];
                    push(
                        &mut out,
                        &format!(
                            "store {} {expr}, ptr {slot}, align {}",
                            val.ty(),
                            val.align()
                        ),
                    );
                }
                InstructionKind::Store { place, value } => {
                    let (val, expr) = emit_operand(
                        program, types, plan, fn_id, value, &slot_of, &mut ssa, &mut out,
                    )?;
                    let slot = &slot_of[&program.place(*place).unwrap().root];
                    push(
                        &mut out,
                        &format!(
                            "store {} {expr}, ptr {slot}, align {}",
                            val.ty(),
                            val.align()
                        ),
                    );
                }
            }
        }
        let block_text = out.split_off(0);
        let _ = std::fmt::Write::write_str(&mut body, &block_text);
        // Terminator.
        emit_terminator(
            program,
            types,
            plan,
            fn_id,
            &block.terminator,
            &slot_of,
            &mut ssa,
            &mut body,
        )?;
    }

    let _ = std::fmt::Write::write_str(text, &body);
    let _ = std::fmt::Write::write_str(text, "}\n");
    Ok(())
}

/// The bare LLVM block name (labels are written `b0:`; references are `%b0`).
fn label(block: BasicBlockId) -> String {
    format!("b{}", block.raw())
}

/// The `%`-prefixed form used in branch/goto references.
fn label_ref(block: BasicBlockId) -> String {
    format!("%{}", label(block))
}

/// An IEEE decimal float literal. LLVM requires a decimal point (or an
/// exponent with one), so whole numbers like `10` are written `1.0e1`.
fn float_literal(bits: u64) -> String {
    let value = f64::from_bits(bits);
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    let mut text = format!("{:e}", value);
    if !text.contains('.') {
        match text.find(['e', 'E']) {
            Some(position) => text.insert_str(position, ".0"),
            None => text.push_str(".0"),
        }
    }
    text
}

fn rpo(program: &MirProgram, function: &MirFunction) -> Vec<BasicBlockId> {
    let entry = function.entry;
    let mut seen = BTreeSet::new();
    let mut stack: Vec<(BasicBlockId, bool)> = vec![(entry, false)];
    let mut post: Vec<BasicBlockId> = Vec::new();
    while let Some((blk, processed)) = stack.pop() {
        if processed {
            post.push(blk);
            continue;
        }
        if !seen.insert(blk) {
            continue;
        }
        stack.push((blk, true));
        if let Some(block) = program.block(blk) {
            for succ in successors(program, &block.terminator) {
                if !seen.contains(&succ) {
                    stack.push((succ, false));
                }
            }
        }
    }
    post.reverse();
    post
}

fn successors(program: &MirProgram, term: &Terminator) -> Vec<BasicBlockId> {
    match term {
        Terminator::Goto(t) => vec![*t],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::SwitchEnum { targets, .. } => {
            program.switch_targets(*targets).iter().copied().collect()
        }
        Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// Emit one instruction's rvalue as an LLVM expression, appending the
/// producing instructions to `out`. Returns the value class and expression.
fn fresh(ssa: &mut u32) -> String {
    let v = format!("%v{ssa}");
    *ssa += 1;
    v
}

fn emit_value(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
    rvalue: &Rvalue,
    slot_of: &HashMap<MirLocalId, String>,
    ssa: &mut u32,
    out: &mut String,
) -> Result<(Val, String), CodegenError> {
    match rvalue {
        Rvalue::Use(o) => emit_operand(program, types, plan, fn_id, o, slot_of, ssa, out),
        Rvalue::Unary { operator, operand } => {
            let (val, expr) =
                emit_operand(program, types, plan, fn_id, operand, slot_of, ssa, out)?;
            let v = fresh(ssa);
            match (*operator, val) {
                (UnaryOperator::Not, Val::I8) => push(out, &format!("{v} = sub i8 1, {expr}")),
                (UnaryOperator::Negate, Val::I64) => push(out, &format!("{v} = sub i64 0, {expr}")),
                (UnaryOperator::Negate, Val::F64) => {
                    push(out, &format!("{v} = fneg double {expr}"))
                }
                _ => return Err(unsupported("unary operator", Some(fn_id))),
            }
            Ok((val, v))
        }
        Rvalue::Binary {
            left,
            operator,
            right,
        } => emit_binary(
            program, types, plan, fn_id, *operator, left, right, slot_of, ssa, out,
        ),
        Rvalue::Builtin { builtin, arguments } => emit_builtin(
            program, types, plan, fn_id, *builtin, *arguments, slot_of, ssa, out,
        ),
        Rvalue::Call { callee, arguments } => emit_call(
            program, types, plan, fn_id, *callee, *arguments, slot_of, ssa, out,
        ),
        Rvalue::Load(place) => {
            let place = program.place(*place).unwrap();
            let val = val_of_type(types, place.ty)?;
            let slot = &slot_of[&place.root];
            let v = fresh(ssa);
            push(
                out,
                &format!("{v} = load {}, ptr {slot}, align {}", val.ty(), val.align()),
            );
            Ok((val, v))
        }
        _ => Err(unsupported("rvalue", Some(fn_id))),
    }
}

fn push(out: &mut String, line: &str) {
    let _ = std::fmt::Write::write_str(out, line);
    let _ = std::fmt::Write::write_str(out, "\n");
}

fn emit_operand(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
    operand: &Operand,
    slot_of: &HashMap<MirLocalId, String>,
    ssa: &mut u32,
    out: &mut String,
) -> Result<(Val, String), CodegenError> {
    match operand {
        Operand::Copy(local) => {
            let local_id = *local;
            let local = program.local(local_id).unwrap();
            let val = val_of_type(types, local.ty)?;
            let slot = slot_of.get(&local_id).expect("local mapped");
            let v = fresh(ssa);
            push(
                out,
                &format!("{v} = load {}, ptr {slot}, align {}", val.ty(), val.align()),
            );
            Ok((val, v))
        }
        // Constants are emitted as untyped literals (the instruction supplies
        // the type); floats are IEEE decimal literals (LLVM accepts NaN/inf).
        Operand::Constant(c) => Ok(match c {
            Constant::Integer(v) => (Val::I64, format!("{v}")),
            Constant::FloatBits(bits) => (Val::F64, float_literal(*bits)),
            Constant::Bool(b) => (Val::I8, format!("{}", u8::from(*b))),
            Constant::Character { character, .. } => (Val::I32, format!("{}", *character as i64)),
            Constant::String { string, .. } => {
                let content = program.string(*string).unwrap();
                let index = plan.string_of.get(content).expect("string in plan");
                let array_len = content.len() + 17;
                (
                    Val::Ptr,
                    format!(
                        "getelementptr inbounds ([{array_len} x i8], ptr @str_{index}, i64 0, i64 16)"
                    ),
                )
            }
        }),
        Operand::Function(target) => plan
            .symbols
            .get(target)
            .map(|name| (Val::Function, format!("@{name}")))
            .ok_or_else(|| unsupported("unknown function value", Some(fn_id))),
    }
}

fn emit_binary(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
    operator: BinaryOperator,
    left: &Operand,
    right: &Operand,
    slot_of: &HashMap<MirLocalId, String>,
    ssa: &mut u32,
    out: &mut String,
) -> Result<(Val, String), CodegenError> {
    let (lval, le) = emit_operand(program, types, plan, fn_id, left, slot_of, ssa, out)?;
    let (rval, re) = emit_operand(program, types, plan, fn_id, right, slot_of, ssa, out)?;
    if lval != rval {
        return Err(unsupported("mixed-type binary", Some(fn_id)));
    }
    let is_cmp = matches!(
        operator,
        BinaryOperator::Less
            | BinaryOperator::Greater
            | BinaryOperator::LessEqual
            | BinaryOperator::GreaterEqual
            | BinaryOperator::Equal
            | BinaryOperator::NotEqual
    );
    if lval == Val::Function {
        return Err(unsupported("function-value binary", Some(fn_id)));
    }
    // String comparisons produce an i8 bool via lpp_str_eq.
    if lval == Val::Ptr {
        return match operator {
            BinaryOperator::Add => {
                let v = fresh(ssa);
                push(
                    out,
                    &format!("{v} = call ptr @lpp_str_concat(ptr {le}, ptr {re})"),
                );
                Ok((Val::Ptr, v))
            }
            BinaryOperator::Equal | BinaryOperator::NotEqual => {
                let eq = fresh(ssa);
                push(
                    out,
                    &format!("{eq} = call i64 @lpp_str_eq(ptr {le}, ptr {re})"),
                );
                let v = fresh(ssa);
                // `lpp_str_eq` returns 1 when equal, so `==` is `!= 0` and
                // `!=` is `== 0`.
                let op = if operator == BinaryOperator::Equal {
                    "ne"
                } else {
                    "eq"
                };
                push(out, &format!("{v} = icmp {op} i64 {eq}, 0"));
                let v8 = fresh(ssa);
                push(out, &format!("{v8} = zext i1 {v} to i8"));
                Ok((Val::I8, v8))
            }
            _ => Err(unsupported("string binary", Some(fn_id))),
        };
    }
    if lval == Val::I64 && matches!(operator, BinaryOperator::Divide | BinaryOperator::Modulo) {
        // Match the MIR oracle exactly: zero is a defined runtime panic,
        // and INT_MIN / -1 wraps rather than becoming LLVM poison/UB.
        let is_zero = fresh(ssa);
        push(out, &format!("{is_zero} = icmp eq i64 {re}, 0"));
        let block = *ssa;
        *ssa += 1;
        push(
            out,
            &format!("br i1 {is_zero}, label %div_zero_{block}, label %div_ok_{block}"),
        );
        push(out, &format!("div_zero_{block}:"));
        push(out, "call void @lpp_panic_div_zero()");
        push(out, "unreachable");
        push(out, &format!("div_ok_{block}:"));

        let is_min = fresh(ssa);
        push(
            out,
            &format!("{is_min} = icmp eq i64 {le}, -9223372036854775808"),
        );
        let is_neg_one = fresh(ssa);
        push(out, &format!("{is_neg_one} = icmp eq i64 {re}, -1"));
        let overflow = fresh(ssa);
        push(out, &format!("{overflow} = and i1 {is_min}, {is_neg_one}"));
        let safe_right = fresh(ssa);
        push(
            out,
            &format!("{safe_right} = select i1 {overflow}, i64 1, i64 {re}"),
        );
        let result = fresh(ssa);
        let op = if operator == BinaryOperator::Divide {
            "sdiv"
        } else {
            "srem"
        };
        push(out, &format!("{result} = {op} i64 {le}, {safe_right}"));
        return Ok((Val::I64, result));
    }

    let v = fresh(ssa);
    let (op, res_ty) = match (operator, lval) {
        (BinaryOperator::Add, Val::I64) => ("add", "i64"),
        (BinaryOperator::Subtract, Val::I64) => ("sub", "i64"),
        (BinaryOperator::Multiply, Val::I64) => ("mul", "i64"),
        (BinaryOperator::Divide, Val::I64) => ("sdiv", "i64"),
        (BinaryOperator::Modulo, Val::I64) => ("srem", "i64"),
        (BinaryOperator::BitAnd, Val::I64) => ("and", "i64"),
        (BinaryOperator::BitOr, Val::I64) => ("or", "i64"),
        (BinaryOperator::BitXor, Val::I64) => ("xor", "i64"),
        (BinaryOperator::ShiftLeft, Val::I64) => ("shl", "i64"),
        (BinaryOperator::ShiftRight, Val::I64) => ("ashr", "i64"),
        (BinaryOperator::Add, Val::F64) => ("fadd", "double"),
        (BinaryOperator::Subtract, Val::F64) => ("fsub", "double"),
        (BinaryOperator::Multiply, Val::F64) => ("fmul", "double"),
        (BinaryOperator::Divide, Val::F64) => ("fdiv", "double"),
        (BinaryOperator::Modulo, Val::F64) => ("frem", "double"),
        (BinaryOperator::LogicalAnd, Val::I8) => ("and", "i8"),
        (BinaryOperator::LogicalOr, Val::I8) => ("or", "i8"),
        _ => ("", ""),
    };
    if is_cmp {
        let cmp = match lval {
            Val::I64 => match operator {
                BinaryOperator::Less => "slt",
                BinaryOperator::Greater => "sgt",
                BinaryOperator::LessEqual => "sle",
                BinaryOperator::GreaterEqual => "sge",
                BinaryOperator::Equal => "eq",
                BinaryOperator::NotEqual => "ne",
                _ => "",
            },
            Val::F64 => match operator {
                BinaryOperator::Less => "olt",
                BinaryOperator::Greater => "ogt",
                BinaryOperator::LessEqual => "ole",
                BinaryOperator::GreaterEqual => "oge",
                BinaryOperator::Equal => "oeq",
                // `!=` must be `une` (unordered): NaN != NaN is true in IEEE.
                BinaryOperator::NotEqual => "une",
                _ => "",
            },
            Val::I32 => match operator {
                BinaryOperator::Less => "slt",
                BinaryOperator::Greater => "sgt",
                BinaryOperator::LessEqual => "sle",
                BinaryOperator::GreaterEqual => "sge",
                BinaryOperator::Equal => "eq",
                BinaryOperator::NotEqual => "ne",
                _ => "",
            },
            Val::I8 => match operator {
                BinaryOperator::Equal => "eq",
                BinaryOperator::NotEqual => "ne",
                _ => "",
            },
            _ => "",
        };
        let cmp_inst = if lval == Val::F64 { "fcmp" } else { "icmp" };
        push(
            out,
            &format!("{v} = {cmp_inst} {cmp} {} {le}, {re}", lval.ty()),
        );
        let v8 = fresh(ssa);
        push(out, &format!("{v8} = zext i1 {v} to i8"));
        return Ok((Val::I8, v8));
    }
    if op.is_empty() {
        return Err(unsupported("binary operator", Some(fn_id)));
    }
    push(out, &format!("{v} = {op} {res_ty} {le}, {re}"));
    Ok((lval, v))
}

fn emit_call(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
    callee: Operand,
    arguments: lpp_mir::ListRange<lpp_mir::Operand>,
    slot_of: &HashMap<MirLocalId, String>,
    ssa: &mut u32,
    out: &mut String,
) -> Result<(Val, String), CodegenError> {
    let (ret, callee_expression) = match &callee {
        Operand::Function(target) => {
            let target_fn = program
                .function(*target)
                .ok_or_else(|| unsupported("unknown direct callee", Some(fn_id)))?;
            let ret = if matches!(
                types.kind(target_fn.return_type),
                TypeKind::Primitive(PrimitiveType::Void)
            ) {
                Val::Void
            } else {
                val_of_type(types, target_fn.return_type)?
            };
            let name = plan
                .symbols
                .get(target)
                .ok_or_else(|| unsupported("unknown direct callee", Some(fn_id)))?;
            (ret, format!("@{name}"))
        }
        Operand::Copy(local) => {
            let local_type = program.local(*local).unwrap().ty;
            let TypeKind::Function { result, .. } = types.kind(local_type) else {
                return Err(unsupported("indirect call", Some(fn_id)));
            };
            let ret = if matches!(types.kind(result), TypeKind::Primitive(PrimitiveType::Void)) {
                Val::Void
            } else {
                val_of_type(types, result)?
            };
            let (kind, expression) =
                emit_operand(program, types, plan, fn_id, &callee, slot_of, ssa, out)?;
            if kind != Val::Function {
                return Err(unsupported("indirect call", Some(fn_id)));
            }
            (ret, expression)
        }
        Operand::Constant(_) => return Err(unsupported("indirect call", Some(fn_id))),
    };
    let operands = program.operands(arguments).to_vec();
    let mut parts = Vec::with_capacity(operands.len());
    for operand in &operands {
        let (val, expr) = emit_operand(program, types, plan, fn_id, operand, slot_of, ssa, out)?;
        parts.push(format!("{} {expr}", val.ty()));
    }
    if ret == Val::Void {
        // A void call has no result to bind.
        push(
            out,
            &format!("call void {callee_expression}({})", parts.join(", ")),
        );
        Ok((Val::Void, String::new()))
    } else {
        let v = fresh(ssa);
        push(
            out,
            &format!(
                "{v} = call {} {callee_expression}({})",
                ret.ty(),
                parts.join(", ")
            ),
        );
        Ok((ret, v))
    }
}

fn emit_builtin(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
    builtin: BuiltinId,
    arguments: lpp_mir::ListRange<lpp_mir::Operand>,
    slot_of: &HashMap<MirLocalId, String>,
    ssa: &mut u32,
    out: &mut String,
) -> Result<(Val, String), CodegenError> {
    let raw_name = builtin.descriptor().name;
    let name = raw_name.strip_prefix("lpp_").unwrap_or(raw_name);
    let operands = program.operands(arguments).to_vec();
    let mut arg_vals: Vec<(Val, String)> = Vec::with_capacity(operands.len());
    for operand in &operands {
        arg_vals.push(emit_operand(
            program, types, plan, fn_id, operand, slot_of, ssa, out,
        )?);
    }
    let (aval, aexpr) = &arg_vals[0];
    let v = fresh(ssa);
    match name {
        "print" => match aval {
            Val::I64 => push(out, &format!("call void @lpp_print_int(i64 {aexpr})")),
            Val::I8 => push(out, &format!("call void @lpp_print_bool(i8 {aexpr})")),
            Val::Ptr => push(out, &format!("call void @lpp_print_str(ptr {aexpr})")),
            // A char prints its code point; a constant is widened in Rust
            // (this clang rejects `sext` of a constexpr), an SSA value via `sext`.
            Val::I32 => match const_int_value(&operands[0]) {
                Some(value) => push(out, &format!("call void @lpp_print_int(i64 {value})")),
                None => push(
                    out,
                    &format!("call void @lpp_print_int(i64 sext i32 {aexpr} to i64)"),
                ),
            },
            _ => return Err(unsupported("print of unsupported type", Some(fn_id))),
        },
        "print_str" | "eprint_str" => push(out, &format!("call void @lpp_print_str(ptr {aexpr})")),
        "print_int" => push(out, &format!("call void @lpp_print_int(i64 {aexpr})")),
        "print_bool" => push(out, &format!("call void @lpp_print_bool(i8 {aexpr})")),
        "print_float" => push(out, &format!("call void @lpp_print_float(double {aexpr})")),
        "write_str" => push(out, &format!("call void @lpp_write_str(ptr {aexpr})")),
        "str_len" => {
            push(out, &format!("{v} = call i64 @lpp_str_len(ptr {aexpr})"));
            return Ok((Val::I64, v));
        }
        "str_eq" => {
            let Some((Val::Ptr, right)) = arg_vals.get(1) else {
                return Err(unsupported("str_eq arguments", Some(fn_id)));
            };
            let raw = fresh(ssa);
            push(
                out,
                &format!("{raw} = call i64 @lpp_str_eq(ptr {aexpr}, ptr {right})"),
            );
            push(out, &format!("{v} = trunc i64 {raw} to i8"));
            return Ok((Val::I8, v));
        }
        "str_concat" => {
            let Some((Val::Ptr, right)) = arg_vals.get(1) else {
                return Err(unsupported("str_concat arguments", Some(fn_id)));
            };
            push(
                out,
                &format!("{v} = call ptr @lpp_str_concat(ptr {aexpr}, ptr {right})"),
            );
            return Ok((Val::Ptr, v));
        }
        _ => return Err(unsupported("unported builtin", Some(fn_id))),
    }
    // Void builtins' result is discarded by valid MIR.
    Ok((Val::Void, String::new()))
}

fn emit_terminator(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
    term: &Terminator,
    slot_of: &HashMap<MirLocalId, String>,
    ssa: &mut u32,
    out: &mut String,
) -> Result<(), CodegenError> {
    match term {
        Terminator::Goto(target) => {
            push(out, &format!("br label {}", label_ref(*target)));
        }
        Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            let (val, expr) =
                emit_operand(program, types, plan, fn_id, condition, slot_of, ssa, out)?;
            // Source conditions permit integer truthiness. LLVM branches on
            // i1, so normalize every supported scalar condition to `!= 0`.
            let v = fresh(ssa);
            match val {
                Val::I8 => push(out, &format!("{v} = icmp ne i8 {expr}, 0")),
                Val::I32 => push(out, &format!("{v} = icmp ne i32 {expr}, 0")),
                Val::I64 => push(out, &format!("{v} = icmp ne i64 {expr}, 0")),
                _ => return Err(unsupported("branch condition", Some(fn_id))),
            }
            push(
                out,
                &format!(
                    "br i1 {v}, label {}, label {}",
                    label_ref(*then_block),
                    label_ref(*else_block)
                ),
            );
        }
        Terminator::Return(value) => {
            let is_main = plan.symbols.get(&fn_id).is_some_and(|n| n == "main");
            let Some(value) = value else {
                // The platform entry point is always C `int main(...)`, even
                // when L++ source omits an explicit return type/value.
                push(out, if is_main { "ret i32 0" } else { "ret void" });
                return Ok(());
            };
            let (val, expr) = emit_operand(program, types, plan, fn_id, value, slot_of, ssa, out)?;
            if is_main {
                // The C entry point returns `int`; truncate/widen the value.
                // A constant is converted in Rust (this clang rejects `trunc`
                // / `zext` of a constexpr); an SSA value via the instruction.
                if let Some(value) = const_int_value(value) {
                    push(out, &format!("ret i32 {}", value as i32));
                } else {
                    let v = fresh(ssa);
                    if val == Val::I64 {
                        push(out, &format!("{v} = trunc i64 {expr} to i32"));
                    } else {
                        push(out, &format!("{v} = zext {} {expr} to i32", val.ty()));
                    }
                    push(out, &format!("ret i32 {v}"));
                }
            } else {
                push(out, &format!("ret {} {expr}", val.ty()));
            }
        }
        Terminator::Unreachable => push(out, "unreachable"),
        Terminator::SwitchEnum { .. } => return Err(unsupported("switch on enum", Some(fn_id))),
    }
    Ok(())
}
