//! The MIR -> textual LLVM IR emitter (5E slice 1: the scalar data surface).
//!
//! Non-SSA, alloca-based: every MIR local is an `alloca` at the function
//! entry, reads are `load`s and writes are `store`s. This trades some
//! efficiency for a straightforward, phi-free lowering that is correct for
//! the slice-1 (no managed types) differential.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use lpp_codegen_api::{CodegenError, CodegenErrorKind};
use lpp_mir::{
    BasicBlockId, BinaryOperator, Constant, InstructionKind, MirFunction, MirFunctionId,
    MirLocalId, MirProgram, Operand, Rvalue, Terminator, UnaryOperator,
};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind};

/// A lowered value's LLVM type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Val {
    I64,
    F64,
    I8,
    I32,
    Ptr,
    /// A void function's return type (`define void`, `ret void`).
    Void,
}

impl Val {
    fn ty(self) -> &'static str {
        match self {
            Val::I64 => "i64",
            Val::F64 => "double",
            Val::I8 => "i8",
            Val::I32 => "i32",
            Val::Ptr => "ptr",
            Val::Void => "void",
        }
    }

    fn align(self) -> u32 {
        match self {
            Val::I64 | Val::F64 | Val::Ptr => 8,
            Val::I32 => 4,
            Val::I8 => 1,
            Val::Void => 1,
        }
    }
}

fn unsupported(construct: &'static str, fn_id: Option<MirFunctionId>) -> CodegenError {
    CodegenError::new(fn_id, CodegenErrorKind::UnsupportedConstruct { construct })
}

fn val_of_type(types: &TypeInterner, ty: TypeId) -> Result<Val, CodegenError> {
    Ok(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Int) => Val::I64,
        TypeKind::Primitive(PrimitiveType::Float) => Val::F64,
        TypeKind::Primitive(PrimitiveType::Bool) => Val::I8,
        TypeKind::Primitive(PrimitiveType::Char) => Val::I32,
        TypeKind::Primitive(PrimitiveType::String) => Val::Ptr,
        TypeKind::Primitive(PrimitiveType::Void) => Val::I64,
        TypeKind::Primitive(PrimitiveType::StrSlice)
        | TypeKind::Primitive(PrimitiveType::VectorI64x2) => {
            return Err(unsupported("SIMD/slice scalar", None));
        }
        // The remaining primitives are the six integer types; the native ABI
        // carries every integer as i64.
        TypeKind::Primitive(_) => Val::I64,
        _ => return Err(unsupported("unsupported scalar type", None)),
    })
}

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
            }
            Ok(())
        }
        Rvalue::Builtin { builtin, arguments } => {
            let name = builtin.descriptor().name;
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
        Rvalue::Call { callee, arguments } => {
            // A direct call's callee is a function operand (supported). An
            // indirect call (function value) is a copy of a function-typed
            // local, which is the 5E2 function-value surface.
            match callee {
                Operand::Function(_) => {}
                Operand::Copy(_) | Operand::Constant(_) => {
                    return Err(unsupported("function value", Some(fn_id)));
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
        Operand::Function(_) => Err(unsupported("function value", Some(fn_id))),
    }
}

/// The integer value of an operand, if it is an integer/bool/char constant.
/// This clang rejects width conversions of constexprs, so constants are
/// widened/narrowed in Rust at the conversion site.
fn const_int_value(operand: &Operand) -> Option<i64> {
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
        Operand::Function(_) => Err(unsupported("function value", Some(fn_id))),
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

/// Escape a Rust string into an LLVM `c"..."` literal body (including the
/// terminating NUL).
fn llvm_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for byte in text.bytes() {
        match byte {
            b'\n' => out.push_str("\\0A"),
            b'\t' => out.push_str("\\09"),
            b'\r' => out.push_str("\\0D"),
            b'"' => out.push_str("\\22"),
            b'\\' => out.push_str("\\5C"),
            b'%' => out.push_str("\\25"),
            0x20..=0x7e => out.push(byte as char),
            other => out.push_str(&format!("\\{other:02X}")),
        }
    }
    out.push_str("\\00");
    out
}

/// Emit the full module text.
pub(crate) fn lower_module(program: &MirProgram, types: &TypeInterner, plan: &Plan) -> String {
    let mut text = String::new();
    let _ = std::fmt::Write::write_str(
        &mut text,
        "target triple = \"x86_64-unknown-linux-gnu\"\n\n",
    );

    // String globals (deterministic: content order, per the plan).
    for (content, index) in &plan.string_of {
        let bytes = content.as_bytes();
        let _ = std::fmt::Write::write_fmt(
            &mut text,
            format_args!(
                "@str_{index} = private unnamed_addr constant [{} x i8] c\"{}\", align 1\n",
                bytes.len() + 1,
                llvm_string(content)
            ),
        );
    }
    text.push('\n');

    // Builtin declarations.
    for &symbol in &plan.builtins {
        match symbol {
            "lpp_print_str" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare void @lpp_print_str(ptr)\n");
            }
            "lpp_print_int" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare void @lpp_print_int(i64)\n");
            }
            "lpp_print_bool" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare void @lpp_print_bool(i8)\n");
            }
            "lpp_print_float" => {
                let _ = std::fmt::Write::write_str(
                    &mut text,
                    "declare void @lpp_print_float(double)\n",
                );
            }
            "lpp_write_str" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare void @lpp_write_str(ptr)\n");
            }
            "lpp_str_len" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare i64 @lpp_str_len(ptr)\n");
            }
            "lpp_str_eq" => {
                let _ =
                    std::fmt::Write::write_str(&mut text, "declare i64 @lpp_str_eq(ptr, ptr)\n");
            }
            "lpp_str_concat" => {
                let _ = std::fmt::Write::write_str(
                    &mut text,
                    "declare ptr @lpp_str_concat(ptr, ptr)\n",
                );
            }
            _ => {}
        }
    }
    text.push('\n');

    // Functions (MirFunctionId order).
    for (fn_id, function) in program.functions() {
        emit_function(program, types, plan, fn_id, function, &mut text);
        text.push('\n');
    }
    text
}

/// Emit one function definition (non-SSA, alloca-based).
fn emit_function(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
    function: &MirFunction,
    text: &mut String,
) {
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
                    )
                    .expect("checked in plan");
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
                    )
                    .expect("checked in plan");
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
        )
        .expect("checked in plan");
    }

    let _ = std::fmt::Write::write_str(text, &body);
    let _ = std::fmt::Write::write_str(text, "}\n");
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
                (Val::Ptr, format!("@str_{index}"))
            }
        }),
        Operand::Function(_) => Err(unsupported("function value", Some(fn_id))),
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
    let Operand::Function(target) = callee else {
        return Err(unsupported("indirect call", Some(fn_id)));
    };
    let target_fn = program.function(target).expect("callee exists");
    let ret = if matches!(
        types.kind(target_fn.return_type),
        TypeKind::Primitive(PrimitiveType::Void)
    ) {
        Val::Void
    } else {
        val_of_type(types, target_fn.return_type)?
    };
    let name = &plan.symbols[&target];
    let operands = program.operands(arguments).to_vec();
    let mut parts = Vec::with_capacity(operands.len());
    for operand in &operands {
        let (val, expr) = emit_operand(program, types, plan, fn_id, operand, slot_of, ssa, out)?;
        parts.push(format!("{} {expr}", val.ty()));
    }
    if ret == Val::Void {
        // A void call has no result to bind.
        push(out, &format!("call void @{name}({})", parts.join(", ")));
        Ok((Val::Void, String::new()))
    } else {
        let v = fresh(ssa);
        push(
            out,
            &format!("{v} = call {} @{name}({})", ret.ty(), parts.join(", ")),
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
    let name = builtin.descriptor().name;
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
        "str_len" => push(out, &format!("{v} = call i64 @lpp_str_len(ptr {aexpr})")),
        _ => return Err(unsupported("unported builtin", Some(fn_id))),
    }
    // Void builtins return an i64 placeholder (the rvalue is discarded).
    if name == "str_len" {
        return Ok((Val::I64, v));
    }
    Ok((Val::I64, v))
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
            debug_assert_eq!(val, Val::I8);
            // LLVM branches on `i1`; the bool local is `i8`.
            let v = fresh(ssa);
            push(out, &format!("{v} = trunc i8 {expr} to i1"));
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
                push(out, "ret void");
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
