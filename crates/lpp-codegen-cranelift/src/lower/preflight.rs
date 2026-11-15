use super::*;

mod shape_assign;
mod shape_builtin;

use shape_assign::check_assign_shape;
use shape_builtin::check_builtin_shape;

/// Walk the whole program and type-reject anything outside the slice
/// before the module holds any declaration: a backend either lowers
/// the program completely or reports the exact first rejection.
pub(crate) fn pre_scan(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn lpp_codegen_api::NameResolver,
    pgo_instrument: bool,
) -> Result<Plan, CodegenError> {
    // The 4D ownership plan decides which allocations can sit on a
    // reference cycle (`Shared` types) and therefore need collector
    // marking. The driver proves the same plan independently; the
    // backend recomputes it because the `Backend` contract carries
    // only MIR + types.
    let shared_types: BTreeSet<TypeId> = lpp_ownership::compute_ownership_plan(program, types)
        .map(|plan| plan.pinned_types().into_iter().collect())
        .map_err(|error| {
            emission_failed(format!(
                "ownership plan for cycle collection failed: {error}"
            ))
        })?;
    let aggregates = AggregateIndex::build(program);
    let mut layouts = HashMap::new();
    for (id, _) in program.aggregates() {
        layouts.insert(id, aggregate_layout(program, types, id));
    }

    let mut export_names = BTreeMap::new();
    let mut used_export_names: BTreeSet<String> = BTreeSet::new();
    let mut imports: BTreeSet<&'static str> = BTreeSet::new();
    if pgo_instrument {
        imports.insert(IMP_PGO_HIT);
        imports.insert(IMP_PGO_FLUSH);
    }
    let mut external_functions: BTreeSet<MirExternalFunctionId> = BTreeSet::new();
    let mut destructors: Vec<MirAggregateId> = Vec::new();
    let mut closure_dtors: Vec<MirFunctionId> = Vec::new();
    let mut tuple_destructors: Vec<TypeId> = Vec::new();
    let mut task_thunks: Vec<MirFunctionId> = Vec::new();
    let mut closure_thunks: Vec<MirFunctionId> = Vec::new();
    // Function-value provenance is resolved by a whole-program
    // fixpoint first (see `resolve_value_classes`): the per-function
    // scan below then only conflict-checks what it re-derives.
    let mut value_classes = resolve_value_classes(program, types, &aggregates, &layouts)?;

    for (mir_id, function) in program.functions() {
        {
            let mut scan = ScanState {
                shared: &shared_types,
                imports: &mut imports,
                external_functions: &mut external_functions,
                destructors: &mut destructors,
                closure_dtors: &mut closure_dtors,
                tuple_destructors: &mut tuple_destructors,
                task_thunks: &mut task_thunks,
                closure_thunks: &mut closure_thunks,
                value_classes: &mut value_classes,
            };
            check_function_shape(
                program,
                types,
                function,
                mir_id,
                &aggregates,
                &layouts,
                &mut scan,
            )?;
        }

        // Custom `@export_name("...")` overrides the default symbol mapping so
        // kernels and bare-metal entry points (`_start`) or C-ABI hooks export
        // under their exact link name. Otherwise source-level `main` exports as
        // `lpp_main`, and closures as `lpp_c{n}`.
        if let Some(custom) = program.function_attrs(mir_id).export_name.as_deref() {
            let export = custom.to_owned();
            used_export_names.insert(export.clone());
            export_names.insert(mir_id, export);
        } else {
            match function
                .name
                .as_ref()
                .and_then(|symbol| names.resolve(symbol.raw()))
            {
                Some(name) if name == "main" => {
                    export_names.insert(mir_id, "lpp_main".to_owned());
                }
                Some(name) => {
                    // Distinct functions/methods can share a source name or match
                    // reserved C runtime symbols like `free`/`open`/`read`.
                    let export = lpp_codegen_api::sanitize_internal_export_name(
                        name,
                        mir_id.raw(),
                        &mut used_export_names,
                    );
                    export_names.insert(mir_id, export);
                }
                None if matches!(function.kind, MirFunctionKind::Closure) => {
                    export_names.insert(mir_id, format!("lpp_c{}", mir_id.raw()));
                }
                None => return Err(unsupported(mir_id, "unnamed function")),
            }
        }
    }

    // An async entry `main` is drained by the generated `main`
    // wrapper through its task thunk (the wrapper never calls
    // `lpp_main` directly), so the thunk must exist even though no
    // MIR instruction references it.
    let main_id = export_names
        .iter()
        .find(|(_, name)| name.as_str() == "lpp_main")
        .map(|(id, _)| *id);
    if let (Some(main_id), Some(main_fn)) = (main_id, main_id.and_then(|id| program.function(id))) {
        if matches!(main_fn.kind, MirFunctionKind::Async) {
            task_thunks.push(main_id);
            imports.insert(IMP_TUPLE_ALLOC);
            imports.insert(IMP_TASK_NEW);
            imports.insert(IMP_TASK_AWAIT);
            imports.insert(IMP_TASK_DESTROY);
        }
    }

    // The generated `main` wrapper runs one final collection so a
    // program that ends holding only cyclic garbage exits leak-free.
    if main_id.is_some() && !shared_types.is_empty() {
        imports.insert(IMP_COLLECT_CYCLES);
    }

    destructors.sort_unstable();
    destructors.dedup();
    closure_dtors.sort_unstable();
    closure_dtors.dedup();
    tuple_destructors.sort_unstable();
    tuple_destructors.dedup();
    task_thunks.sort_unstable();
    task_thunks.dedup();
    closure_thunks.sort_unstable();
    closure_thunks.dedup();

    let traces: Vec<MirAggregateId> = destructors
        .iter()
        .copied()
        .filter(|aggregate| {
            program
                .aggregate(*aggregate)
                .is_some_and(|agg| shared_types.contains(&agg.ty))
        })
        .collect();
    let closure_traces: Vec<MirFunctionId> = closure_dtors
        .iter()
        .copied()
        .filter(|function_id| {
            program
                .function(*function_id)
                .is_some_and(|function| shared_types.contains(&function.ty))
        })
        .collect();
    let tuple_traces: Vec<TypeId> = tuple_destructors
        .iter()
        .copied()
        .filter(|ty| shared_types.contains(ty))
        .collect();

    Ok(Plan {
        export_names,
        imports,
        external_functions: external_functions.into_iter().collect(),
        destructors,
        closure_dtors,
        task_thunks,
        closure_thunks,
        value_classes,
        shared_types,
        traces,
        closure_traces,
        tuple_destructors,
        tuple_traces,
        aggregates,
        layouts,
    })
}

/// The provenance class of a function-value local. A closure value
/// and a materialized function value are the same MIR type, but their
/// capsule code pointers have different call ABIs (the closure code
/// takes the capture env first), so the class is tracked per local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueClass {
    Closure,
    FunctionValue,
}

/// The mutable scan state threaded through `check_function_shape`.
struct ScanState<'a> {
    shared: &'a BTreeSet<TypeId>,
    imports: &'a mut BTreeSet<&'static str>,
    external_functions: &'a mut BTreeSet<MirExternalFunctionId>,
    destructors: &'a mut Vec<MirAggregateId>,
    closure_dtors: &'a mut Vec<MirFunctionId>,
    tuple_destructors: &'a mut Vec<TypeId>,
    task_thunks: &'a mut Vec<MirFunctionId>,
    closure_thunks: &'a mut Vec<MirFunctionId>,
    value_classes: &'a mut BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
}

/// Resolve function-value provenance classes for the whole program.
///
/// The per-function scan records classes in instruction order, which
/// cannot see provenance that arrives *backwards* (a list whose
/// element class is established by a push after a copy) or *across*
/// functions (a list captured into a closure whose body reads function
/// values out of it — the capture's class lives in the enclosing
/// function). This pre-pass therefore collects the same facts as
/// constraints — constants (materialized functions, closures created
/// by `MakeClosure`, function-typed parameters) and equalities
/// (copies, list push/get/set, list literals, closure captures) — and
/// propagates them to a fixpoint. A component forced to both classes
/// is the same "mixed function value origins" the scan rejects.
///
/// Deliberately *not* modelled (matching the scan's long-standing
/// boundary): provenance through struct fields and through direct
/// call arguments/returns, where function-typed parameters keep their
/// caller-supplied-capsule default. Unconstrained locals stay absent
/// from the map and are still rejected as unknown origin if a call
/// site actually consumes them.
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
    let local_ty = |local: MirLocalId| {
        program
            .local(local)
            .expect("validated MIR retains locals")
            .ty
    };
    let carries_fn = |local: MirLocalId| is_function_value_kind(types, types.kind(local_ty(local)));
    let fn_element = |ty: TypeId| match types.kind(ty) {
        TypeKind::List(element) => matches!(types.kind(element), TypeKind::Function { .. }),
        _ => false,
    };

    let mut classes: BTreeMap<ClassKey, ValueClass> = BTreeMap::new();
    let mut equalities: Vec<(ClassKey, ClassKey)> = Vec::new();

    for (mir_id, function) in program.functions() {
        for &local in program.function_locals(function) {
            let descriptor = program.local(local).expect("validated MIR retains locals");
            if matches!(types.kind(descriptor.ty), TypeKind::Function { .. })
                && matches!(descriptor.kind, lpp_mir::MirLocalKind::Parameter)
            {
                set_const(&mut classes, (mir_id, local), ValueClass::Closure)?;
            }
        }
        for &block in program.function_blocks(function) {
            let basic = program.block(block).expect("validated MIR retains blocks");
            for &instruction in program.block_instructions(basic) {
                let instr = program
                    .instruction(instruction)
                    .expect("validated MIR retains instructions");
                match &instr.kind {
                    InstructionKind::Assign { target, value } => match value {
                        Rvalue::Use(Operand::Function(_)) => {
                            set_const(&mut classes, (mir_id, *target), ValueClass::FunctionValue)?;
                        }
                        Rvalue::Use(Operand::Copy(source)) => {
                            if carries_fn(*source) {
                                equalities.push(((mir_id, *target), (mir_id, *source)));
                            }
                        }
                        Rvalue::MakeClosure {
                            function: closure_id,
                            captures,
                        } => {
                            set_const(&mut classes, (mir_id, *target), ValueClass::Closure)?;
                            let closure_fn = program
                                .function(*closure_id)
                                .expect("validated MIR retains functions");
                            let capture_locals: Vec<MirLocalId> = program
                                .function_parameters(closure_fn)
                                .iter()
                                .copied()
                                .filter(|local| {
                                    matches!(
                                        program
                                            .local(*local)
                                            .expect("validated MIR retains locals")
                                            .kind,
                                        lpp_mir::MirLocalKind::Capture
                                    )
                                })
                                .collect();
                            for (operand, capture_local) in
                                program.operands(*captures).iter().zip(capture_locals)
                            {
                                if let Operand::Copy(source) = *operand
                                    && carries_fn(source)
                                {
                                    equalities
                                        .push(((*closure_id, capture_local), (mir_id, source)));
                                }
                            }
                        }
                        Rvalue::Load(place) => {
                            if let Ok(resolution) = resolve_place(
                                program, types, *place, aggregates, layouts, mir_id, false,
                            ) && let Some(PlaceStep::ListIndex { element, .. }) =
                                resolution.steps.last()
                                && matches!(types.kind(*element), TypeKind::Function { .. })
                            {
                                equalities.push(((mir_id, *target), (mir_id, resolution.root)));
                            }
                        }
                        Rvalue::List(operands) => {
                            if fn_element(local_ty(*target)) {
                                for operand in program.operands(*operands) {
                                    if let Operand::Copy(source) = *operand {
                                        equalities.push(((mir_id, *target), (mir_id, source)));
                                    }
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
                                            if let Some(Operand::Copy(source)) = args.get(1) {
                                                equalities.push((
                                                    (mir_id, *list_local),
                                                    (mir_id, *source),
                                                ));
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
                        value: Operand::Copy(source),
                    } => {
                        if let Ok(resolution) =
                            resolve_place(program, types, *place, aggregates, layouts, mir_id, true)
                            && let Some(PlaceStep::ListIndex { element, .. }) =
                                resolution.steps.last()
                            && matches!(types.kind(*element), TypeKind::Function { .. })
                        {
                            equalities.push(((mir_id, resolution.root), (mir_id, *source)));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Monotone propagation: each key is labelled at most once, so
    // this terminates after at most (#keys) productive passes.
    loop {
        let mut changed = false;
        for &(a, b) in &equalities {
            match (classes.get(&a).copied(), classes.get(&b).copied()) {
                (Some(x), Some(y)) => {
                    if x != y {
                        return Err(unsupported(a.0, "mixed function value origins"));
                    }
                }
                (Some(x), None) => {
                    classes.insert(b, x);
                    changed = true;
                }
                (None, Some(y)) => {
                    classes.insert(a, y);
                    changed = true;
                }
                (None, None) => {}
            }
        }
        if !changed {
            break;
        }
    }
    Ok(classes)
}

/// Record (or conflict-check) the provenance class of a
/// function-value local: a local holding both a closure and a
/// materialized function mixes the two call ABIs.
fn record_value_class(
    scan: &mut ScanState,
    mir_id: MirFunctionId,
    local: MirLocalId,
    class: ValueClass,
) -> Result<(), CodegenError> {
    if let Some(existing) = scan.value_classes.get(&(mir_id, local)) {
        if *existing != class {
            return Err(unsupported(mir_id, "mixed function value origins"));
        }
        return Ok(());
    }
    scan.value_classes.insert((mir_id, local), class);
    Ok(())
}

/// The user parameters of a function (capture parameters excluded —
/// a closure's ABI is the env pointer plus the user parameters).
pub(super) fn user_parameters(program: &MirProgram, function: &MirFunction) -> Vec<MirLocalId> {
    program
        .function_parameters(function)
        .iter()
        .copied()
        .filter(|local| {
            !matches!(
                program
                    .local(*local)
                    .expect("validated MIR retains locals")
                    .kind,
                lpp_mir::MirLocalKind::Capture
            )
        })
        .collect()
}

fn check_function_shape(
    program: &MirProgram,
    types: &TypeInterner,
    function: &MirFunction,
    mir_id: MirFunctionId,
    aggregates: &AggregateIndex,
    layouts: &HashMap<MirAggregateId, AggregateLayout>,
    scan: &mut ScanState,
) -> Result<(), CodegenError> {
    // Plain functions, closure functions (lowered with the env
    // pointer as their leading parameter), and async functions
    // (called through generated task thunks) all lower.
    match function.kind {
        MirFunctionKind::Function | MirFunctionKind::Closure | MirFunctionKind::Async => {}
    }

    let mut has_managed_local = false;
    for &local in program.function_locals(function) {
        let ty = program
            .local(local)
            .expect("validated MIR retains locals")
            .ty;
        if !machine_type(types, ty).is_some() && !is_void_type(types, ty) {
            return Err(unsupported(
                mir_id,
                unsupported_construct_for(types.kind(ty)),
            ));
        }
        has_managed_local |= is_managed(types, ty);
        if matches!(types.kind(ty), TypeKind::Function { .. })
            && matches!(
                program
                    .local(local)
                    .expect("validated MIR retains locals")
                    .kind,
                lpp_mir::MirLocalKind::Parameter
            )
        {
            // Function-typed parameters are capsule values supplied by the caller.
            // The current source surface most commonly passes closures here; mark
            // the parameter as closure-compatible so indirect calls have a known
            // ABI instead of failing as "unknown origin".
            record_value_class(scan, mir_id, local, ValueClass::Closure)?;
        }
    }
    let return_managed = is_managed(types, function.return_type);
    if !machine_type(types, function.return_type).is_some()
        && !is_void_type(types, function.return_type)
    {
        return Err(unsupported(
            mir_id,
            unsupported_construct_for(types.kind(function.return_type)),
        ));
    }
    if has_managed_local {
        // Every managed slot is released by the exit pass, and every
        // transfer that sources from a local retains.
        scan.imports.insert(IMP_ARC_RELEASE);
        scan.imports.insert(IMP_ARC_RETAIN);
    }
    if return_managed {
        // The return retains for the caller (the callee's exit pass
        // releases the slot's own reference).
        scan.imports.insert(IMP_ARC_RETAIN);
    }

    for &block in program.function_blocks(function) {
        let basic = program.block(block).expect("validated MIR retains blocks");
        for &instruction in program.block_instructions(basic) {
            let instr = program
                .instruction(instruction)
                .expect("validated MIR retains instructions");
            match &instr.kind {
                InstructionKind::Store { place, value } => {
                    check_store_shape(
                        program, types, *place, value, mir_id, aggregates, layouts, scan,
                    )?;
                }
                InstructionKind::Assign { target, value } => {
                    check_assign_shape(
                        program, types, *target, value, function, mir_id, aggregates, layouts, scan,
                    )?;
                }
            }
        }

        check_terminator_shape(
            program,
            types,
            function,
            mir_id,
            aggregates,
            &basic.terminator,
        )?;
    }

    Ok(())
}

fn check_binary(
    operator: BinaryOperator,
    left: CLType,
    right: CLType,
    function: MirFunctionId,
) -> Result<(), CodegenError> {
    let invalid = || Err(unsupported(function, "invalid binary operands"));
    let integer_operands = left.is_int() && right.is_int();
    if left != right && !integer_operands {
        return invalid();
    }
    let comparison = matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::Less
            | BinaryOperator::Greater
            | BinaryOperator::LessEqual
            | BinaryOperator::GreaterEqual
    );
    let arithmetic = matches!(
        operator,
        BinaryOperator::Add
            | BinaryOperator::Subtract
            | BinaryOperator::Multiply
            | BinaryOperator::Divide
            | BinaryOperator::Modulo
    );
    let bitwise = matches!(
        operator,
        BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::BitXor
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight
    );
    if integer_operands {
        if arithmetic || bitwise || comparison {
            return Ok(());
        }
        if left == cltypes::I8
            && right == cltypes::I8
            && matches!(
                operator,
                BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr
            )
        {
            return Ok(());
        }
        return invalid();
    }
    match left {
        cltypes::F64 if arithmetic || comparison => Ok(()),
        _ => invalid(),
    }
}

fn check_store_shape(
    program: &MirProgram,
    types: &TypeInterner,
    place: lpp_mir::MirPlaceId,
    value: &Operand,
    mir_id: MirFunctionId,
    aggregates: &AggregateIndex,
    layouts: &HashMap<MirAggregateId, AggregateLayout>,
    scan: &mut ScanState,
) -> Result<(), CodegenError> {
    let check_operand = |operand: &Operand| operand_type(program, types, operand, mir_id);
    let resolution = resolve_place(program, types, place, aggregates, layouts, mir_id, true)?;
    check_operand(value)?;
    if matches!(value, Operand::Copy(_)) {
        match resolution.steps.last() {
            // Bare local or struct field: the backend
            // retains for the new owner.
            None | Some(PlaceStep::Field { managed: true, .. }) => {
                scan.imports.insert(IMP_ARC_RETAIN);
            }
            // The runtime's lpp_list_set_arc retains
            // (and releases the replaced element)
            // internally; a scalar field replacement
            // releases nothing.
            _ => {}
        }
    }
    if let Some(PlaceStep::ListIndex { element, .. }) = resolution.steps.last() {
        match element_class(types, *element) {
            ElementClass::Managed => {
                scan.imports.insert(IMP_LIST_SET_ARC);
            }
            ElementClass::Scalar => {
                scan.imports.insert(IMP_LIST_SET);
            }
            ElementClass::Float => {
                scan.imports.insert(IMP_LIST_SET_FLOAT);
            }
            ElementClass::Bool => {
                scan.imports.insert(IMP_LIST_SET_BOOL);
            }
        };
        // A list of function values inherits the
        // provenance class of what it stores.
        if matches!(types.kind(*element), TypeKind::Function { .. })
            && let Operand::Copy(source) = value
        {
            let class = scan
                .value_classes
                .get(&(mir_id, *source))
                .copied()
                .ok_or_else(|| unsupported(mir_id, "function value of unknown origin"))?;
            record_value_class(scan, mir_id, resolution.root, class)?;
        }
    }
    record_chain_gets(types, &resolution, scan.imports);
    Ok(())
}

fn check_terminator_shape(
    program: &MirProgram,
    types: &TypeInterner,
    function: &MirFunction,
    mir_id: MirFunctionId,
    aggregates: &AggregateIndex,
    terminator: &Terminator,
) -> Result<(), CodegenError> {
    let check_operand = |operand: &Operand| operand_type(program, types, operand, mir_id);
    let local_ty = |local: MirLocalId| {
        program
            .local(local)
            .expect("validated MIR retains locals")
            .ty
    };
    match terminator {
        Terminator::Goto(_) => {}
        Terminator::Branch { condition, .. } => {
            let ty = check_operand(condition)?;
            // A `Bool` (i8) or an integer handle (i64) is a valid
            // condition: the branch lowering compares it `!= 0`, so an
            // `Int`-typed predicate (`file_exists(p)`, `map_has(m, k)`)
            // reads as truthy without an explicit `== 1`.
            if ty != cltypes::I8 && ty != cltypes::I64 {
                return Err(unsupported(mir_id, "non-bool branch condition"));
            }
        }
        Terminator::SwitchEnum {
            subject,
            aggregate,
            targets,
        } => {
            let ty = check_operand(subject)?;
            if ty != cltypes::I64 {
                return Err(unsupported(mir_id, "switch over non-nominal"));
            }
            let subject_ty = match subject {
                Operand::Copy(local) => local_ty(*local),
                _ => return Err(unsupported(mir_id, "switch over non-local")),
            };
            let subject_aggregate = aggregates
                .aggregate_for(subject_ty)
                .ok_or_else(|| unsupported(mir_id, "switch over non-enum"))?;
            let agg = program
                .aggregate(subject_aggregate)
                .expect("validated MIR retains every aggregate");
            if agg.kind != lpp_mir::MirAggregateKind::Enum {
                return Err(unsupported(mir_id, "switch over struct"));
            }
            if subject_aggregate != *aggregate {
                return Err(unsupported(mir_id, "switch aggregate mismatch"));
            }
            let variant_count = program.aggregate_variants(agg).len();
            if program.switch_targets(*targets).len() != variant_count {
                return Err(unsupported(mir_id, "switch enum target count"));
            }
        }
        Terminator::Return(operand) => {
            if let Some(operand) = operand {
                let ty = check_operand(operand)?;
                if machine_type(types, function.return_type) != Some(ty) {
                    return Err(unsupported(mir_id, "return type mismatch"));
                }
            }
        }
        Terminator::Unreachable => {}
    }
    Ok(())
}
