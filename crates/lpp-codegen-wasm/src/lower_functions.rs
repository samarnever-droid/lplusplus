use super::*;

pub(super) fn is_void(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(types.kind(ty), TypeKind::Primitive(PrimitiveType::Void))
}

pub(super) fn local_val(types: &TypeInterner, ty: TypeId) -> Result<Val, CodegenError> {
    Ok(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Void) => Val::I64,
        TypeKind::Primitive(PrimitiveType::Int) => Val::I64,
        TypeKind::Primitive(PrimitiveType::Float) => Val::F64,
        TypeKind::Primitive(PrimitiveType::Bool) => Val::I32,
        TypeKind::Primitive(PrimitiveType::Char) => Val::I32,
        TypeKind::Primitive(PrimitiveType::String) => Val::I32,
        TypeKind::Primitive(PrimitiveType::StrSlice) => Val::I32,
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => {
            return Err(unsupported(
                "SIMD scalars are deferred on wasm32 (5D2b)",
                None,
            ));
        }
        // The remaining primitives are the six integer types; the wasm ABI
        // carries every integer as i64.
        TypeKind::Primitive(_) => Val::I64,
        // 5D2b: a closure's capsule pointer (16-byte ARC node).
        TypeKind::Function { .. } => Val::I32,
        // 5D2b slice 3: a task handle (32-byte ARC node).
        TypeKind::Task(_) => Val::I32,
        TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Map { .. }
        | TypeKind::Tuple(_)
        | TypeKind::Nominal { .. } => Val::I32,
        other => return Err(unsupported(type_desc(&other), None)),
    })
}

pub(super) fn operand_class(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
) -> Result<Class, CodegenError> {
    match operand {
        Operand::Copy(local) => {
            let ty = program
                .local(*local)
                .ok_or_else(|| {
                    CodegenError::new(
                        None,
                        CodegenErrorKind::ObjectEmissionFailed("unknown local".to_string()),
                    )
                })?
                .ty;
            class_of_type(types, ty)
        }
        Operand::Constant(c) => Ok(match c {
            Constant::Integer(_) => Class::I64,
            Constant::FloatBits(_) => Class::F64,
            Constant::Bool(_) => Class::Bool,
            Constant::Character { .. } => Class::Char,
            Constant::String { .. } => Class::Str,
        }),
        // 5D2b slice 2: a bare function value is a 16-byte capsule
        // ARC node (a heap pointer).
        Operand::Function(_) => Ok(Class::Ptr),
    }
}

pub(super) fn class_of_type(types: &TypeInterner, ty: TypeId) -> Result<Class, CodegenError> {
    Ok(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Int) => Class::I64,
        TypeKind::Primitive(PrimitiveType::Float) => Class::F64,
        TypeKind::Primitive(PrimitiveType::Bool) => Class::Bool,
        TypeKind::Primitive(PrimitiveType::Char) => Class::Char,
        TypeKind::Primitive(PrimitiveType::String) => Class::Str,
        TypeKind::Primitive(PrimitiveType::Void) => return Err(unsupported("void operand", None)),
        TypeKind::Primitive(PrimitiveType::StrSlice) => Class::Ptr,
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => {
            return Err(unsupported(
                "SIMD scalars are deferred on wasm32 (5D2b)",
                None,
            ));
        }
        // The remaining primitives are the six integer types; the wasm ABI
        // carries every integer as i64.
        TypeKind::Primitive(_) => Class::I64,
        // Function values are 16-byte capsule ARC nodes (5D2b); task
        // handles are 32-byte ARC nodes (slice 3).
        TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Map { .. }
        | TypeKind::Tuple(_)
        | TypeKind::Nominal { .. }
        | TypeKind::Function { .. }
        | TypeKind::Task(_) => Class::Ptr,
        other => return Err(unsupported(type_desc(&other), None)),
    })
}

/// Load a field/element slot of `ty` at `offset` (wasm width per
/// class: i64 for Int, f64 for Float, i32 for the rest).
pub(super) fn load_by_type(fb: &mut FB, types: &TypeInterner, offset: u32, ty: TypeId) {
    match class_of_type(types, ty).unwrap() {
        Class::I64 => fb.load64(offset),
        Class::F64 => fb.loadf64(offset),
        _ => fb.load32(offset),
    };
}

/// Store a value of `ty` at `offset` (the value is already on the
/// stack in its natural class width).
pub(super) fn store_by_type(fb: &mut FB, types: &TypeInterner, offset: u32, ty: TypeId) {
    match class_of_type(types, ty).unwrap() {
        Class::I64 => fb.store64(offset),
        Class::F64 => fb.storef64(offset),
        _ => fb.store32(offset),
    };
}

/// Zero-extend an i32-class value (Bool/Char) to the i64 list slot
/// width (Int values are already i64).
pub(super) fn widen_to_i64(fb: &mut FB, types: &TypeInterner, element: TypeId) {
    if class_of_type(types, element).unwrap() != Class::I64 {
        fb.op(op::I64_EXTEND_I32_U);
    }
}

pub(super) fn type_desc(kind: &TypeKind) -> &'static str {
    match kind {
        TypeKind::List(_) => "list",
        TypeKind::Nominal { .. } => "struct",
        TypeKind::Slice(_) => "slice",
        TypeKind::Tuple(_) => "tuple",
        TypeKind::Function { .. } => "function value",
        TypeKind::Task(_) => "task",
        TypeKind::Map { .. } => "map",
        TypeKind::Primitive(PrimitiveType::StrSlice) => "str slice",
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => "simd vector",
        TypeKind::Primitive(PrimitiveType::Void) => "void",
        _ => "type",
    }
}

pub(super) fn unsupported(construct: &'static str, fn_id: Option<MirFunctionId>) -> CodegenError {
    CodegenError::new(fn_id, CodegenErrorKind::UnsupportedConstruct { construct })
}

pub(super) fn unrepresentable(
    builtin: BuiltinId,
    reason: &'static str,
    fn_id: MirFunctionId,
) -> CodegenError {
    CodegenError::new(
        Some(fn_id),
        CodegenErrorKind::UnrepresentableBuiltin { builtin, reason },
    )
}

// ---------------------------------------------------------------------------
// String pool.
// ---------------------------------------------------------------------------

/// Register a signature in the type table, returning its index.
pub(super) fn reg_type(
    map: &mut HashMap<(Vec<Val>, Vec<Val>), u32>,
    list: &mut Vec<(Vec<Val>, Vec<Val>)>,
    params: &[Val],
    results: &[Val],
) -> u32 {
    let key = (params.to_vec(), results.to_vec());
    if let Some(&idx) = map.get(&key) {
        return idx;
    }
    let idx = list.len() as u32;
    map.insert(key.clone(), idx);
    list.push(key);
    idx
}

pub(super) fn build_pool(strings: &BTreeSet<String>) -> (HashMap<String, u32>, Vec<u8>) {
    let mut string_ptr = HashMap::new();
    let mut pool = Vec::new();
    let mut ptr = POOL_START;
    for content in strings {
        let base = ptr;
        string_ptr.insert(content.clone(), base + STR_HEADER);
        pool.extend_from_slice(&IMMORTAL_RC.to_le_bytes());
        pool.extend_from_slice(&0i64.to_le_bytes());
        pool.extend_from_slice(&ARC_MAGIC.to_le_bytes());
        pool.extend_from_slice(&(content.len() as i32).to_le_bytes());
        pool.extend_from_slice(content.as_bytes());
        let entry = STR_HEADER as usize + 4 + content.len();
        let padded = entry.div_ceil(8) * 8;
        pool.resize(pool.len() + (padded - entry), 0);
        ptr = base + padded as u32;
    }
    (string_ptr, pool)
}

// ---------------------------------------------------------------------------
// User-function lowering (dispatch loop).
// ---------------------------------------------------------------------------

/// The lowered artifacts of one user function: the export name, the
/// parameter value classes, the result value classes, the function's extra
/// value locals, and the function body bytes.
type FnLowering = (Option<String>, Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>);

pub(super) fn lower_user_function(
    env: &Env<'_>,
    fn_id: MirFunctionId,
) -> Result<FnLowering, CodegenError> {
    let function = env.program.function(fn_id).unwrap();
    let name = function
        .name
        .and_then(|s| env.names.resolve(s.raw()))
        .map(str::to_string);
    let all_locals = env.program.function_locals(function);
    let params = env.program.function_parameters(function);
    let is_closure = matches!(function.kind, lpp_mir::MirFunctionKind::Closure);
    // The closure frame's capture slots: the `function_parameters`
    // entries of `Capture` kind (they are also value locals — the
    // body reads and writes them). The remaining parameters are the
    // user parameters.
    let capture_frame: Vec<MirLocalId> = params
        .iter()
        .copied()
        .filter(|&p| {
            matches!(
                env.program.local(p).unwrap().kind,
                lpp_mir::MirLocalKind::Capture
            )
        })
        .collect();
    let user_params: Vec<MirLocalId> = params
        .iter()
        .copied()
        .filter(|p| !capture_frame.contains(p))
        .collect();

    let mut local_index: HashMap<MirLocalId, u32> = HashMap::new();
    // Every wasm function carries the env pointer as local 0 (the
    // 5D2b dispatch ABI): closures read their capture slots from it;
    // plain functions receive NULL and ignore it. This keeps the
    // dispatch table and the call_indirect signatures uniform (a
    // function value may dispatch to any function of its type).
    let mut param_vals: Vec<Val> = Vec::new();
    param_vals.push(Val::I32); // env
    for &p in &user_params {
        param_vals.push(local_val(env.types, env.program.local(p).unwrap().ty)?);
    }
    let env_offset: u32 = 1;
    for (i, &p) in user_params.iter().enumerate() {
        local_index.insert(p, env_offset + i as u32);
    }
    let p = param_vals.len() as u32;
    let mut fb = FB::new(p);
    let mut np = 0u32;
    for &l in all_locals {
        if let std::collections::hash_map::Entry::Vacant(e) = local_index.entry(l) {
            e.insert(p + np);
            let ty = env.program.local(l).unwrap().ty;
            fb.extras.push(local_val(env.types, ty)?);
            np += 1;
        }
    }
    let cur_local = p + np;
    fb.extras.push(Val::I32); // `current`

    if is_closure {
        // Entry: load the capture slots (views, no retain) from the env.
        for (i, &slot) in capture_frame.iter().enumerate() {
            let index = local_index[&slot];
            let ty = env.program.local(slot).unwrap().ty;
            fb.g(0);
            match local_val(env.types, ty)? {
                Val::I32 => fb.load32(8 * i as u32),
                Val::I64 => fb.load64(8 * i as u32),
                Val::F64 => fb.loadf64(8 * i as u32),
            };
            fb.s(index);
        }
    }

    let (order, positions) = block_layout(env.program, function);
    let entry_pos = positions[&function.entry];

    let mut capture_slot = HashMap::new();
    for (i, &c) in capture_frame.iter().enumerate() {
        capture_slot.insert(c, i as u32);
    }
    let lower = FnLower {
        env,
        local_index,
        cur_local,
        fn_id,
        captures: capture_slot,
    };

    // current = entry
    fb.i32c(entry_pos as i64);
    fb.s(cur_local);
    fb.loop_();
    for (pos, block_id) in order {
        let block = env.program.block(block_id).unwrap();
        fb.g(cur_local);
        fb.i32c(pos as i64);
        fb.op(op::I32_EQ);
        fb.if_();
        for &instr in env.program.block_instructions(block) {
            let instruction = env.program.instruction(instr).unwrap();
            match &instruction.kind {
                InstructionKind::Assign { target, value } => {
                    lower.emit_assign(&mut fb, *target, value)?;
                }
                InstructionKind::Store { place, value } => {
                    lower.emit_store(&mut fb, *place, value)?;
                }
            }
        }
        lower.emit_terminator(&mut fb, &block.terminator, &positions)?;
        fb.end();
    }
    // A wasm `loop` does not auto-repeat: its `end` falls through. Re-dispatch
    // explicitly by branching back to the loop start (depth 0) after the last
    // guard.
    fb.br(0);
    fb.end(); // end loop
    // The dispatch loop only exits through `return`/`unreachable` in a block,
    // so this point is unreachable; state it explicitly so the function's
    // result requirement is satisfied for value-returning functions.
    fb.op(op::UNREACHABLE);

    let results = if is_void(env.types, function.return_type) {
        vec![]
    } else {
        vec![local_val(env.types, function.return_type)?]
    };
    Ok((name, param_vals, results, fb.extras, fb.body))
}

// ---------------------------------------------------------------------------
// Block layout: reverse post-order (deterministic; any order is correct for
// the dispatch loop, RPO just keeps forward edges local).
// ---------------------------------------------------------------------------
