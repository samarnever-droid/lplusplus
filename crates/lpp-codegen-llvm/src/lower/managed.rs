use std::fmt::Write as _;

use lpp_codegen_api::CodegenError;
use lpp_codegen_api::layout::tuple_layout;
use lpp_mir::{
    MirAggregateId, MirAggregateKind, MirFunctionId, MirFunctionKind, MirLocalKind, MirProgram,
};
use lpp_types::{PrimitiveType, TypeId, TypeInterner, TypeKind};

use super::plan::Plan;
use super::{
    IMP_ARC_COLLECT_CYCLES, IMP_ARC_RELEASE, IMP_ARC_RETAIN, IMP_PGO_FLUSH, IMP_TASK_AWAIT,
    IMP_TASK_DESTROY, IMP_TASK_NEW, IMP_TRACE_VISIT, IMP_TUPLE_ALLOC, Val, is_managed,
    is_void_type, llvm_sym, user_parameters, val_of_type,
};

pub(crate) fn emit_destructors_and_thunks(
    out: &mut String,
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    pgo_fingerprint: Option<u64>,
) -> Result<(), CodegenError> {
    for &agg_id in &plan.destructors {
        emit_aggregate_destructor(out, program, types, plan, agg_id);
    }
    for &agg_id in &plan.traces {
        emit_aggregate_trace(out, program, types, plan, agg_id);
    }
    for &ty in &plan.tuple_destructors {
        emit_tuple_destructor(out, types, ty);
    }
    for &ty in &plan.tuple_traces {
        emit_tuple_trace(out, types, ty);
    }
    for &fn_id in &plan.closure_dtors {
        emit_closure_dtor(out, program, types, fn_id);
    }
    for &fn_id in &plan.closure_traces {
        emit_closure_trace(out, program, types, fn_id);
    }
    for &fn_id in &plan.task_thunks {
        emit_task_thunk(out, program, types, plan, fn_id)?;
    }
    for &fn_id in &plan.closure_thunks {
        emit_closure_thunk(out, program, types, plan, fn_id)?;
    }
    emit_main_wrapper(out, program, types, plan, pgo_fingerprint)?;
    Ok(())
}

fn emit_aggregate_destructor(
    out: &mut String,
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    agg_id: MirAggregateId,
) {
    let agg = program.aggregate(agg_id).expect("aggregate");
    let tag = match agg.kind {
        MirAggregateKind::Struct => 's',
        MirAggregateKind::Enum => 'e',
    };
    let name = format!("lpp_drop_{tag}{}", agg_id.raw());
    let layout = &plan.layouts[&agg_id];
    let _ = writeln!(out, "define void @{} (ptr %payload) {{", llvm_sym(&name));
    let _ = writeln!(out, "entry:");
    let mut seq = 0usize;
    let mut fresh = || {
        let s = format!("%t{seq}");
        seq += 1;
        s
    };

    match layout.kind {
        MirAggregateKind::Struct => {
            for slot in &layout.struct_fields {
                if is_managed(types, slot.ty) {
                    let gep = fresh();
                    let child = fresh();
                    let _ = writeln!(
                        out,
                        "  {gep} = getelementptr i8, ptr %payload, i64 {}",
                        slot.offset
                    );
                    let _ = writeln!(out, "  {child} = load ptr, ptr {gep}, align 8");
                    let _ = writeln!(out, "  call void @{IMP_ARC_RELEASE}(ptr {child})");
                }
            }
            let _ = writeln!(out, "  ret void");
        }
        MirAggregateKind::Enum => {
            let tag_val = fresh();
            let _ = writeln!(out, "  {tag_val} = load i64, ptr %payload, align 8");
            if layout.variant_fields.is_empty() {
                let _ = writeln!(out, "  ret void");
            } else {
                let _ = write!(out, "  switch i64 {tag_val}, label %v0 [");
                for ord in 1..layout.variant_fields.len() {
                    let _ = write!(out, " i64 {ord}, label %v{ord}");
                }
                let _ = writeln!(out, " ]");
                for (ord, slots) in layout.variant_fields.iter().enumerate() {
                    let _ = writeln!(out, "v{ord}:");
                    for slot in slots {
                        if is_managed(types, slot.ty) {
                            let gep = fresh();
                            let child = fresh();
                            let _ = writeln!(
                                out,
                                "  {gep} = getelementptr i8, ptr %payload, i64 {}",
                                slot.offset
                            );
                            let _ = writeln!(out, "  {child} = load ptr, ptr {gep}, align 8");
                            let _ = writeln!(out, "  call void @{IMP_ARC_RELEASE}(ptr {child})");
                        }
                    }
                    let _ = writeln!(out, "  ret void");
                }
            }
        }
    }
    let _ = writeln!(out, "}}");
}

fn emit_aggregate_trace(
    out: &mut String,
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    agg_id: MirAggregateId,
) {
    let agg = program.aggregate(agg_id).expect("aggregate");
    let tag = match agg.kind {
        MirAggregateKind::Struct => 's',
        MirAggregateKind::Enum => 'e',
    };
    let name = format!("lpp_trace_{tag}{}", agg_id.raw());
    let layout = &plan.layouts[&agg_id];
    let _ = writeln!(
        out,
        "define void @{} (ptr %payload, ptr %ctx) {{",
        llvm_sym(&name)
    );
    let _ = writeln!(out, "entry:");
    let mut seq = 0usize;
    let mut fresh = || {
        let s = format!("%t{seq}");
        seq += 1;
        s
    };

    match layout.kind {
        MirAggregateKind::Struct => {
            for slot in &layout.struct_fields {
                if is_managed(types, slot.ty) {
                    let gep = fresh();
                    let child = fresh();
                    let _ = writeln!(
                        out,
                        "  {gep} = getelementptr i8, ptr %payload, i64 {}",
                        slot.offset
                    );
                    let _ = writeln!(out, "  {child} = load ptr, ptr {gep}, align 8");
                    let _ = writeln!(out, "  call void @{IMP_TRACE_VISIT}(ptr %ctx, ptr {child})");
                }
            }
            let _ = writeln!(out, "  ret void");
        }
        MirAggregateKind::Enum => {
            let tag_val = fresh();
            let _ = writeln!(out, "  {tag_val} = load i64, ptr %payload, align 8");
            if layout.variant_fields.is_empty() {
                let _ = writeln!(out, "  ret void");
            } else {
                let _ = write!(out, "  switch i64 {tag_val}, label %v0 [");
                for ord in 1..layout.variant_fields.len() {
                    let _ = write!(out, " i64 {ord}, label %v{ord}");
                }
                let _ = writeln!(out, " ]");
                for (ord, slots) in layout.variant_fields.iter().enumerate() {
                    let _ = writeln!(out, "v{ord}:");
                    for slot in slots {
                        if is_managed(types, slot.ty) {
                            let gep = fresh();
                            let child = fresh();
                            let _ = writeln!(
                                out,
                                "  {gep} = getelementptr i8, ptr %payload, i64 {}",
                                slot.offset
                            );
                            let _ = writeln!(out, "  {child} = load ptr, ptr {gep}, align 8");
                            let _ = writeln!(
                                out,
                                "  call void @{IMP_TRACE_VISIT}(ptr %ctx, ptr {child})"
                            );
                        }
                    }
                    let _ = writeln!(out, "  ret void");
                }
            }
        }
    }
    let _ = writeln!(out, "}}");
}

fn emit_tuple_destructor(out: &mut String, types: &TypeInterner, ty: TypeId) {
    let name = format!("lpp_drop_t{}", ty.raw());
    let (slots, _) = tuple_layout(types, ty).expect("tuple layout");
    let _ = writeln!(out, "define void @{name}(ptr %payload) {{");
    let _ = writeln!(out, "entry:");
    for (idx, (offset, elem_ty)) in slots.iter().enumerate() {
        if is_managed(types, *elem_ty) {
            let _ = writeln!(
                out,
                "  %gep{idx} = getelementptr i8, ptr %payload, i64 {offset}"
            );
            let _ = writeln!(out, "  %child{idx} = load ptr, ptr %gep{idx}, align 8");
            let _ = writeln!(out, "  call void @{IMP_ARC_RELEASE}(ptr %child{idx})");
        }
    }
    let _ = writeln!(out, "  ret void");
    let _ = writeln!(out, "}}");
}

fn emit_tuple_trace(out: &mut String, types: &TypeInterner, ty: TypeId) {
    let name = format!("lpp_trace_t{}", ty.raw());
    let (slots, _) = tuple_layout(types, ty).expect("tuple layout");
    let _ = writeln!(out, "define void @{name}(ptr %payload, ptr %ctx) {{");
    let _ = writeln!(out, "entry:");
    for (idx, (offset, elem_ty)) in slots.iter().enumerate() {
        if is_managed(types, *elem_ty) {
            let _ = writeln!(
                out,
                "  %gep{idx} = getelementptr i8, ptr %payload, i64 {offset}"
            );
            let _ = writeln!(out, "  %child{idx} = load ptr, ptr %gep{idx}, align 8");
            let _ = writeln!(
                out,
                "  call void @{IMP_TRACE_VISIT}(ptr %ctx, ptr %child{idx})"
            );
        }
    }
    let _ = writeln!(out, "  ret void");
    let _ = writeln!(out, "}}");
}

fn emit_closure_dtor(
    out: &mut String,
    program: &MirProgram,
    types: &TypeInterner,
    fn_id: MirFunctionId,
) {
    let name = format!("lpp_drop_c{}", fn_id.raw());
    let function = program.function(fn_id).expect("closure");
    let captures: Vec<TypeId> = program
        .function_parameters(function)
        .iter()
        .filter(|l| {
            matches!(
                program.local(**l).expect("local").kind,
                MirLocalKind::Capture
            )
        })
        .map(|l| program.local(*l).expect("local").ty)
        .collect();
    let _ = writeln!(out, "define void @{name}(ptr %env) {{");
    let _ = writeln!(out, "entry:");
    for (i, ty) in captures.iter().enumerate() {
        if is_managed(types, *ty) {
            let off = 8 * i;
            let _ = writeln!(out, "  %gep{i} = getelementptr i8, ptr %env, i64 {off}");
            let _ = writeln!(out, "  %old{i} = load ptr, ptr %gep{i}, align 8");
            let _ = writeln!(out, "  call void @{IMP_ARC_RELEASE}(ptr %old{i})");
        }
    }
    let _ = writeln!(out, "  ret void");
    let _ = writeln!(out, "}}");
}

fn emit_closure_trace(
    out: &mut String,
    program: &MirProgram,
    types: &TypeInterner,
    fn_id: MirFunctionId,
) {
    let name = format!("lpp_trace_c{}", fn_id.raw());
    let function = program.function(fn_id).expect("closure");
    let captures: Vec<TypeId> = program
        .function_parameters(function)
        .iter()
        .filter(|l| {
            matches!(
                program.local(**l).expect("local").kind,
                MirLocalKind::Capture
            )
        })
        .map(|l| program.local(*l).expect("local").ty)
        .collect();
    let _ = writeln!(out, "define void @{name}(ptr %env, ptr %ctx) {{");
    let _ = writeln!(out, "entry:");
    for (i, ty) in captures.iter().enumerate() {
        if is_managed(types, *ty) {
            let off = 8 * i;
            let _ = writeln!(out, "  %gep{i} = getelementptr i8, ptr %env, i64 {off}");
            let _ = writeln!(out, "  %child{i} = load ptr, ptr %gep{i}, align 8");
            let _ = writeln!(
                out,
                "  call void @{IMP_TRACE_VISIT}(ptr %ctx, ptr %child{i})"
            );
        }
    }
    let _ = writeln!(out, "  ret void");
    let _ = writeln!(out, "}}");
}

fn emit_box_to_i64(
    out: &mut String,
    val_opt: Option<&str>,
    ty: TypeId,
    types: &TypeInterner,
    prefix: &str,
) -> String {
    let Some(val) = val_opt else {
        return "0".to_string();
    };
    if is_void_type(types, ty) {
        return "0".to_string();
    }
    let kind = types.kind(ty);
    match kind {
        TypeKind::Primitive(PrimitiveType::Float) => {
            let tmp = format!("%{prefix}_fbox");
            let _ = writeln!(out, "  {tmp} = bitcast double {val} to i64");
            tmp
        }
        TypeKind::Primitive(PrimitiveType::Bool) => {
            let tmp = format!("%{prefix}_bbox");
            let _ = writeln!(out, "  {tmp} = zext i8 {val} to i64");
            tmp
        }
        TypeKind::Primitive(
            PrimitiveType::U8 | PrimitiveType::U16 | PrimitiveType::U32 | PrimitiveType::Char,
        ) => {
            let src = match kind {
                TypeKind::Primitive(PrimitiveType::U8) => "i8",
                TypeKind::Primitive(PrimitiveType::U16) => "i16",
                _ => "i32",
            };
            let tmp = format!("%{prefix}_ubox");
            let _ = writeln!(out, "  {tmp} = zext {src} {val} to i64");
            tmp
        }
        TypeKind::Primitive(PrimitiveType::I8 | PrimitiveType::I16 | PrimitiveType::I32) => {
            let src = match kind {
                TypeKind::Primitive(PrimitiveType::I8) => "i8",
                TypeKind::Primitive(PrimitiveType::I16) => "i16",
                _ => "i32",
            };
            let tmp = format!("%{prefix}_sbox");
            let _ = writeln!(out, "  {tmp} = sext {src} {val} to i64");
            tmp
        }
        TypeKind::Primitive(
            PrimitiveType::Int | PrimitiveType::U64 | PrimitiveType::USize | PrimitiveType::ISize,
        ) => val.to_string(),
        _ => {
            // Pointer / managed handle -> ptrtoint i64
            let tmp = format!("%{prefix}_pbox");
            let _ = writeln!(out, "  {tmp} = ptrtoint ptr {val} to i64");
            tmp
        }
    }
}

pub(crate) fn emit_unbox_from_i64(
    out: &mut String,
    boxed: &str,
    ty: TypeId,
    types: &TypeInterner,
    fresh: &mut impl FnMut() -> String,
) -> Option<(Val, String)> {
    if is_void_type(types, ty) {
        return None;
    }
    let val = val_of_type(types, ty, MirFunctionId::from_raw(0)).ok()?;
    let res = match val {
        Val::I64 => boxed.to_string(),
        Val::I8 | Val::I16 | Val::I32 => {
            let r = fresh();
            let _ = writeln!(out, "  {r} = trunc i64 {boxed} to {}", val.llvm());
            r
        }
        Val::F64 => {
            let r = fresh();
            let _ = writeln!(out, "  {r} = bitcast i64 {boxed} to double");
            r
        }
        Val::Ptr => {
            let r = fresh();
            let _ = writeln!(out, "  {r} = inttoptr i64 {boxed} to ptr");
            r
        }
        Val::V2I64 | Val::Void => return None,
    };
    Some((val, res))
}

fn emit_task_thunk(
    out: &mut String,
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
) -> Result<(), CodegenError> {
    let function = program.function(fn_id).expect("fn");
    let thunk_name = format!("__lpp_task_thunk{}", fn_id.raw());
    let callee_name = llvm_sym(&plan.export_names[&fn_id]);
    let ret_val = if is_void_type(types, function.return_type) {
        None
    } else {
        Some(val_of_type(types, function.return_type, fn_id)?)
    };

    let _ = writeln!(out, "define i64 @{thunk_name}(ptr %env) {{");
    let _ = writeln!(out, "entry:");

    let mut arg_list = Vec::new();
    for (i, &param) in user_parameters(program, function).iter().enumerate() {
        let ty = program.local(param).expect("local").ty;
        let p_val = val_of_type(types, ty, fn_id)?;
        let off = 16 + 8 * i;
        let _ = writeln!(out, "  %gep{i} = getelementptr i8, ptr %env, i64 {off}");
        let raw = format!("%raw{i}");
        let _ = writeln!(out, "  {raw} = load i64, ptr %gep{i}, align 8");
        let loaded = match p_val {
            Val::I64 => raw,
            Val::I8 | Val::I16 | Val::I32 => {
                let c = format!("%arg{i}");
                let _ = writeln!(out, "  {c} = trunc i64 {raw} to {}", p_val.llvm());
                c
            }
            Val::F64 => {
                let c = format!("%arg{i}");
                let _ = writeln!(out, "  {c} = bitcast i64 {raw} to double");
                c
            }
            Val::Ptr => {
                let c = format!("%arg{i}");
                let _ = writeln!(out, "  {c} = inttoptr i64 {raw} to ptr");
                if is_managed(types, ty) {
                    let _ = writeln!(out, "  call void @{IMP_ARC_RETAIN}(ptr {c})");
                }
                c
            }
            Val::V2I64 | Val::Void => raw,
        };
        arg_list.push(format!("{} {loaded}", p_val.llvm()));
    }

    let args_str = arg_list.join(", ");
    let ret_name = if let Some(rv) = ret_val {
        let _ = writeln!(
            out,
            "  %res = call {} @{callee_name}({args_str})",
            rv.llvm()
        );
        Some("%res")
    } else {
        let _ = writeln!(out, "  call void @{callee_name}({args_str})");
        None
    };

    let boxed = emit_box_to_i64(out, ret_name, function.return_type, types, "tret");
    let _ = writeln!(out, "  ret i64 {boxed}");
    let _ = writeln!(out, "}}");
    Ok(())
}

fn emit_closure_thunk(
    out: &mut String,
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    fn_id: MirFunctionId,
) -> Result<(), CodegenError> {
    let function = program.function(fn_id).expect("fn");
    let thunk_name = format!("__lpp_closure_thunk{}", fn_id.raw());
    let callee_name = llvm_sym(&plan.export_names[&fn_id]);
    let ret_val = if is_void_type(types, function.return_type) {
        None
    } else {
        Some(val_of_type(types, function.return_type, fn_id)?)
    };

    let _ = writeln!(out, "define i64 @{thunk_name}(ptr %wrapper) {{");
    let _ = writeln!(out, "entry:");
    let _ = writeln!(out, "  %gep = getelementptr i8, ptr %wrapper, i64 16");
    let _ = writeln!(out, "  %cenv = load ptr, ptr %gep, align 8");
    let ret_name = if let Some(rv) = ret_val {
        let _ = writeln!(out, "  %res = call {} @{callee_name}(ptr %cenv)", rv.llvm());
        Some("%res")
    } else {
        let _ = writeln!(out, "  call void @{callee_name}(ptr %cenv)");
        None
    };
    let boxed = emit_box_to_i64(out, ret_name, function.return_type, types, "cret");
    let _ = writeln!(out, "  ret i64 {boxed}");
    let _ = writeln!(out, "}}");
    Ok(())
}

fn emit_main_wrapper(
    out: &mut String,
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
    pgo_fingerprint: Option<u64>,
) -> Result<(), CodegenError> {
    let Some((main_fn_id, _)) = plan.export_names.iter().find(|(_, n)| *n == "lpp_main") else {
        return Ok(());
    };
    let main_fn = program.function(*main_fn_id).expect("main fn");
    let ret_ty = if is_void_type(types, main_fn.return_type) {
        Val::Void
    } else {
        val_of_type(types, main_fn.return_type, *main_fn_id)?
    };

    let _ = writeln!(out, "define i32 @main() {{");
    let _ = writeln!(out, "entry:");
    if matches!(main_fn.kind, MirFunctionKind::Async) {
        let thunk = format!("__lpp_task_thunk{}", main_fn_id.raw());
        let _ = writeln!(
            out,
            "  %env = call ptr @{IMP_TUPLE_ALLOC}(i64 16, i64 0, i64 0)"
        );
        let managed = if is_managed(types, main_fn.return_type) {
            1
        } else {
            0
        };
        let _ = writeln!(
            out,
            "  %task = call ptr @{IMP_TASK_NEW}(ptr @{thunk}, ptr %env, i64 {managed})"
        );
        let _ = writeln!(out, "  %ignored = call i64 @{IMP_TASK_AWAIT}(ptr %task)");
        let _ = writeln!(out, "  call void @{IMP_TASK_DESTROY}(ptr %task)");
    } else if ret_ty == Val::Void {
        let _ = writeln!(out, "  call void @lpp_main()");
    } else {
        let _ = writeln!(out, "  %ignored = call {} @lpp_main()", ret_ty.llvm());
    }

    if !plan.shared_types.is_empty() {
        let _ = writeln!(out, "  call void @{IMP_ARC_COLLECT_CYCLES}()");
    }
    if let Some(fp) = pgo_fingerprint {
        let _ = writeln!(out, "  call void @{IMP_PGO_FLUSH}(i64 {})", fp as i64);
    }
    let _ = writeln!(out, "  ret i32 0");
    let _ = writeln!(out, "}}");
    Ok(())
}
