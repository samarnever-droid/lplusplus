use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use lpp_codegen_api::{CodegenError, CodegenErrorKind, CodegenOptions, NameResolver, Target};
use lpp_mir::{MirFunction, MirFunctionId, MirLocalId, MirLocalKind, MirProgram};
use lpp_types::{PrimitiveType, TypeId, TypeInterner, TypeKind};

mod debug;
mod function;
mod intrinsics;
mod managed;
mod plan;
mod strings;

use crate::llvm_target_triple;
use debug::DebugMetadata;
use plan::pre_scan;
use strings::llvm_arc_string;

pub(crate) const IMP_PGO_HIT: &str = "lpp_pgo_hit";
pub(crate) const IMP_PGO_FLUSH: &str = "lpp_pgo_flush";
pub(crate) const IMP_PRINT_STR: &str = "lpp_print_str";
pub(crate) const IMP_PRINT_INT: &str = "lpp_print_int";
pub(crate) const IMP_PRINT_FLOAT: &str = "lpp_print_float";
pub(crate) const IMP_PRINT_BOOL: &str = "lpp_print_bool";
pub(crate) const IMP_FMOD: &str = "lpp_fmod";
pub(crate) const IMP_ARC_ALLOC: &str = "lpp_arc_alloc_with_destructor";
pub(crate) const IMP_ARC_RETAIN: &str = "lpp_arc_retain";
pub(crate) const IMP_ARC_RELEASE: &str = "lpp_arc_release";
pub(crate) const IMP_ARC_MARK_COLLECTIBLE: &str = "lpp_arc_mark_collectible";
pub(crate) const IMP_TRACE_VISIT: &str = "lpp_trace_visit";
pub(crate) const IMP_ARC_COLLECT_CYCLES: &str = "lpp_arc_collect_cycles";
pub(crate) const IMP_PANIC_DIV_ZERO: &str = "lpp_panic_div_zero";
pub(crate) const IMP_LIST_NEW: &str = "lpp_list_new";
pub(crate) const IMP_LIST_NEW_ARC: &str = "lpp_list_new_arc";
pub(crate) const IMP_LIST_PUSH: &str = "lpp_list_push";
pub(crate) const IMP_LIST_PUSH_ARC: &str = "lpp_list_push_arc";
pub(crate) const IMP_LIST_PUSH_FLOAT: &str = "lpp_list_push_float";
pub(crate) const IMP_LIST_PUSH_BOOL: &str = "lpp_list_push_bool";
pub(crate) const IMP_LIST_GET: &str = "lpp_list_get";
pub(crate) const IMP_LIST_GET_ARC: &str = "lpp_list_get_arc";
pub(crate) const IMP_LIST_GET_FLOAT: &str = "lpp_list_get_float";
pub(crate) const IMP_LIST_GET_BOOL: &str = "lpp_list_get_bool";
pub(crate) const IMP_LIST_SET: &str = "lpp_list_set";
pub(crate) const IMP_LIST_SET_ARC: &str = "lpp_list_set_arc";
pub(crate) const IMP_LIST_SET_FLOAT: &str = "lpp_list_set_float";
pub(crate) const IMP_LIST_SET_BOOL: &str = "lpp_list_set_bool";
pub(crate) const IMP_LIST_LEN: &str = "lpp_list_len";
pub(crate) const IMP_LIST_TRACE: &str = "lpp_list_trace";
pub(crate) const IMP_TUPLE_ALLOC: &str = "lpp_tuple_alloc";
pub(crate) const IMP_TUPLE_TRACE: &str = "lpp_tuple_trace";
pub(crate) const IMP_CLOSURE_DESTROY: &str = "lpp_closure_destroy";
pub(crate) const IMP_CLOSURE_TRACE: &str = "lpp_closure_trace";
pub(crate) const IMP_TASK_NEW: &str = "lpp_task_new";
pub(crate) const IMP_TASK_POLL: &str = "lpp_task_poll";
pub(crate) const IMP_TASK_AWAIT: &str = "lpp_task_await";
pub(crate) const IMP_TASK_DESTROY: &str = "lpp_task_destroy";
pub(crate) const IMP_TASK_TRACE: &str = "lpp_task_trace";
pub(crate) const IMP_STR_CONCAT: &str = "lpp_str_concat";
pub(crate) const IMP_STR_EQ: &str = "lpp_str_eq";
pub(crate) const IMP_EMPTY_STR: &str = "lpp_empty_str";
pub(crate) const IMP_INT_TO_STR: &str = "lpp_int_to_str";
pub(crate) const IMP_FLOAT_TO_STR: &str = "lpp_float_to_str";
pub(crate) const IMP_BOOL_TO_STR: &str = "lpp_bool_to_str";
pub(crate) const IMP_VEC_CHECKSUM: &str = "lpp_vec_i64_checksum";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Val {
    I8,
    I16,
    I32,
    I64,
    F64,
    Ptr,
    V2I64,
    Void,
}

impl Val {
    pub fn llvm(self) -> &'static str {
        match self {
            Val::I8 => "i8",
            Val::I16 => "i16",
            Val::I32 => "i32",
            Val::I64 => "i64",
            Val::F64 => "double",
            Val::Ptr => "ptr",
            Val::V2I64 => "<2 x i64>",
            Val::Void => "void",
        }
    }

    pub fn align(self) -> u32 {
        match self {
            Val::I8 | Val::Void => 1,
            Val::I16 => 2,
            Val::I32 => 4,
            Val::I64 | Val::F64 | Val::Ptr => 8,
            Val::V2I64 => 16,
        }
    }

    pub fn is_int(self) -> bool {
        matches!(self, Val::I8 | Val::I16 | Val::I32 | Val::I64)
    }

    pub fn bit_width(self) -> u32 {
        match self {
            Val::I8 => 8,
            Val::I16 => 16,
            Val::I32 => 32,
            Val::I64 | Val::Ptr => 64,
            Val::F64 => 64,
            Val::V2I64 => 128,
            Val::Void => 0,
        }
    }

    pub fn zero_const(self) -> &'static str {
        match self {
            Val::I8 | Val::I16 | Val::I32 | Val::I64 => "0",
            Val::F64 => "0.0",
            Val::Ptr => "null",
            Val::V2I64 => "zeroinitializer",
            Val::Void => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ElementClass {
    Managed,
    Scalar,
    Float,
    Bool,
}

pub(crate) fn element_class(types: &TypeInterner, element: TypeId) -> ElementClass {
    if is_managed(types, element) {
        return ElementClass::Managed;
    }
    match types.kind(element) {
        TypeKind::Primitive(PrimitiveType::Float) => ElementClass::Float,
        TypeKind::Primitive(PrimitiveType::Bool) => ElementClass::Bool,
        _ => ElementClass::Scalar,
    }
}

pub(crate) fn is_void_type(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(types.kind(ty), TypeKind::Primitive(PrimitiveType::Void))
}

pub(crate) fn is_managed(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(
        types.kind(ty),
        TypeKind::Primitive(PrimitiveType::String)
            | TypeKind::List(_)
            | TypeKind::Nominal { .. }
            | TypeKind::Task(_)
            | TypeKind::Function { .. }
            | TypeKind::Tuple(_)
    )
}

pub(crate) fn is_signed_integer_kind(kind: Option<TypeKind>) -> bool {
    matches!(
        kind,
        Some(TypeKind::Primitive(
            PrimitiveType::Int
                | PrimitiveType::I8
                | PrimitiveType::I16
                | PrimitiveType::I32
                | PrimitiveType::ISize
        ))
    )
}

pub(crate) fn val_of_type(
    types: &TypeInterner,
    ty: TypeId,
    fn_id: MirFunctionId,
) -> Result<Val, CodegenError> {
    Ok(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Void) => Val::Void,
        TypeKind::Primitive(PrimitiveType::Bool | PrimitiveType::U8 | PrimitiveType::I8) => Val::I8,
        TypeKind::Primitive(PrimitiveType::U16 | PrimitiveType::I16) => Val::I16,
        TypeKind::Primitive(PrimitiveType::Char | PrimitiveType::U32 | PrimitiveType::I32) => {
            Val::I32
        }
        TypeKind::Primitive(
            PrimitiveType::Int | PrimitiveType::U64 | PrimitiveType::USize | PrimitiveType::ISize,
        ) => Val::I64,
        TypeKind::Primitive(PrimitiveType::Float) => Val::F64,
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => Val::V2I64,
        TypeKind::Primitive(
            PrimitiveType::String | PrimitiveType::StrSlice | PrimitiveType::RawPtr,
        )
        | TypeKind::Ptr(_)
        | TypeKind::List(_)
        | TypeKind::Map { .. }
        | TypeKind::Slice(_)
        | TypeKind::Nominal { .. }
        | TypeKind::Task(_)
        | TypeKind::Function { .. }
        | TypeKind::Tuple(_) => Val::Ptr,
        _ => return Err(unsupported(fn_id, "unsupported type in LLVM backend")),
    })
}

pub(crate) fn unsupported(function: MirFunctionId, construct: &'static str) -> CodegenError {
    CodegenError::new(
        Some(function),
        CodegenErrorKind::UnsupportedConstruct { construct },
    )
}

pub(crate) fn user_parameters(program: &MirProgram, function: &MirFunction) -> Vec<MirLocalId> {
    program
        .function_parameters(function)
        .iter()
        .copied()
        .filter(|l| !matches!(program.local(*l).expect("l").kind, MirLocalKind::Capture))
        .collect()
}

pub(crate) fn llvm_sym(name: &str) -> String {
    let clean = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'$'));
    if clean {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

pub(crate) struct LlvmIrOutput {
    pub ir: String,
    pub exported_symbols: BTreeSet<String>,
    pub imported_symbols: BTreeSet<String>,
    pub link_libraries: Vec<String>,
    pub entry: Option<String>,
}

/// Emit LLVM IR text targeting `Target::X86_64` (convenience helper for tests & inspection).
pub fn emit_llvm_ir(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn NameResolver,
) -> Result<String, CodegenError> {
    let options = CodegenOptions::new(Target::X86_64, names);
    Ok(emit_llvm_ir_with_options(program, types, &options)?.ir)
}

/// Emit complete LLVM IR module text along with export/import censuses.
pub(crate) fn emit_llvm_ir_with_options(
    program: &MirProgram,
    types: &TypeInterner,
    options: &CodegenOptions<'_>,
) -> Result<LlvmIrOutput, CodegenError> {
    let plan = pre_scan(
        program,
        types,
        options.names,
        options.pgo_fingerprint.is_some(),
    )?;

    let mut out = String::with_capacity(8192);
    let _ = writeln!(out, "source_filename = \"lpp_module.ll\"");
    let _ = writeln!(
        out,
        "target triple = \"{}\"",
        llvm_target_triple(options.target)
    );
    let _ = writeln!(out);

    let mut imported_symbols = BTreeSet::new();
    for (sym, sig) in &plan.imports {
        let params: Vec<&str> = sig.params.iter().map(|v| v.llvm()).collect();
        let ret = sig.result.map_or("void", |v| v.llvm());
        let _ = writeln!(
            out,
            "declare {ret} @{}({})",
            llvm_sym(sym),
            params.join(", ")
        );
        imported_symbols.insert(sym.clone());
    }
    for &ext_id in &plan.external_functions {
        let ext = program.external_function(ext_id).expect("ext");
        if imported_symbols.contains(&ext.name) {
            continue;
        }
        let TypeKind::Function { parameters, result } = types.kind(ext.ty) else {
            return Err(unsupported(MirFunctionId::from_raw(0), "ext non-fn"));
        };
        let mut p_strs = Vec::new();
        for &pt in types.list(parameters) {
            p_strs.push(val_of_type(types, pt, MirFunctionId::from_raw(0))?.llvm());
        }
        let ret_s = if is_void_type(types, result) {
            "void"
        } else {
            val_of_type(types, result, MirFunctionId::from_raw(0))?.llvm()
        };
        let _ = writeln!(
            out,
            "declare {ret_s} @{}({})",
            llvm_sym(&ext.name),
            p_strs.join(", ")
        );
        imported_symbols.insert(ext.name.clone());
    }

    // String globals (deduplicated per MirStringId)
    let mut string_ids = HashMap::new();
    for &str_id in &plan.used_strings {
        let text = program.string(str_id).expect("string");
        let idx = string_ids.len();
        string_ids.insert(str_id, idx);
        let len = 16 + text.len() + 1;
        let escaped = llvm_arc_string(text);
        let _ = writeln!(
            out,
            "@.str.{idx} = private unnamed_addr constant [{len} x i8] c\"{escaped}\", align 16"
        );
    }
    let _ = writeln!(out);

    let mut dbg = DebugMetadata::new(
        options.debug_info,
        options.debug_sources,
        options.target,
        options.opt_level,
    );

    let emit_ctx = function::EmitFunctionCtx {
        program,
        types,
        plan: &plan,
        string_ids: &string_ids,
        pgo_fingerprint: options.pgo_fingerprint,
        names: options.names,
    };
    for (mir_id, function) in program.functions() {
        function::emit_function(&mut out, &emit_ctx, mir_id, function, &mut dbg)?;
    }

    managed::emit_destructors_and_thunks(&mut out, program, types, &plan, options.pgo_fingerprint)?;
    dbg.emit_module_trailer(&mut out);

    let mut exported_symbols: BTreeSet<String> = plan.export_names.values().cloned().collect();
    for agg in &plan.destructors {
        let tag = match program.aggregate(*agg).expect("agg").kind {
            lpp_mir::MirAggregateKind::Struct => 's',
            lpp_mir::MirAggregateKind::Enum => 'e',
        };
        exported_symbols.insert(format!("lpp_drop_{tag}{}", agg.raw()));
    }
    for agg in &plan.traces {
        let tag = match program.aggregate(*agg).expect("agg").kind {
            lpp_mir::MirAggregateKind::Struct => 's',
            lpp_mir::MirAggregateKind::Enum => 'e',
        };
        exported_symbols.insert(format!("lpp_trace_{tag}{}", agg.raw()));
    }
    for ty in &plan.tuple_destructors {
        exported_symbols.insert(format!("lpp_drop_t{}", ty.raw()));
    }
    for ty in &plan.tuple_traces {
        exported_symbols.insert(format!("lpp_trace_t{}", ty.raw()));
    }
    for fn_id in &plan.closure_dtors {
        exported_symbols.insert(format!("lpp_drop_c{}", fn_id.raw()));
    }
    for fn_id in &plan.closure_traces {
        exported_symbols.insert(format!("lpp_trace_c{}", fn_id.raw()));
    }
    for fn_id in &plan.task_thunks {
        exported_symbols.insert(format!("__lpp_task_thunk{}", fn_id.raw()));
    }
    for fn_id in &plan.closure_thunks {
        exported_symbols.insert(format!("__lpp_closure_thunk{}", fn_id.raw()));
    }

    let entry = if exported_symbols.contains("lpp_main") {
        exported_symbols.insert("main".to_owned());
        Some("main".to_owned())
    } else if exported_symbols.contains("_start") {
        Some("_start".to_owned())
    } else {
        None
    };

    let mut link_libraries = Vec::new();
    for &ext_id in &plan.external_functions {
        let ext = program.external_function(ext_id).expect("ext");
        if imported_symbols.contains(&ext.name) {
            continue;
        }
        if let Some(lib) = ext.link_library.as_deref()
            && !link_libraries.iter().any(|existing| existing == lib)
        {
            link_libraries.push(lib.to_owned());
        }
    }

    Ok(LlvmIrOutput {
        ir: out,
        exported_symbols,
        imported_symbols,
        link_libraries,
        entry,
    })
}
