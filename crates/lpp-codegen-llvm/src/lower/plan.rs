use super::*;

/// The module-level plan: string globals (deduped by content) and the
/// builtins in use, plus the resolved function symbols.
pub(crate) struct Plan {
    pub(crate) string_of: BTreeMap<String, u32>,
    pub(crate) builtins: BTreeSet<&'static str>,
    pub(crate) symbols: HashMap<MirFunctionId, String>,
}

/// One pre-pass: reject the non-slice-1 constructs (typed) and collect the
/// string globals, the builtins in use, and the function symbols.
pub(crate) fn build_plan(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn lpp_codegen_api::NameResolver,
) -> Result<Plan, CodegenError> {
    let mut strings: BTreeSet<String> = BTreeSet::new();
    let mut builtins: BTreeSet<&'static str> = BTreeSet::new();
    let mut symbols: HashMap<MirFunctionId, String> = HashMap::new();

    for (fn_id, function) in program.functions() {
        let name = function
            .name
            .and_then(|s| names.resolve(s.raw()))
            .map(str::to_string)
            .unwrap_or_else(|| format!("fn_{}", fn_id.raw()));
        symbols.insert(fn_id, name);
        for &block in program.function_blocks(function) {
            let block = program.block(block).unwrap();
            for &instr in program.block_instructions(block) {
                let kind = &program.instruction(instr).unwrap().kind;
                match kind {
                    InstructionKind::Assign { value, .. } => {
                        check_rvalue(program, types, fn_id, value, &mut strings, &mut builtins)?
                    }
                    InstructionKind::Store { value, .. } => {
                        check_operand(program, types, fn_id, value, &mut strings, &mut builtins)?
                    }
                }
            }
            check_terminator(program, types, fn_id, &block.terminator, &mut builtins)?;
        }
    }

    let string_of: BTreeMap<String, u32> = strings
        .iter()
        .enumerate()
        .map(|(index, content)| (content.clone(), index as u32))
        .collect();
    Ok(Plan {
        string_of,
        builtins,
        symbols,
    })
}

/// Reject non-slice-1 rvalues and collect strings/builtins.
fn check_rvalue(
    program: &MirProgram,
    types: &TypeInterner,
    fn_id: MirFunctionId,
    rvalue: &Rvalue,
    strings: &mut BTreeSet<String>,
    builtins: &mut BTreeSet<&'static str>,
) -> Result<(), CodegenError> {
    match rvalue {
        Rvalue::Use(o) | Rvalue::Unary { operand: o, .. } => {
            check_operand(program, types, fn_id, o, strings, builtins)
        }
        Rvalue::Binary {
            left,
            operator,
            right,
        } => {
            check_operand(program, types, fn_id, left, strings, builtins)?;
            check_operand(program, types, fn_id, right, strings, builtins)?;
            // String content ops call the C runtime.
            if operand_class(program, types, fn_id, left)? == Val::Ptr {
                match operator {
                    BinaryOperator::Equal | BinaryOperator::NotEqual => {
                        builtins.insert("lpp_str_eq");
                    }
                    BinaryOperator::Add => {
                        builtins.insert("lpp_str_concat");
                    }
                    _ => {
                        return Err(unsupported("string binary", Some(fn_id)));
                    }
                }
            } else if matches!(operator, BinaryOperator::Divide | BinaryOperator::Modulo)
                && operand_class(program, types, fn_id, left)? == Val::I64
            {
                builtins.insert("lpp_panic_div_zero");
            }
            Ok(())
        }
        Rvalue::Builtin { builtin, arguments } => {
            let raw_name = builtin.descriptor().name;
            let name = raw_name.strip_prefix("lpp_").unwrap_or(raw_name);
            let operands = program.operands(*arguments).to_vec();
            match name {
                "print" => {
                    let class = operand_class(program, types, fn_id, &operands[0])?;
                    let symbol = match class {
                        Val::I64 => "lpp_print_int",
                        Val::I8 => "lpp_print_bool",
                        Val::Ptr => "lpp_print_str",
                        // char prints its code point as an integer.
                        Val::I32 => "lpp_print_int",
                        _ => return Err(unsupported("print of unsupported type", Some(fn_id))),
                    };
                    builtins.insert(symbol);
                }
                "print_str" | "eprint_str" => {
                    builtins.insert("lpp_print_str");
                }
                "print_int" => {
                    builtins.insert("lpp_print_int");
                }
                "print_bool" => {
                    builtins.insert("lpp_print_bool");
                }
                "print_float" => {
                    builtins.insert("lpp_print_float");
                }
                "write_str" => {
                    builtins.insert("lpp_write_str");
                }
                "str_len" => {
                    builtins.insert("lpp_str_len");
                }
                "str_eq" => {
                    builtins.insert("lpp_str_eq");
                }
                "str_concat" => {
                    builtins.insert("lpp_str_concat");
                }
                _ => return Err(unsupported("unported builtin", Some(fn_id))),
            }
            for operand in &operands {
                check_operand(program, types, fn_id, operand, strings, builtins)?;
            }
            Ok(())
        }
        Rvalue::ExternalCall { .. } => Err(unsupported("external call", Some(fn_id))),
        Rvalue::Call { callee, arguments } => {
            // A direct call's callee is a function operand (supported). An
            // indirect call (function value) is a copy of a function-typed
            // local, which is the 5E2 function-value surface.
            match callee {
                Operand::Function(_) => {}
                Operand::Copy(local)
                    if val_of_type(types, program.local(*local).unwrap().ty)
                        .is_ok_and(|value| value == Val::Function) => {}
                Operand::Copy(_) | Operand::Constant(_) => {
                    return Err(unsupported("indirect call", Some(fn_id)));
                }
            }
            for operand in program.operands(*arguments) {
                check_operand(program, types, fn_id, operand, strings, builtins)?;
            }
            Ok(())
        }
        Rvalue::Load(place) => {
            let place = program.place(*place).unwrap();
            val_of_type(types, place.ty).map_err(|e| CodegenError::new(Some(fn_id), e.kind))?;
            Ok(())
        }
        Rvalue::List(_) | Rvalue::Tuple(_) => Err(unsupported("aggregate", Some(fn_id))),
        Rvalue::ConstructStruct { .. } => Err(unsupported("struct", Some(fn_id))),
        Rvalue::ConstructVariant { .. } => Err(unsupported("enum variant", Some(fn_id))),
        Rvalue::ListLen(_) => Err(unsupported("list length", Some(fn_id))),
        Rvalue::MakeClosure { .. } => Err(unsupported("closure", Some(fn_id))),
        Rvalue::Await(_) => Err(unsupported("await", Some(fn_id))),
        Rvalue::Spawn(_) => Err(unsupported("spawn", Some(fn_id))),
        Rvalue::Cast { .. } => Err(unsupported("cast", Some(fn_id))),
        Rvalue::Intrinsic { .. } => Err(unsupported("kernel intrinsic", Some(fn_id))),
    }
}

fn check_operand(
    program: &MirProgram,
    types: &TypeInterner,
    fn_id: MirFunctionId,
    operand: &Operand,
    strings: &mut BTreeSet<String>,
    _builtins: &mut BTreeSet<&'static str>,
) -> Result<(), CodegenError> {
    match operand {
        Operand::Copy(local) => {
            let local = program.local(*local).unwrap();
            val_of_type(types, local.ty).map_err(|e| CodegenError::new(Some(fn_id), e.kind))?;
            Ok(())
        }
        Operand::Constant(c) => {
            if let Constant::String { string, .. } = c {
                strings.insert(program.string(*string).unwrap().clone());
            }
            Ok(())
        }
        Operand::Function(target) => program
            .function(*target)
            .map(|_| ())
            .ok_or_else(|| unsupported("unknown function value", Some(fn_id))),
    }
}

/// The integer value of an operand, if it is an integer/bool/char constant.
/// This clang rejects width conversions of constexprs, so constants are
/// widened/narrowed in Rust at the conversion site.
pub(super) fn const_int_value(operand: &Operand) -> Option<i64> {
    match operand {
        Operand::Constant(c) => match c {
            Constant::Integer(v) => Some(*v),
            Constant::Bool(b) => Some(i64::from(u8::from(*b))),
            Constant::Character { character, .. } => Some(*character as i64),
            _ => None,
        },
        _ => None,
    }
}

/// The value class of an operand (without emitting), for dispatching the
/// polymorphic `print` and the string ops.
fn operand_class(
    program: &MirProgram,
    types: &TypeInterner,
    fn_id: MirFunctionId,
    operand: &Operand,
) -> Result<Val, CodegenError> {
    match operand {
        Operand::Copy(local) => {
            let local = program.local(*local).unwrap();
            val_of_type(types, local.ty).map_err(|e| CodegenError::new(Some(fn_id), e.kind))
        }
        Operand::Constant(c) => Ok(match c {
            Constant::Integer(_) => Val::I64,
            Constant::FloatBits(_) => Val::F64,
            Constant::Bool(_) => Val::I8,
            Constant::Character { .. } => Val::I32,
            Constant::String { .. } => Val::Ptr,
        }),
        Operand::Function(_) => Ok(Val::Function),
    }
}

fn check_terminator(
    _program: &MirProgram,
    _types: &TypeInterner,
    fn_id: MirFunctionId,
    term: &Terminator,
    _builtins: &mut BTreeSet<&'static str>,
) -> Result<(), CodegenError> {
    match term {
        Terminator::Goto(_)
        | Terminator::Branch { .. }
        | Terminator::Return(_)
        | Terminator::Unreachable => Ok(()),
        Terminator::SwitchEnum { .. } => Err(unsupported("switch on enum", Some(fn_id))),
    }
}
