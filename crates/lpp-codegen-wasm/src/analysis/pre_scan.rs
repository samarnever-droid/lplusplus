use super::*;

pub(crate) fn pre_scan(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn NameResolver,
    layouts: &BTreeMap<MirAggregateId, AggregateLayout>,
    aggregates: &AggregateIndex,
) -> Result<Plan, CodegenError> {
    let mut needs = BTreeSet::new();
    let mut strings = BTreeSet::new();
    let mut main: Option<MirFunctionId> = None;
    let mut main_void = false;
    let mut has_list = false;
    let mut has_tuple = false;
    let mut has_managed = false;
    let mut has_function_value = false;
    let mut has_tasks = false;
    let mut closures: Vec<MirFunctionId> = Vec::new();
    let mut task_thunks: Vec<MirFunctionId> = Vec::new();
    let mut closure_thunks: Vec<MirFunctionId> = Vec::new();
    let mut value_origins: BTreeMap<(MirFunctionId, MirLocalId), MirFunctionId> = BTreeMap::new();
    let mut main_async = false;

    for (fn_id, function) in program.functions() {
        if matches!(function.kind, lpp_mir::MirFunctionKind::Closure) {
            closures.push(fn_id);
        }
        let is_main = function.name.and_then(|s| names.resolve(s.raw())) == Some("main");
        if is_main {
            main = Some(fn_id);
            main_void = is_void(types, function.return_type);
            // An async entry is drained by `_start` through its task
            // thunk (the wrapper never calls it directly), so the
            // thunk must exist even though no instruction names it.
            main_async = matches!(function.kind, lpp_mir::MirFunctionKind::Async);
            if main_async {
                has_tasks = true;
                task_thunks.push(fn_id);
            }
        }

        let mut lower = ScanLower {
            program,
            types,
            fn_id,
            needs: &mut needs,
            strings: &mut strings,
            has_list: &mut has_list,
            has_tuple: &mut has_tuple,
            has_managed: &mut has_managed,
            has_function_value: &mut has_function_value,
            has_tasks: &mut has_tasks,
            task_thunks: &mut task_thunks,
            closure_thunks: &mut closure_thunks,
            value_origins: &mut value_origins,
            layouts,
            aggregates,
        };
        for &block_id in program.function_blocks(function) {
            let block = program.block(block_id).unwrap();
            for &instr in program.block_instructions(block) {
                let instruction = program.instruction(instr).unwrap();
                check_instruction(&mut lower, &instruction.kind)?;
            }
            check_terminator(&mut lower, &block.terminator)?;
        }
    }

    let main = main.ok_or_else(|| {
        CodegenError::new(
            None,
            CodegenErrorKind::ObjectEmissionFailed(
                "program defines no source-level `main`".to_string(),
            ),
        )
    })?;

    // `fd_write` is pulled in by any printing helper.
    if needs.contains(&H::PrintInt)
        || needs.contains(&H::PrintBool)
        || needs.contains(&H::PrintStr)
        || needs.contains(&H::PrintFloat)
        || needs.contains(&H::WriteStr)
    {
        needs.insert(H::FdWrite);
    }
    // The task runtime (5D2b slice 3) rides the ARC heap: task nodes
    // and env tuples are ARC nodes with tabled destructors.
    if has_tasks {
        has_managed = true;
        needs.insert(H::TaskNew);
        needs.insert(H::TaskPoll);
        needs.insert(H::TaskAwait);
        needs.insert(H::TaskDestroy);
    }
    // The ARC machinery is whole-program: one heap, one table, and the
    // destructor set are shared by every managed value.
    if has_managed || has_list {
        for h in [H::Alloc, H::ArcAlloc, H::Retain, H::Release] {
            needs.insert(h);
        }
    }
    // The float formatter's literals live in the pool (deduped by
    // content, deterministic).
    if needs.contains(&H::PrintFloat) {
        for literal in ["NaN\n", "-inf\n", "inf\n", "-", ".", ".000000\n"] {
            strings.insert(literal.to_string());
        }
    }
    // The string-builtin helpers compose their output from pooled
    // fragments (deduped by content, deterministic).
    if needs.contains(&H::FloatToStr) || needs.contains(&H::BoolToStr) {
        for literal in [
            "true", "false", "nan", "inf", "-inf", "-0", "0", "-", ".", "e", "+",
        ] {
            strings.insert(literal.to_string());
        }
    }

    task_thunks.sort_unstable();
    task_thunks.dedup();
    closure_thunks.sort_unstable();
    closure_thunks.dedup();

    Ok(Plan {
        needs,
        strings,
        main,
        main_void,
        has_list,
        has_tuple,
        closures,
        has_function_value,
        task_thunks,
        closure_thunks,
        has_tasks,
        main_async,
        value_origins,
    })
}
