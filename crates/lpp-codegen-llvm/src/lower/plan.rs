use std::collections::{BTreeMap, BTreeSet, HashMap};

use lpp_codegen_api::layout::{AggregateIndex, AggregateLayout, aggregate_layout, tuple_layout};
use lpp_codegen_api::{CodegenError, CodegenErrorKind, NameResolver};
use lpp_mir::{
    Constant, InstructionKind, KernelIntrinsic, MirAggregateId, MirExternalFunctionId, MirFunction,
    MirFunctionId, MirFunctionKind, MirLocalId, MirLocalKind, MirProgram, MirStringId, Operand,
    PlaceProjection, Rvalue, Terminator,
};
use lpp_runtime_abi::generated::{AbiType, SemanticAbiType};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind};

use super::{
    ElementClass, IMP_ARC_ALLOC, IMP_ARC_COLLECT_CYCLES, IMP_ARC_MARK_COLLECTIBLE, IMP_ARC_RELEASE,
    IMP_ARC_RETAIN, IMP_BOOL_TO_STR, IMP_CLOSURE_DESTROY, IMP_CLOSURE_TRACE, IMP_EMPTY_STR,
    IMP_FLOAT_TO_STR, IMP_FMOD, IMP_INT_TO_STR, IMP_LIST_GET, IMP_LIST_GET_ARC, IMP_LIST_GET_BOOL,
    IMP_LIST_GET_FLOAT, IMP_LIST_LEN, IMP_LIST_NEW, IMP_LIST_NEW_ARC, IMP_LIST_PUSH,
    IMP_LIST_PUSH_ARC, IMP_LIST_PUSH_BOOL, IMP_LIST_PUSH_FLOAT, IMP_LIST_SET, IMP_LIST_SET_ARC,
    IMP_LIST_SET_BOOL, IMP_LIST_SET_FLOAT, IMP_LIST_TRACE, IMP_PANIC_DIV_ZERO, IMP_PGO_FLUSH,
    IMP_PGO_HIT, IMP_PRINT_BOOL, IMP_PRINT_FLOAT, IMP_PRINT_INT, IMP_PRINT_STR, IMP_STR_CONCAT,
    IMP_STR_EQ, IMP_TASK_AWAIT, IMP_TASK_DESTROY, IMP_TASK_NEW, IMP_TASK_POLL, IMP_TASK_TRACE,
    IMP_TRACE_VISIT, IMP_TUPLE_ALLOC, IMP_TUPLE_TRACE, IMP_VEC_CHECKSUM, Val, element_class,
    is_managed, is_void_type, unsupported, user_parameters, val_of_type,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportSig {
    pub params: Vec<Val>,
    pub result: Option<Val>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueClass {
    FunctionValue,
    Closure,
}

#[derive(Debug, Clone)]
pub(crate) enum PlaceStep {
    Field {
        offset: u32,
        ty: TypeId,
        managed: bool,
    },
    ListIndex {
        element: TypeId,
        index: Operand,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PlaceResolution {
    pub root: MirLocalId,
    pub steps: Vec<PlaceStep>,
    pub ty: TypeId,
}

pub(crate) struct Plan {
    pub aggregates: AggregateIndex,
    pub layouts: HashMap<MirAggregateId, AggregateLayout>,
    pub shared_types: BTreeSet<TypeId>,
    pub export_names: BTreeMap<MirFunctionId, String>,
    pub imports: BTreeMap<String, ImportSig>,
    pub external_functions: Vec<MirExternalFunctionId>,
    pub destructors: Vec<MirAggregateId>,
    pub traces: Vec<MirAggregateId>,
    pub closure_dtors: Vec<MirFunctionId>,
    pub closure_traces: Vec<MirFunctionId>,
    pub tuple_destructors: Vec<TypeId>,
    pub tuple_traces: Vec<TypeId>,
    pub task_thunks: Vec<MirFunctionId>,
    pub closure_thunks: Vec<MirFunctionId>,
    pub used_strings: BTreeSet<MirStringId>,
    pub value_classes: BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
}

impl Plan {
    pub(crate) fn ensure_import(&mut self, symbol: &str) {
        if !self.imports.contains_key(symbol) {
            self.imports
                .insert(symbol.to_string(), core_import_sig(symbol));
        }
    }

    pub(crate) fn ensure_custom_import(&mut self, symbol: &str, sig: ImportSig) {
        self.imports.entry(symbol.to_string()).or_insert(sig);
    }
}

pub(crate) fn core_import_sig(name: &str) -> ImportSig {
    let sig = |params: &[Val], result: Option<Val>| ImportSig {
        params: params.to_vec(),
        result,
    };
    match name {
        IMP_PGO_HIT | IMP_PGO_FLUSH => sig(&[Val::I64], None),
        IMP_PRINT_STR => sig(&[Val::Ptr], None),
        IMP_PRINT_INT => sig(&[Val::I64], None),
        IMP_PRINT_FLOAT => sig(&[Val::F64], None),
        IMP_PRINT_BOOL => sig(&[Val::I8], None),
        IMP_FMOD => sig(&[Val::F64, Val::F64], Some(Val::F64)),
        IMP_ARC_ALLOC => sig(&[Val::I64, Val::Ptr], Some(Val::Ptr)),
        IMP_ARC_RETAIN | IMP_ARC_RELEASE => sig(&[Val::Ptr], None),
        IMP_ARC_MARK_COLLECTIBLE
        | IMP_TRACE_VISIT
        | IMP_LIST_TRACE
        | IMP_TUPLE_TRACE
        | IMP_TASK_TRACE
        | IMP_CLOSURE_TRACE => sig(&[Val::Ptr, Val::Ptr], None),
        IMP_ARC_COLLECT_CYCLES | IMP_PANIC_DIV_ZERO => sig(&[], None),
        IMP_LIST_NEW | IMP_LIST_NEW_ARC => sig(&[], Some(Val::Ptr)),
        IMP_LIST_PUSH => sig(&[Val::Ptr, Val::I64], None),
        IMP_LIST_PUSH_ARC => sig(&[Val::Ptr, Val::Ptr], None),
        IMP_LIST_PUSH_FLOAT => sig(&[Val::Ptr, Val::F64], None),
        IMP_LIST_PUSH_BOOL => sig(&[Val::Ptr, Val::I8], None),
        IMP_LIST_GET => sig(&[Val::Ptr, Val::I64], Some(Val::I64)),
        IMP_LIST_GET_ARC => sig(&[Val::Ptr, Val::I64], Some(Val::Ptr)),
        IMP_LIST_GET_FLOAT => sig(&[Val::Ptr, Val::I64], Some(Val::F64)),
        IMP_LIST_GET_BOOL => sig(&[Val::Ptr, Val::I64], Some(Val::I8)),
        IMP_LIST_SET => sig(&[Val::Ptr, Val::I64, Val::I64], None),
        IMP_LIST_SET_ARC => sig(&[Val::Ptr, Val::I64, Val::Ptr], None),
        IMP_LIST_SET_FLOAT => sig(&[Val::Ptr, Val::I64, Val::F64], None),
        IMP_LIST_SET_BOOL => sig(&[Val::Ptr, Val::I64, Val::I8], None),
        IMP_LIST_LEN => sig(&[Val::Ptr], Some(Val::I64)),
        IMP_STR_CONCAT => sig(&[Val::Ptr, Val::Ptr], Some(Val::Ptr)),
        IMP_STR_EQ => sig(&[Val::Ptr, Val::Ptr], Some(Val::I64)),
        IMP_EMPTY_STR => sig(&[], Some(Val::Ptr)),
        IMP_INT_TO_STR => sig(&[Val::I64], Some(Val::Ptr)),
        IMP_FLOAT_TO_STR => sig(&[Val::F64], Some(Val::Ptr)),
        IMP_BOOL_TO_STR => sig(&[Val::I8], Some(Val::Ptr)),
        "lpp_u64_to_str" => sig(&[Val::I64], Some(Val::Ptr)),
        IMP_CLOSURE_DESTROY => sig(&[Val::Ptr], None),
        IMP_TUPLE_ALLOC => sig(&[Val::I64, Val::I64, Val::I64], Some(Val::Ptr)),
        IMP_TASK_NEW => sig(&[Val::Ptr, Val::Ptr, Val::I64], Some(Val::Ptr)),
        IMP_TASK_POLL | IMP_TASK_AWAIT => sig(&[Val::Ptr], Some(Val::I64)),
        IMP_TASK_DESTROY => sig(&[Val::Ptr], None),
        IMP_VEC_CHECKSUM => sig(&[Val::I64], Some(Val::I64)),
        "lpp_slice_init" => sig(
            &[Val::Ptr, Val::Ptr, Val::I64, Val::I64, Val::I64],
            Some(Val::Ptr),
        ),
        "lpp_str_slice_get" => sig(&[Val::Ptr, Val::I64], Some(Val::Ptr)),
        "lpp_arch_port_in8" => sig(&[Val::I16], Some(Val::I8)),
        "lpp_arch_port_in16" => sig(&[Val::I16], Some(Val::I16)),
        "lpp_arch_port_in32" => sig(&[Val::I16], Some(Val::I32)),
        "lpp_arch_port_out8" => sig(&[Val::I16, Val::I8], None),
        "lpp_arch_port_out16" => sig(&[Val::I16, Val::I16], None),
        "lpp_arch_port_out32" => sig(&[Val::I16, Val::I32], None),
        "lpp_arch_cpu_cli" | "lpp_arch_cpu_sti" | "lpp_arch_cpu_hlt" => sig(&[], None),
        "lpp_arch_rdtsc" | "lpp_arch_read_cr0" | "lpp_arch_read_cr2" | "lpp_arch_read_cr3"
        | "lpp_arch_read_cr4" => sig(&[], Some(Val::I64)),
        "lpp_arch_write_cr3" | "lpp_arch_write_cr4" | "lpp_arch_lgdt" | "lpp_arch_lidt"
        | "lpp_arch_invlpg" => sig(&[Val::I64], None),
        "lpp_arch_rdmsr" => sig(&[Val::I32], Some(Val::I64)),
        "lpp_arch_wrmsr" => sig(&[Val::I32, Val::I64], None),
        _ => sig(&[], None),
    }
}

pub(crate) fn builtin_descriptor_sig(builtin: BuiltinId) -> ImportSig {
    let d = builtin.descriptor();
    let map_param = |abi: AbiType, sem: SemanticAbiType| -> Val {
        if matches!(sem, SemanticAbiType::Str | SemanticAbiType::StrSlice) {
            return Val::Ptr;
        }
        match abi {
            AbiType::Bool => Val::I8,
            AbiType::I32 => Val::I32,
            AbiType::I64 => Val::I64,
            AbiType::F64 => Val::F64,
            AbiType::Void => Val::I64,
        }
    };
    let params = d
        .parameters
        .iter()
        .copied()
        .zip(d.semantic_parameters.iter().copied())
        .map(|(a, s)| map_param(a, s))
        .collect();
    let result = if d.result == AbiType::Void {
        None
    } else {
        Some(map_param(d.result, d.semantic_result))
    };
    ImportSig { params, result }
}

pub(crate) fn intrinsic_arch_import(intrinsic: KernelIntrinsic) -> Option<&'static str> {
    Some(match intrinsic {
        KernelIntrinsic::PortIn8 => "lpp_arch_port_in8",
        KernelIntrinsic::PortIn16 => "lpp_arch_port_in16",
        KernelIntrinsic::PortIn32 => "lpp_arch_port_in32",
        KernelIntrinsic::PortOut8 => "lpp_arch_port_out8",
        KernelIntrinsic::PortOut16 => "lpp_arch_port_out16",
        KernelIntrinsic::PortOut32 => "lpp_arch_port_out32",
        KernelIntrinsic::CpuCli => "lpp_arch_cpu_cli",
        KernelIntrinsic::CpuSti => "lpp_arch_cpu_sti",
        KernelIntrinsic::CpuHlt => "lpp_arch_cpu_hlt",
        KernelIntrinsic::Rdtsc => "lpp_arch_rdtsc",
        KernelIntrinsic::ReadCr0 => "lpp_arch_read_cr0",
        KernelIntrinsic::ReadCr2 => "lpp_arch_read_cr2",
        KernelIntrinsic::ReadCr3 => "lpp_arch_read_cr3",
        KernelIntrinsic::ReadCr4 => "lpp_arch_read_cr4",
        KernelIntrinsic::WriteCr3 => "lpp_arch_write_cr3",
        KernelIntrinsic::WriteCr4 => "lpp_arch_write_cr4",
        KernelIntrinsic::Rdmsr => "lpp_arch_rdmsr",
        KernelIntrinsic::Wrmsr => "lpp_arch_wrmsr",
        KernelIntrinsic::Lgdt => "lpp_arch_lgdt",
        KernelIntrinsic::Lidt => "lpp_arch_lidt",
        KernelIntrinsic::Invlpg => "lpp_arch_invlpg",
        _ => return None,
    })
}

pub(crate) fn pre_scan(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn NameResolver,
    pgo_instrument: bool,
) -> Result<Plan, CodegenError> {
    let shared_types: BTreeSet<TypeId> = lpp_ownership::compute_ownership_plan(program, types)
        .map(|plan| plan.pinned_types().into_iter().collect())
        .map_err(|error| {
            CodegenError::new(
                None,
                CodegenErrorKind::ObjectEmissionFailed(format!(
                    "ownership plan for cycle collection failed: {error}"
                )),
            )
        })?;

    let aggregates = AggregateIndex::build(program);
    let mut layouts = HashMap::new();
    for (id, _) in program.aggregates() {
        layouts.insert(id, aggregate_layout(program, types, id));
    }

    let value_classes = resolve_value_classes(program, types, &aggregates, &layouts)?;

    let mut export_names = BTreeMap::new();
    let mut used_export_names: BTreeSet<String> = BTreeSet::new();
    for (mir_id, function) in program.functions() {
        if let Some(custom) = program.function_attrs(mir_id).export_name.as_deref() {
            used_export_names.insert(custom.to_owned());
            export_names.insert(mir_id, custom.to_owned());
        } else if let Some(name) = function.name.as_ref().and_then(|s| names.resolve(s.raw())) {
            if name == "main" {
                export_names.insert(mir_id, "lpp_main".to_owned());
            } else {
                let mut export = name.to_owned();
                if used_export_names.contains(&export) {
                    export = format!("{export}__{}", mir_id.raw());
                }
                used_export_names.insert(export.clone());
                export_names.insert(mir_id, export);
            }
        } else {
            export_names.insert(mir_id, format!("lpp_c{}", mir_id.raw()));
        }
    }

    let mut plan = Plan {
        aggregates,
        layouts,
        shared_types: shared_types.clone(),
        export_names,
        imports: BTreeMap::new(),
        external_functions: Vec::new(),
        destructors: Vec::new(),
        traces: Vec::new(),
        closure_dtors: Vec::new(),
        closure_traces: Vec::new(),
        tuple_destructors: Vec::new(),
        tuple_traces: Vec::new(),
        task_thunks: Vec::new(),
        closure_thunks: Vec::new(),
        used_strings: BTreeSet::new(),
        value_classes,
    };

    if pgo_instrument {
        plan.ensure_import(IMP_PGO_HIT);
        plan.ensure_import(IMP_PGO_FLUSH);
    }

    let mut seen_externals = BTreeSet::new();
    let mut seen_dtors = BTreeSet::new();
    let mut seen_closure_dtors = BTreeSet::new();
    let mut seen_tuple_dtors = BTreeSet::new();
    let mut seen_task_thunks = BTreeSet::new();
    let mut seen_closure_thunks = BTreeSet::new();

    for (mir_id, function) in program.functions() {
        let mut has_managed_local = false;
        for &local in program.function_locals(function) {
            let desc = program.local(local).expect("validated MIR retains locals");
            if is_managed(types, desc.ty) && !matches!(desc.kind, MirLocalKind::Capture) {
                has_managed_local = true;
            }
            if !is_void_type(types, desc.ty) {
                let _ = val_of_type(types, desc.ty, mir_id)?;
            }
        }
        if !is_void_type(types, function.return_type) {
            let _ = val_of_type(types, function.return_type, mir_id)?;
        }
        if has_managed_local || is_managed(types, function.return_type) {
            plan.ensure_import(IMP_ARC_RETAIN);
            plan.ensure_import(IMP_ARC_RELEASE);
        }
        if matches!(function.kind, MirFunctionKind::Async) {
            seen_task_thunks.insert(mir_id);
        }

        for &block_id in program.function_blocks(function) {
            let block = program.block(block_id).expect("block");
            for &inst_id in program.block_instructions(block) {
                let inst = program.instruction(inst_id).expect("inst");
                match &inst.kind {
                    InstructionKind::Store { place, value } => {
                        record_operand_string(value, &mut plan);
                        let res = resolve_place(
                            program,
                            types,
                            *place,
                            &plan.aggregates,
                            &plan.layouts,
                            mir_id,
                            true,
                        )?;
                        scan_place_imports(&res, types, &mut plan);
                        if matches!(value, Operand::Copy(_)) {
                            match res.steps.last() {
                                None | Some(PlaceStep::Field { managed: true, .. }) => {
                                    plan.ensure_import(IMP_ARC_RETAIN);
                                }
                                _ => {}
                            }
                        }
                        if let Some(PlaceStep::ListIndex { element, .. }) = res.steps.last() {
                            if let PlaceStep::ListIndex { index, .. } =
                                res.steps.last().expect("step")
                            {
                                record_operand_string(index, &mut plan);
                            }
                            match element_class(types, *element) {
                                ElementClass::Managed => plan.ensure_import(IMP_LIST_SET_ARC),
                                ElementClass::Scalar => plan.ensure_import(IMP_LIST_SET),
                                ElementClass::Float => plan.ensure_import(IMP_LIST_SET_FLOAT),
                                ElementClass::Bool => plan.ensure_import(IMP_LIST_SET_BOOL),
                            }
                        }
                        if let Some(PlaceStep::Field { managed: true, .. }) = res.steps.last() {
                            plan.ensure_import(IMP_ARC_RELEASE);
                        }
                    }
                    InstructionKind::Assign { target, value } => {
                        record_rvalue_strings(program, value, &mut plan);
                        let target_ty = program.local(*target).expect("local").ty;
                        if is_managed(types, target_ty) {
                            plan.ensure_import(IMP_ARC_RELEASE);
                        }
                        scan_rvalue(
                            program,
                            types,
                            mir_id,
                            target_ty,
                            value,
                            &mut plan,
                            &mut seen_externals,
                            &mut seen_dtors,
                            &mut seen_closure_dtors,
                            &mut seen_tuple_dtors,
                            &mut seen_task_thunks,
                            &mut seen_closure_thunks,
                        )?;
                    }
                }
            }
            match &block.terminator {
                Terminator::Branch { condition, .. }
                | Terminator::SwitchEnum {
                    subject: condition, ..
                } => {
                    record_operand_string(condition, &mut plan);
                }
                Terminator::Return(Some(op)) => record_operand_string(op, &mut plan),
                Terminator::Goto(_) | Terminator::Return(None) | Terminator::Unreachable => {}
            }
        }
    }

    // Collect destructors & traces cleanly in deterministic ID order.
    for agg in seen_dtors {
        plan.destructors.push(agg);
        let agg_desc = program.aggregate(agg).expect("aggregate");
        let layout = &plan.layouts[&agg];
        let any_managed = match layout.kind {
            lpp_mir::MirAggregateKind::Struct => {
                layout.struct_fields.iter().any(|s| is_managed(types, s.ty))
            }
            lpp_mir::MirAggregateKind::Enum => layout
                .variant_fields
                .iter()
                .flatten()
                .any(|s| is_managed(types, s.ty)),
        };
        if any_managed {
            plan.ensure_import(IMP_ARC_RELEASE);
        }
        if plan.shared_types.contains(&agg_desc.ty) {
            plan.traces.push(agg);
            if any_managed {
                plan.ensure_import(IMP_TRACE_VISIT);
            }
        }
    }
    for fn_id in seen_closure_dtors {
        plan.closure_dtors.push(fn_id);
        let fn_desc = program.function(fn_id).expect("fn");
        if plan.shared_types.contains(&fn_desc.ty) {
            plan.closure_traces.push(fn_id);
            plan.ensure_import(IMP_TRACE_VISIT);
        }
    }
    for ty in seen_tuple_dtors {
        plan.tuple_destructors.push(ty);
        if let Some((slots, _)) = tuple_layout(types, ty)
            && slots.iter().any(|(_, t)| is_managed(types, *t))
        {
            plan.ensure_import(IMP_ARC_RELEASE);
        }
        if plan.shared_types.contains(&ty) {
            plan.tuple_traces.push(ty);
            plan.ensure_import(IMP_TRACE_VISIT);
        }
    }
    plan.external_functions = seen_externals.into_iter().collect();
    plan.task_thunks = seen_task_thunks.into_iter().collect();
    plan.closure_thunks = seen_closure_thunks.into_iter().collect();

    if !plan.shared_types.is_empty() && plan.export_names.values().any(|n| n == "lpp_main") {
        plan.ensure_import(IMP_ARC_COLLECT_CYCLES);
    }

    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
fn scan_rvalue(
    program: &MirProgram,
    types: &TypeInterner,
    mir_id: MirFunctionId,
    target_ty: TypeId,
    value: &Rvalue,
    plan: &mut Plan,
    seen_externals: &mut BTreeSet<MirExternalFunctionId>,
    seen_dtors: &mut BTreeSet<MirAggregateId>,
    seen_closure_dtors: &mut BTreeSet<MirFunctionId>,
    seen_tuple_dtors: &mut BTreeSet<TypeId>,
    seen_task_thunks: &mut BTreeSet<MirFunctionId>,
    seen_closure_thunks: &mut BTreeSet<MirFunctionId>,
) -> Result<(), CodegenError> {
    match value {
        Rvalue::Use(op) => {
            if let Operand::Function(fn_id) = op {
                plan.ensure_import(IMP_ARC_ALLOC);
                plan.ensure_import(IMP_CLOSURE_DESTROY);
                let f = program.function(*fn_id).expect("fn");
                if matches!(f.kind, MirFunctionKind::Async) {
                    seen_task_thunks.insert(*fn_id);
                    plan.ensure_import(IMP_TUPLE_ALLOC);
                }
                if plan.shared_types.contains(&f.ty) {
                    plan.ensure_import(IMP_ARC_MARK_COLLECTIBLE);
                    plan.ensure_import(IMP_CLOSURE_TRACE);
                }
            }
        }
        Rvalue::Binary {
            left,
            operator,
            right,
        } => {
            let l_ty = operand_kind(program, types, left);
            let r_ty = operand_kind(program, types, right);
            if *operator == lpp_mir::BinaryOperator::Add
                && l_ty == Some(TypeKind::Primitive(PrimitiveType::String))
            {
                plan.ensure_import(IMP_STR_CONCAT);
                match r_ty {
                    Some(TypeKind::Primitive(
                        PrimitiveType::Int
                        | PrimitiveType::I8
                        | PrimitiveType::I16
                        | PrimitiveType::I32
                        | PrimitiveType::ISize,
                    )) => plan.ensure_import(IMP_INT_TO_STR),
                    Some(TypeKind::Primitive(
                        PrimitiveType::U8
                        | PrimitiveType::U16
                        | PrimitiveType::U32
                        | PrimitiveType::U64
                        | PrimitiveType::USize,
                    )) => plan.ensure_import("lpp_u64_to_str"),
                    Some(TypeKind::Primitive(PrimitiveType::Float)) => {
                        plan.ensure_import(IMP_FLOAT_TO_STR)
                    }
                    Some(TypeKind::Primitive(PrimitiveType::Bool)) => {
                        plan.ensure_import(IMP_BOOL_TO_STR)
                    }
                    _ => {}
                }
            } else if matches!(
                operator,
                lpp_mir::BinaryOperator::Equal | lpp_mir::BinaryOperator::NotEqual
            ) && l_ty == Some(TypeKind::Primitive(PrimitiveType::String))
            {
                plan.ensure_import(IMP_STR_EQ);
            } else if matches!(
                operator,
                lpp_mir::BinaryOperator::Divide | lpp_mir::BinaryOperator::Modulo
            ) && l_ty != Some(TypeKind::Primitive(PrimitiveType::Float))
            {
                plan.ensure_import(IMP_PANIC_DIV_ZERO);
            }
        }
        Rvalue::ExternalCall { function, .. } => {
            seen_externals.insert(*function);
            let ext = program.external_function(*function).expect("ext");
            if let TypeKind::Function { result, .. } = types.kind(ext.ty)
                && matches!(
                    types.kind(result),
                    TypeKind::Primitive(PrimitiveType::String)
                )
            {
                plan.ensure_import(IMP_EMPTY_STR);
                plan.ensure_import(IMP_STR_CONCAT);
            }
        }
        Rvalue::Call { callee, .. } => {
            if let Operand::Function(fn_id) = callee {
                let f = program.function(*fn_id).expect("fn");
                if matches!(f.kind, MirFunctionKind::Async) {
                    seen_task_thunks.insert(*fn_id);
                    plan.ensure_import(IMP_TUPLE_ALLOC);
                    plan.ensure_import(IMP_TASK_NEW);
                    if plan.shared_types.contains(&f.return_type) {
                        plan.ensure_import(IMP_ARC_MARK_COLLECTIBLE);
                        plan.ensure_import(IMP_TUPLE_TRACE);
                        plan.ensure_import(IMP_TASK_TRACE);
                    }
                }
            } else if let Operand::Copy(local) = callee {
                let ty = program.local(*local).expect("local").ty;
                if let TypeKind::Function { result, .. } = types.kind(ty)
                    && matches!(types.kind(result), TypeKind::Task(_))
                {
                    plan.ensure_import(IMP_TUPLE_ALLOC);
                    plan.ensure_import(IMP_TASK_NEW);
                    if plan.shared_types.contains(&result) {
                        plan.ensure_import(IMP_ARC_MARK_COLLECTIBLE);
                        plan.ensure_import(IMP_TUPLE_TRACE);
                        plan.ensure_import(IMP_TASK_TRACE);
                    }
                }
            }
        }
        Rvalue::Load(place) => {
            let res = resolve_place(
                program,
                types,
                *place,
                &plan.aggregates,
                &plan.layouts,
                mir_id,
                false,
            )?;
            scan_place_imports(&res, types, plan);
            if let Some(last) = res.steps.last() {
                match last {
                    PlaceStep::Field { managed: true, .. } => plan.ensure_import(IMP_ARC_RETAIN),
                    PlaceStep::ListIndex { element, .. } => match element_class(types, *element) {
                        ElementClass::Managed => {
                            plan.ensure_import(IMP_LIST_GET_ARC);
                            plan.ensure_import(IMP_ARC_RETAIN);
                        }
                        ElementClass::Scalar => plan.ensure_import(IMP_LIST_GET),
                        ElementClass::Float => plan.ensure_import(IMP_LIST_GET_FLOAT),
                        ElementClass::Bool => plan.ensure_import(IMP_LIST_GET_BOOL),
                    },
                    _ => {}
                }
            }
        }
        Rvalue::ListLen(_) => plan.ensure_import(IMP_LIST_LEN),
        Rvalue::List(_) => {
            if let TypeKind::List(elem) = types.kind(target_ty) {
                if is_managed(types, elem) {
                    plan.ensure_import(IMP_LIST_NEW_ARC);
                } else {
                    plan.ensure_import(IMP_LIST_NEW);
                }
                match element_class(types, elem) {
                    ElementClass::Managed => {
                        plan.ensure_import(IMP_LIST_PUSH_ARC);
                        plan.ensure_import(IMP_ARC_RELEASE);
                    }
                    ElementClass::Scalar => plan.ensure_import(IMP_LIST_PUSH),
                    ElementClass::Float => plan.ensure_import(IMP_LIST_PUSH_FLOAT),
                    ElementClass::Bool => plan.ensure_import(IMP_LIST_PUSH_BOOL),
                }
                if plan.shared_types.contains(&target_ty) {
                    plan.ensure_import(IMP_ARC_MARK_COLLECTIBLE);
                    plan.ensure_import(IMP_LIST_TRACE);
                }
            }
        }
        Rvalue::ConstructStruct { aggregate, .. } | Rvalue::ConstructVariant { aggregate, .. } => {
            seen_dtors.insert(*aggregate);
            plan.ensure_import(IMP_ARC_ALLOC);
            let agg_desc = program.aggregate(*aggregate).expect("agg");
            if plan.shared_types.contains(&agg_desc.ty) {
                plan.ensure_import(IMP_ARC_MARK_COLLECTIBLE);
            }
        }
        Rvalue::Tuple(_) => {
            seen_tuple_dtors.insert(target_ty);
            plan.ensure_import(IMP_ARC_ALLOC);
            if plan.shared_types.contains(&target_ty) {
                plan.ensure_import(IMP_ARC_MARK_COLLECTIBLE);
            }
        }
        Rvalue::MakeClosure {
            function: closure_id,
            captures,
        } => {
            let closure_fn = program.function(*closure_id).expect("closure");
            if !program.operands(*captures).is_empty() {
                seen_closure_dtors.insert(*closure_id);
                plan.ensure_import(IMP_ARC_RELEASE);
                plan.ensure_import(IMP_ARC_RETAIN);
            }
            let zero_params = user_parameters(program, closure_fn).is_empty();
            if zero_params {
                seen_closure_thunks.insert(*closure_id);
                plan.ensure_import(IMP_TUPLE_ALLOC);
            }
            plan.ensure_import(IMP_ARC_ALLOC);
            plan.ensure_import(IMP_CLOSURE_DESTROY);
            if plan.shared_types.contains(&closure_fn.ty) {
                plan.ensure_import(IMP_ARC_MARK_COLLECTIBLE);
                plan.ensure_import(IMP_CLOSURE_TRACE);
                if zero_params {
                    plan.ensure_import(IMP_TUPLE_TRACE);
                }
            }
        }
        Rvalue::Await(_) => {
            plan.ensure_import(IMP_TASK_AWAIT);
        }
        Rvalue::Spawn(_) => {
            plan.ensure_import(IMP_ARC_RETAIN);
            plan.ensure_import(IMP_TASK_NEW);
            plan.ensure_import(IMP_TASK_POLL);
            plan.ensure_import(IMP_TASK_DESTROY);
        }
        Rvalue::Intrinsic { intrinsic, .. } => {
            if let Some(sym) = intrinsic_arch_import(*intrinsic) {
                plan.ensure_import(sym);
            }
        }
        Rvalue::Builtin { builtin, arguments } => {
            scan_builtin(
                *builtin,
                program.operands(*arguments),
                target_ty,
                program,
                types,
                plan,
            )?;
        }
        Rvalue::Unary { .. } | Rvalue::Cast { .. } => {}
    }
    Ok(())
}

fn record_operand_string(op: &Operand, plan: &mut Plan) {
    if let Operand::Constant(Constant::String { string, .. }) = op {
        plan.used_strings.insert(*string);
    }
}

fn record_rvalue_strings(program: &MirProgram, rvalue: &Rvalue, plan: &mut Plan) {
    match rvalue {
        Rvalue::Use(op)
        | Rvalue::Unary { operand: op, .. }
        | Rvalue::ListLen(op)
        | Rvalue::Await(op)
        | Rvalue::Spawn(op)
        | Rvalue::Cast { value: op, .. } => record_operand_string(op, plan),
        Rvalue::Binary { left, right, .. } => {
            record_operand_string(left, plan);
            record_operand_string(right, plan);
        }
        Rvalue::Call { callee, arguments } => {
            record_operand_string(callee, plan);
            for op in program.operands(*arguments) {
                record_operand_string(op, plan);
            }
        }
        Rvalue::ExternalCall { arguments, .. }
        | Rvalue::Builtin { arguments, .. }
        | Rvalue::Intrinsic { arguments, .. } => {
            for op in program.operands(*arguments) {
                record_operand_string(op, plan);
            }
        }
        Rvalue::List(ops)
        | Rvalue::ConstructStruct { fields: ops, .. }
        | Rvalue::ConstructVariant { fields: ops, .. }
        | Rvalue::Tuple(ops)
        | Rvalue::MakeClosure { captures: ops, .. } => {
            for op in program.operands(*ops) {
                record_operand_string(op, plan);
            }
        }
        Rvalue::Load(place) => {
            let desc = program.place(*place).expect("place");
            for proj in program.place_projections(desc) {
                if let PlaceProjection::ListIndex(op) = proj {
                    record_operand_string(op, plan);
                }
            }
        }
    }
}

fn scan_place_imports(res: &PlaceResolution, _types: &TypeInterner, plan: &mut Plan) {
    for step in res.steps.iter().take(res.steps.len().saturating_sub(1)) {
        if matches!(step, PlaceStep::ListIndex { .. }) {
            plan.ensure_import(IMP_LIST_GET_ARC);
        }
    }
}

fn scan_builtin(
    builtin: BuiltinId,
    args: &[Operand],
    target_ty: TypeId,
    program: &MirProgram,
    types: &TypeInterner,
    plan: &mut Plan,
) -> Result<(), CodegenError> {
    let d = builtin.descriptor();
    let name = d.name;
    match name {
        "print" => {
            if let Some(first) = args.first() {
                match operand_kind(program, types, first) {
                    Some(TypeKind::Primitive(PrimitiveType::Float)) => {
                        plan.ensure_import(IMP_PRINT_FLOAT)
                    }
                    Some(TypeKind::Primitive(PrimitiveType::Bool)) => {
                        plan.ensure_import(IMP_PRINT_BOOL)
                    }
                    Some(TypeKind::Primitive(PrimitiveType::String)) => {
                        plan.ensure_import(IMP_PRINT_STR)
                    }
                    _ => plan.ensure_import(IMP_PRINT_INT),
                }
            }
        }
        "list_new" => {
            if let TypeKind::List(elem) = types.kind(target_ty) {
                if is_managed(types, elem) {
                    plan.ensure_import(IMP_LIST_NEW_ARC);
                } else {
                    plan.ensure_import(IMP_LIST_NEW);
                }
                if plan.shared_types.contains(&target_ty) {
                    plan.ensure_import(IMP_ARC_MARK_COLLECTIBLE);
                    plan.ensure_import(IMP_LIST_TRACE);
                }
            }
        }
        "list_push" | "list_set" => {
            if let Some(Operand::Copy(local)) = args.first()
                && let TypeKind::List(elem) = types.kind(program.local(*local).expect("local").ty)
            {
                let push = name == "list_push";
                match element_class(types, elem) {
                    ElementClass::Managed => {
                        plan.ensure_import(if push {
                            IMP_LIST_PUSH_ARC
                        } else {
                            IMP_LIST_SET_ARC
                        });
                        plan.ensure_import(IMP_ARC_RELEASE);
                    }
                    ElementClass::Scalar => {
                        plan.ensure_import(if push { IMP_LIST_PUSH } else { IMP_LIST_SET });
                    }
                    ElementClass::Float => {
                        plan.ensure_import(if push {
                            IMP_LIST_PUSH_FLOAT
                        } else {
                            IMP_LIST_SET_FLOAT
                        });
                    }
                    ElementClass::Bool => {
                        plan.ensure_import(if push {
                            IMP_LIST_PUSH_BOOL
                        } else {
                            IMP_LIST_SET_BOOL
                        });
                    }
                }
            }
        }
        "list_get" | "get" => {
            if let Some(Operand::Copy(local)) = args.first()
                && let TypeKind::List(elem) = types.kind(program.local(*local).expect("local").ty)
            {
                match element_class(types, elem) {
                    ElementClass::Managed => {
                        plan.ensure_import(IMP_LIST_GET_ARC);
                        plan.ensure_import(IMP_ARC_RETAIN);
                    }
                    ElementClass::Scalar => plan.ensure_import(IMP_LIST_GET),
                    ElementClass::Float => plan.ensure_import(IMP_LIST_GET_FLOAT),
                    ElementClass::Bool => plan.ensure_import(IMP_LIST_GET_BOOL),
                }
            }
        }
        "list_len" | "len" => plan.ensure_import(IMP_LIST_LEN),
        "slice" | "str_slice" => plan.ensure_import("lpp_slice_init"),
        "slice_get" => {
            let is_str = args.first().and_then(|op| operand_kind(program, types, op))
                == Some(TypeKind::Primitive(PrimitiveType::StrSlice));
            let sym = if is_str {
                "lpp_str_slice_get"
            } else {
                d.symbol
            };
            if is_str {
                plan.ensure_import(sym);
            } else {
                plan.ensure_custom_import(sym, builtin_descriptor_sig(builtin));
            }
        }
        "vec_i64_checksum" => plan.ensure_import(IMP_VEC_CHECKSUM),
        _ if name.starts_with("vec_") => {}
        _ => {
            if !d.symbol.is_empty() {
                plan.ensure_custom_import(d.symbol, builtin_descriptor_sig(builtin));
            }
        }
    }
    Ok(())
}

pub(crate) fn operand_kind(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
) -> Option<TypeKind> {
    match operand {
        Operand::Copy(local) => Some(types.kind(program.local(*local)?.ty)),
        Operand::Constant(c) => Some(TypeKind::Primitive(match c {
            Constant::Integer(_) => PrimitiveType::Int,
            Constant::FloatBits(_) => PrimitiveType::Float,
            Constant::Bool(_) => PrimitiveType::Bool,
            Constant::Character { .. } => PrimitiveType::Char,
            Constant::String { .. } => PrimitiveType::String,
        })),
        Operand::Function(fn_id) => Some(types.kind(program.function(*fn_id)?.ty)),
    }
}

pub(crate) fn resolve_place(
    program: &MirProgram,
    types: &TypeInterner,
    place: lpp_mir::MirPlaceId,
    aggregates: &AggregateIndex,
    layouts: &HashMap<MirAggregateId, AggregateLayout>,
    mir_id: MirFunctionId,
    for_store: bool,
) -> Result<PlaceResolution, CodegenError> {
    let descriptor = program.place(place).expect("validated MIR retains places");
    let projections: Vec<PlaceProjection> = program.place_projections(descriptor).to_vec();
    if for_store && matches!(projections.last(), Some(PlaceProjection::Downcast(_))) {
        return Err(unsupported(mir_id, "store through downcast"));
    }

    let mut current_ty = program.local(descriptor.root).expect("local").ty;
    let mut steps = Vec::new();
    let mut active_enum: Option<(MirAggregateId, u32)> = None;

    for (position, projection) in projections.iter().enumerate() {
        let is_last = position + 1 == projections.len();
        match projection {
            PlaceProjection::Downcast(variant) => {
                let agg_id = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported(mir_id, "downcast of non-nominal"))?;
                let variant_desc = program.variant(*variant).expect("variant");
                active_enum = Some((agg_id, variant_desc.ordinal));
            }
            PlaceProjection::Field(field) => {
                let agg_id = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported(mir_id, "field of non-nominal"))?;
                let layout = layouts.get(&agg_id).expect("layout");
                let slots = match layout.kind {
                    lpp_mir::MirAggregateKind::Struct => &layout.struct_fields,
                    lpp_mir::MirAggregateKind::Enum => {
                        let (_, ordinal) = active_enum
                            .take()
                            .ok_or_else(|| unsupported(mir_id, "enum field without downcast"))?;
                        &layout.variant_fields[ordinal as usize]
                    }
                };
                let slot = slots
                    .iter()
                    .find(|s| s.field == *field)
                    .ok_or_else(|| unsupported(mir_id, "unknown field offset"))?;
                let managed = is_managed(types, slot.ty);
                if !is_last && !managed {
                    return Err(unsupported(mir_id, "projection through scalar field"));
                }
                steps.push(PlaceStep::Field {
                    offset: slot.offset,
                    ty: slot.ty,
                    managed,
                });
                current_ty = slot.ty;
            }
            PlaceProjection::ListIndex(index) => {
                let TypeKind::List(element) = types.kind(current_ty) else {
                    return Err(unsupported(mir_id, "index of non-list"));
                };
                if !is_last && !is_managed(types, element) {
                    return Err(unsupported(
                        mir_id,
                        "projection through scalar list element",
                    ));
                }
                steps.push(PlaceStep::ListIndex {
                    element,
                    index: *index,
                });
                current_ty = element;
            }
            PlaceProjection::TupleField(index) => {
                let Some((slots, _)) = tuple_layout(types, current_ty) else {
                    return Err(unsupported(mir_id, "tuple field of non-tuple"));
                };
                let (offset, element) = slots
                    .get(*index as usize)
                    .copied()
                    .ok_or_else(|| unsupported(mir_id, "tuple field out of range"))?;
                let managed = is_managed(types, element);
                if !is_last && !managed {
                    return Err(unsupported(mir_id, "projection through scalar tuple field"));
                }
                steps.push(PlaceStep::Field {
                    offset,
                    ty: element,
                    managed,
                });
                current_ty = element;
            }
        }
    }

    Ok(PlaceResolution {
        root: descriptor.root,
        steps,
        ty: current_ty,
    })
}

fn is_function_value_kind(types: &TypeInterner, kind: TypeKind) -> bool {
    match kind {
        TypeKind::Function { .. } => true,
        TypeKind::List(element) => matches!(types.kind(element), TypeKind::Function { .. }),
        _ => false,
    }
}

fn resolve_value_classes(
    program: &MirProgram,
    types: &TypeInterner,
    aggregates: &AggregateIndex,
    layouts: &HashMap<MirAggregateId, AggregateLayout>,
) -> Result<BTreeMap<(MirFunctionId, MirLocalId), ValueClass>, CodegenError> {
    type ClassKey = (MirFunctionId, MirLocalId);
    fn set_const(
        classes: &mut BTreeMap<ClassKey, ValueClass>,
        key: ClassKey,
        class: ValueClass,
    ) -> Result<(), CodegenError> {
        if let Some(existing) = classes.get(&key) {
            if *existing != class {
                return Err(unsupported(key.0, "mixed function value origins"));
            }
            return Ok(());
        }
        classes.insert(key, class);
        Ok(())
    }

    let local_ty = |local: MirLocalId| program.local(local).expect("local").ty;
    let carries_fn = |local: MirLocalId| is_function_value_kind(types, types.kind(local_ty(local)));
    let fn_element = |ty: TypeId| match types.kind(ty) {
        TypeKind::List(element) => matches!(types.kind(element), TypeKind::Function { .. }),
        _ => false,
    };

    let mut classes: BTreeMap<ClassKey, ValueClass> = BTreeMap::new();
    let mut equalities: Vec<(ClassKey, ClassKey)> = Vec::new();

    for (mir_id, function) in program.functions() {
        for &local in program.function_locals(function) {
            let desc = program.local(local).expect("local");
            if matches!(types.kind(desc.ty), TypeKind::Function { .. })
                && matches!(desc.kind, MirLocalKind::Parameter)
            {
                set_const(&mut classes, (mir_id, local), ValueClass::Closure)?;
            }
        }
        for &block in program.function_blocks(function) {
            let basic = program.block(block).expect("block");
            for &instruction in program.block_instructions(basic) {
                let instr = program.instruction(instruction).expect("inst");
                match &instr.kind {
                    InstructionKind::Assign { target, value } => match value {
                        Rvalue::Use(Operand::Function(_)) => {
                            set_const(&mut classes, (mir_id, *target), ValueClass::FunctionValue)?;
                        }
                        Rvalue::Use(Operand::Copy(source)) if carries_fn(*source) => {
                            equalities.push(((mir_id, *target), (mir_id, *source)));
                        }
                        Rvalue::MakeClosure {
                            function: closure_id,
                            captures,
                        } => {
                            set_const(&mut classes, (mir_id, *target), ValueClass::Closure)?;
                            let closure_fn = program.function(*closure_id).expect("fn");
                            let capture_locals: Vec<MirLocalId> = program
                                .function_parameters(closure_fn)
                                .iter()
                                .copied()
                                .filter(|l| {
                                    matches!(
                                        program.local(*l).expect("l").kind,
                                        MirLocalKind::Capture
                                    )
                                })
                                .collect();
                            for (op, cap_local) in
                                program.operands(*captures).iter().zip(capture_locals)
                            {
                                if let Operand::Copy(src) = *op
                                    && carries_fn(src)
                                {
                                    equalities.push(((*closure_id, cap_local), (mir_id, src)));
                                }
                            }
                        }
                        Rvalue::Load(place) => {
                            if let Ok(res) = resolve_place(
                                program, types, *place, aggregates, layouts, mir_id, false,
                            ) && let Some(PlaceStep::ListIndex { element, .. }) =
                                res.steps.last()
                                && matches!(types.kind(*element), TypeKind::Function { .. })
                            {
                                equalities.push(((mir_id, *target), (mir_id, res.root)));
                            }
                        }
                        Rvalue::List(operands) if fn_element(local_ty(*target)) => {
                            for op in program.operands(*operands) {
                                if let Operand::Copy(src) = *op {
                                    equalities.push(((mir_id, *target), (mir_id, src)));
                                }
                            }
                        }
                        Rvalue::Builtin { builtin, arguments } => {
                            let name = builtin.descriptor().name;
                            if matches!(name, "list_push" | "list_set" | "list_get" | "get") {
                                let args = program.operands(*arguments);
                                if let Some(Operand::Copy(list_local)) = args.first()
                                    && fn_element(local_ty(*list_local))
                                {
                                    match name {
                                        "list_get" | "get" => {
                                            equalities
                                                .push(((mir_id, *target), (mir_id, *list_local)));
                                        }
                                        _ => {
                                            if let Some(Operand::Copy(src)) = args.get(1) {
                                                equalities
                                                    .push(((mir_id, *list_local), (mir_id, *src)));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                    InstructionKind::Store {
                        place,
                        value: Operand::Copy(src),
                    } => {
                        if let Ok(res) =
                            resolve_place(program, types, *place, aggregates, layouts, mir_id, true)
                            && let Some(PlaceStep::ListIndex { element, .. }) = res.steps.last()
                            && matches!(types.kind(*element), TypeKind::Function { .. })
                        {
                            equalities.push(((mir_id, res.root), (mir_id, *src)));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    loop {
        let mut changed = false;
        for &(a, b) in &equalities {
            match (classes.get(&a).copied(), classes.get(&b).copied()) {
                (Some(x), Some(y)) if x != y => {
                    return Err(unsupported(a.0, "mixed function value origins"));
                }
                (Some(x), None) => {
                    classes.insert(b, x);
                    changed = true;
                }
                (None, Some(y)) => {
                    classes.insert(a, y);
                    changed = true;
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    Ok(classes)
}
