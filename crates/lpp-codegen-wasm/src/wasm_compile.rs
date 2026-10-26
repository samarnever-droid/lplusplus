use super::*;

pub(super) fn compile_module(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn NameResolver,
) -> Result<CompiledModule, CodegenError> {
    let _zzz_out = std::env::var("ZZZ_OUT");
    // Aggregates in stable MirAggregateId order — the order of the
    // generated destructors and their drop-table slots (slot 0 is the
    // "no destructor" no-op).
    let aggregates: Vec<MirAggregateId> = program.aggregates().map(|(id, _)| id).collect();
    let mut layouts: BTreeMap<MirAggregateId, AggregateLayout> = BTreeMap::new();
    let mut drop_slot: BTreeMap<MirAggregateId, u32> = BTreeMap::new();
    for (pos, &id) in aggregates.iter().enumerate() {
        layouts.insert(id, aggregate_layout(program, types, id));
        drop_slot.insert(id, (pos + 1) as u32);
    }
    let aggregate_index = AggregateIndex::build(program);
    let plan = pre_scan(program, types, names, &layouts, &aggregate_index)?;
    // The dispatch TABLE positions are deterministic from the plan:
    // the element segment is [users (user_ids order), task thunks
    // (plan order), closure thunks (plan order)], so thunk i sits at
    // user_count + its plan index. Fill the maps now — before `_start`
    // (and every user body) reads them; the emission loop below only
    // records the MODULE indices.
    // Closures, function values, and tasks ride the ARC heap (capsule /
    // env / tuple / task nodes), so such a program gets a heap and a
    // drop table even without lists or structs.
    let has_heap = !aggregates.is_empty()
        || plan.has_list
        || plan.has_tuple
        || !plan.closures.is_empty()
        || plan.has_function_value
        || plan.has_tasks
        // String-producing builtins (5D2b slice 4) allocate on the ARC
        // bump heap without any aggregate type in the program.
        || plan.needs.contains(&H::Alloc);
    let list_drop_slot = plan.has_list.then(|| (aggregates.len() + 1) as u32);

    // Imports in deterministic order: output, input, then process exit.
    let mut import_fields: Vec<&str> = Vec::new();
    if plan.needs.contains(&H::FdWrite) {
        import_fields.push("fd_write");
    }
    if plan.needs.contains(&H::FdRead) {
        import_fields.push("fd_read");
    }
    import_fields.push("proc_exit");
    let fd_write_idx = import_fields
        .iter()
        .position(|f| *f == "fd_write")
        .unwrap_or(0) as u32;
    let fd_read_idx = import_fields
        .iter()
        .position(|f| *f == "fd_read")
        .unwrap_or(0) as u32;
    let proc_exit_idx = import_fields
        .iter()
        .position(|f| *f == "proc_exit")
        .unwrap() as u32;

    // Build the string pool (sorted by content → deterministic).
    let (string_ptr, pool_bytes) = build_pool(&plan.strings);
    let pool_end = POOL_START + pool_bytes.len() as u32;
    // The bump heap starts 8-aligned, just past the pool.
    let heap_start = (pool_end + 7) & !7;

    // 5D2b: the dispatch table holds every user function (sync, async,
    // closure) in `MirFunctionId` order, followed by the 5D2b slice 3
    // task/closure thunks; a capsule's `code` word is its index there
    // (an async function value points at its task thunk, not the
    // function itself).
    let all_fn_ids: Vec<(MirFunctionId, lpp_mir::MirFunctionKind)> =
        program.functions().map(|(id, f)| (id, f.kind)).collect();
    let user_ids: Vec<MirFunctionId> = all_fn_ids.iter().map(|(id, _)| *id).collect();
    let user_count = user_ids.len() as u32;
    let dispatch: HashMap<MirFunctionId, u32> = user_ids
        .iter()
        .enumerate()
        .map(|(pos, &id)| (id, pos as u32))
        .collect();
    // The thunk dispatch maps are filled while the thunk bodies are
    // emitted (their module indices only exist at that point — the
    // helpers and destructors land between the users and the
    // thunks). The user functions are lowered after the thunks
    // because a task call embeds its thunk's module index.
    let user_dispatch: HashMap<MirFunctionId, u32> = HashMap::new();
    let task_thunk_dispatch: HashMap<MirFunctionId, u32> = HashMap::new();
    let closure_thunk_dispatch: HashMap<MirFunctionId, u32> = HashMap::new();
    let mut thunk_func_indices: Vec<u32> = Vec::new();

    // Closure env destructors (`lpp_drop_c{n}`) take the drop-table slots
    // after the aggregates (and the list destroyer, when present), in
    // `MirFunctionId` order over closures with at least one capture;
    // `lpp_closure_destroy` takes the final slot.
    let mut env_drop_slot: HashMap<MirFunctionId, u32> = HashMap::new();
    let mut next_drop_slot = match list_drop_slot {
        Some(slot) => slot + 1,
        None => (aggregates.len() + 1) as u32,
    };
    for &closure_id in &plan.closures {
        let captures = program.function_captures(program.function(closure_id).unwrap());
        if !captures.is_empty() {
            env_drop_slot.insert(closure_id, next_drop_slot);
            next_drop_slot += 1;
        }
    }
    // Function-value capsules (5D2b slice 2) reuse the same destroyer.
    let closure_destroy_slot = (!plan.closures.is_empty() || plan.has_function_value).then(|| {
        let slot = next_drop_slot;
        next_drop_slot += 1;
        slot
    });
    // Task runtime destructors (5D2b slice 3), after the closure
    // destroyer: `lpp_drop_task`, then `lpp_drop_tuple`.
    let task_drop_slot = plan.has_tasks.then(|| next_drop_slot);
    if plan.has_tasks {
        next_drop_slot += 1;
    }
    let tuple_drop_slot = (plan.has_tasks || plan.has_tuple).then(|| next_drop_slot);

    let mut env = Env {
        program,
        types,
        names,
        user_index: HashMap::new(),
        helper_index: HashMap::new(),
        string_ptr,
        drop_slot: drop_slot.clone(),
        list_drop_slot,
        drop_call_type: 0,
        layouts,
        aggregates: aggregate_index,
        dispatch,
        env_drop_slot,
        closure_destroy_slot,
        closure_call_type: HashMap::new(),
        user_dispatch,
        task_thunk_dispatch,
        closure_thunk_dispatch,
        task_drop_slot,
        tuple_drop_slot,
        task_code_call_type: 0,
        proc_exit_import_idx: proc_exit_idx,
        dispatch_table: if has_heap { 1 } else { 0 },
        value_origins: plan.value_origins,
    };

    // Lowered functions, in stable MirFunctionId order (async
    // functions lower too; their thunks call them directly).
    let base = import_fields.len() as u32;
    for (pos, fn_id) in user_ids.iter().enumerate() {
        env.user_index.insert(*fn_id, base + pos as u32);
        env.user_dispatch.insert(*fn_id, pos as u32);
    }
    // Task thunks: plan order (the drained async main is included).
    for (i, &fn_id) in plan.task_thunks.iter().enumerate() {
        env.task_thunk_dispatch.insert(fn_id, user_count + i as u32);
    }
    // Closure thunks: after the task thunks.
    for (i, &fn_id) in plan.closure_thunks.iter().enumerate() {
        env.closure_thunk_dispatch
            .insert(fn_id, user_count + plan.task_thunks.len() as u32 + i as u32);
    }
    // Needed helpers, fixed order.
    let helpers: Vec<H> = [
        H::FdWrite,
        H::FdRead,
        H::Input,
        H::PrintInt,
        H::PrintBool,
        H::PrintStr,
        H::PrintFloat,
        H::WriteStr,
        H::StrEq,
        H::StrLen,
        H::Alloc,
        H::ArcAlloc,
        H::Retain,
        H::Release,
        H::ListNew,
        H::ListPushI64,
        H::ListPushF64,
        H::ListPushPtr,
        H::ListLen,
        H::ListGetI64,
        H::ListGetF64,
        H::ListGetPtr,
        H::ListSetI64,
        H::ListSetF64,
        H::ListSetPtr,
        H::SliceNewList,
        H::SliceNewStr,
        H::SliceLen,
        H::SliceGetI64,
        H::SliceGetF64,
        H::SliceGetPtr,
        H::StrSliceGet,
        H::StrSliceToStr,
        H::MapNew,
        H::MapEnsure,
        H::MapFindI64,
        H::MapFindStr,
        H::MapPutI64,
        H::MapPutStr,
        H::MapGetI64,
        H::MapGetStr,
        H::MapHasI64,
        H::MapHasStr,
        H::MapRemoveI64,
        H::MapRemoveStr,
        H::MapLen,
        H::TaskNew,
        H::TaskPoll,
        H::TaskAwait,
        H::TaskDestroy,
        // 5D2b slice 4, batch 3: string builtins
        H::StrConcat,
        H::StrContains,
        H::StrStartsWith,
        H::StrEndsWith,
        H::StrFind,
        H::StrReplace,
        H::StrSubstr,
        H::StrRepeat,
        H::StrSplit,
        H::CharAt,
        H::Ord,
        H::Chr,
        H::StrTrim,
        H::StrLower,
        H::StrUpper,
        H::IntToStr,
        H::StrToInt,
        H::FloatToStr,
        H::BoolToStr,
        H::U64ToStr,
        H::U64ToHex,
        H::StrToU64,
        H::Log2,
        H::Exp2,
        H::Pow,
        H::Sin,
        H::Cos,
    ]
    .into_iter()
    .filter(|h| plan.needs.contains(h))
    .collect();
    for (pos, h) in helpers.iter().enumerate() {
        env.helper_index.insert(*h, base + user_count + pos as u32);
    }
    let start_index = base + user_count + helpers.len() as u32;

    // Memory size (at least the pool; the heap grows at runtime).
    let min_pages = pool_end.div_ceil(65536).max(1);

    // Type table: imports first (deterministic), then the destructor
    // signature `(i32) -> ()` (needed by `Release`'s `call_indirect`
    // before the bodies exist), then functions in first-use order,
    // deduped by signature.
    let mut type_list: Vec<(Vec<Val>, Vec<Val>)> = Vec::new();
    let mut type_map: HashMap<(Vec<Val>, Vec<Val>), u32> = HashMap::new();
    let import_tys: Vec<u32> = import_fields
        .iter()
        .map(|f| {
            let (p, r) = if matches!(*f, "fd_write" | "fd_read") {
                (vec![Val::I32; 4], vec![Val::I32])
            } else {
                (vec![Val::I32], vec![])
            };
            reg_type(&mut type_map, &mut type_list, &p, &r)
        })
        .collect();
    if has_heap {
        env.drop_call_type = reg_type(&mut type_map, &mut type_list, &[Val::I32], &[]);
    }
    // 5D2b slice 3: the task-code signature `(env) -> result word`
    // — every task thunk and closure thunk has exactly this shape
    // (the thunk takes the task env tuple; the callee's dispatch
    // index is the call_indirect element index), and
    // `lpp_task_poll` dispatches through it.
    if plan.has_tasks {
        env.task_code_call_type = reg_type(&mut type_map, &mut type_list, &[Val::I32], &[Val::I64]);
    }
    // 5D2b: every sync function's call signature (env, user params...)
    // -> result. Closures and bare function values dispatch through the
    // same table and the same call types (a function value's env word
    // is NULL). The frame's capture slots are `Capture`-kind parameters
    // (not user parameters) — they come from the env, not the stack.
    for &(fn_id, kind) in &all_fn_ids {
        if matches!(kind, lpp_mir::MirFunctionKind::Async) {
            continue;
        }
        let function = program.function(fn_id).unwrap();
        let mut params = vec![Val::I32]; // env
        for &param in program.function_parameters(function) {
            if matches!(
                program.local(param).unwrap().kind,
                lpp_mir::MirLocalKind::Capture
            ) {
                continue;
            }
            let ty = program.local(param).unwrap().ty;
            params.push(local_val(types, ty)?);
        }
        let results = if is_void(types, function.return_type) {
            vec![]
        } else {
            vec![local_val(types, function.return_type)?]
        };
        env.closure_call_type.insert(
            function.ty,
            reg_type(&mut type_map, &mut type_list, &params, &results),
        );
    }

    let mut funcs: Vec<Func> = Vec::new();
    for _ in &user_ids {
        funcs.push(Func {
            name: None,
            params: Vec::new(),
            results: Vec::new(),
            extra: Vec::new(),
            body: Vec::new(),
        });
    }
    // The printing helpers call the 2-arg `lpp_wasm_fd_write` helper, which in
    // turn drives the 4-arg WASI `fd_write` import.
    let fd_helper_idx = env
        .helper_index
        .get(&H::FdWrite)
        .copied()
        .unwrap_or(fd_write_idx);
    for &h in &helpers {
        let (params, results, extra, body) = lower_helper(
            &env,
            h,
            fd_helper_idx,
            fd_write_idx,
            fd_read_idx,
            proc_exit_idx,
        )?;
        funcs.push(Func {
            name: None,
            params,
            results,
            extra,
            body,
        });
    }
    // `_start`
    {
        let mut fb = FB::new(0);
        let main_index = env.user_index.get(&plan.main).unwrap();
        if plan.main_async {
            // The async entry is drained here — the execution boundary
            // holds no references: empty env tuple, task, await,
            // destroy (5D2b slice 3). The awaited result word is the
            // exit code.
            let main_fn = program.function(plan.main).unwrap();
            let tuple_slot = env.tuple_drop_slot.expect("task slots planned");
            let main_thunk = *env
                .task_thunk_dispatch
                .get(&plan.main)
                .expect("async main thunk planned");
            let h = |h: H| *env.helper_index.get(&h).expect("task helpers planned");
            fb.i32c(16);
            fb.i32c(tuple_slot as i64);
            fb.call(h(H::ArcAlloc));
            let tuple = fb.scratch(Val::I32);
            fb.t(tuple);
            fb.g(tuple).i64c(0).store64(0);
            fb.g(tuple).i64c(0).store64(8);
            // The tee'd tuple pointer: the stores re-read the
            // scratch, and TaskNew's env argument does the same —
            // drop the original stack copy before pushing the args.
            fb.op(op::DROP);
            let managed = if is_managed(types, main_fn.return_type) {
                1
            } else {
                0
            };
            fb.i32c(main_thunk as i64);
            fb.g(tuple);
            fb.i32c(managed as i64);
            fb.call(h(H::TaskNew));
            // TaskNew returns the node pointer as an i64 word; narrow
            // for the wasm ABI. Plain local SET (no tee): the drain
            // ends with `proc_exit`, so the stack must be empty at
            // every point before the final exit code.
            let task = fb.scratch(Val::I32);
            fb.op(op::I32_WRAP_I64);
            fb.s(task);
            // Await the drained result word into the scratch, then
            // destroy the node (void).
            fb.g(task);
            fb.call(h(H::TaskAwait));
            let word = fb.scratch(Val::I64);
            fb.s(word);
            fb.g(task);
            fb.call(h(H::TaskDestroy));
            fb.g(word);
            if plan.main_void {
                fb.op(op::DROP);
                fb.i32c(0);
            } else if is_managed(types, main_fn.return_type) {
                // Release the awaiter's share at the boundary (void
                // helper: the wrapped pointer is the argument), then
                // exit 0. The re-read word is the Release argument.
                fb.op(op::I32_WRAP_I64);
                fb.call(h(H::Release));
                fb.i32c(0);
            } else {
                fb.op(op::I32_WRAP_I64);
            }
        } else {
            fb.i32c(0); // env = NULL (dispatch ABI)
            fb.call(*main_index);
            if plan.main_void {
                fb.i32c(0);
            } else {
                fb.op(op::I32_WRAP_I64);
            }
        }
        fb.call(proc_exit_idx);
        // The async drain allocates scratch locals (tuple/task/word);
        // they are function locals (not parameters), so encode them.
        funcs.push(Func {
            name: None,
            params: vec![],
            results: vec![],
            extra: fb.extras.clone(),
            body: fb.body,
        });
    }
    // The 5D2a destructors, in MirAggregateId order (matching their
    // drop-table slots), then the list destroyer, then the no-op "no
    // destructor" that occupies table slot 0.
    let mut drop_func_indices: Vec<u32> = Vec::new();
    for &id in &aggregates {
        let (name, params, results, extra, body) = lower_destructor(&env, id)?;
        drop_func_indices.push(base + funcs.len() as u32);
        funcs.push(Func {
            name: Some(name),
            params,
            results,
            extra,
            body,
        });
    }
    let mut list_drop_index: Option<u32> = None;
    if has_heap && plan.has_list {
        let (name, params, results, extra, body) = lower_list_destructor(&env)?;
        list_drop_index = Some(base + funcs.len() as u32);
        funcs.push(Func {
            name: Some(name),
            params,
            results,
            extra,
            body,
        });
    }
    let mut noop_drop_index: Option<u32> = None;
    if has_heap {
        noop_drop_index = Some(base + funcs.len() as u32);
        let fb = FB::new(1);
        funcs.push(Func {
            name: Some("lpp_drop_none".to_string()),
            params: vec![Val::I32],
            results: vec![],
            extra: fb.extras.clone(),
            body: fb.body,
        });
    }
    // 5D2b: the closure env destructors (`lpp_drop_c{n}`, in
    // `MirFunctionId` order over capturing closures — matching the
    // drop-table slots) and `lpp_closure_destroy`, which releases the
    // capsule's env word (a NULL env is a no-op).
    // Present iff any closure exists (closure creation sets
    // `has_managed`, which pulls the whole ARC set into `needs`).
    let release_idx: Option<u32> = env.helper_index.get(&H::Release).copied();
    let mut closure_dtor_indices: Vec<u32> = Vec::new();
    for &closure_id in &plan.closures {
        if !env.env_drop_slot.contains_key(&closure_id) {
            continue;
        }
        let release_idx = release_idx.expect("Release registered");
        let function = program.function(closure_id).unwrap();
        // The env slots are the closure frame's capture slots
        // (`Capture`-kind parameters, in slot order).
        let capture_frame: Vec<MirLocalId> = program
            .function_parameters(function)
            .iter()
            .copied()
            .filter(|&p| {
                matches!(
                    program.local(p).unwrap().kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .collect();
        let mut fb = FB::new(1);
        for (i, &slot) in capture_frame.iter().enumerate() {
            let ty = program.local(slot).unwrap().ty;
            if is_managed(types, ty) {
                fb.g(0);
                fb.load32(8 * i as u32);
                fb.call(release_idx);
            }
        }
        closure_dtor_indices.push(base + funcs.len() as u32);
        funcs.push(Func {
            name: Some(format!("lpp_drop_c{}", closure_id.raw())),
            params: vec![Val::I32],
            results: vec![],
            extra: fb.extras.clone(),
            body: fb.body,
        });
    }
    let mut closure_destroy_index: Option<u32> = None;
    if let Some(_slot) = closure_destroy_slot {
        let release_idx = release_idx.expect("Release registered");
        closure_destroy_index = Some(base + funcs.len() as u32);
        let mut fb = FB::new(1);
        fb.g(0).load64(8).op(op::I32_WRAP_I64).call(release_idx);
        funcs.push(Func {
            name: Some("lpp_closure_destroy".to_string()),
            params: vec![Val::I32],
            results: vec![],
            extra: fb.extras.clone(),
            body: fb.body,
        });
    }
    // 5D2b slice 3: `lpp_drop_task` releases the task's env tuple and,
    // once resolved, the managed result; `lpp_drop_tuple` releases the
    // managed argument slots (bit `i` of the mask → slot at `16+8i`).
    let mut task_dtor_index: Option<u32> = None;
    let mut tuple_dtor_index: Option<u32> = None;
    if plan.has_tasks {
        let release_idx = release_idx.expect("Release registered");
        // ── lpp_drop_task ──
        task_dtor_index = Some(base + funcs.len() as u32);
        {
            let mut fb = FB::new(1);
            // Release is void (its pass-through is a caller
            // convenience) — the destructor drops nothing.
            fb.g(0).load64(8).op(op::I32_WRAP_I64).call(release_idx);
            fb.g(0).load32(24).i32c(2).op(op::I32_EQ);
            fb.g(0).load32(28).i32c(1).op(op::I32_EQ);
            fb.op(op::I32_AND);
            fb.if_();
            fb.g(0).load64(16).op(op::I32_WRAP_I64).call(release_idx);
            fb.end();
            funcs.push(Func {
                name: Some("lpp_drop_task".to_string()),
                params: vec![Val::I32],
                results: vec![],
                extra: fb.extras.clone(),
                body: fb.body,
            });
        }
    }
    if plan.has_tasks || plan.has_tuple {
        let release_idx = release_idx.expect("Release registered");
        // ── lpp_drop_tuple ──
        tuple_dtor_index = Some(base + funcs.len() as u32);
        {
            let mut fb = FB::new(1);
            let mask = fb.scratch(Val::I64);
            fb.g(0).load64(0).t(mask);
            // The tee's stack copy is never re-read (every use goes
            // through the local) — drop it.
            fb.op(op::DROP);
            for i in 0..64 {
                fb.g(mask)
                    .i64c(i as i64)
                    .op(op::I64_SHR_U)
                    .i64c(1)
                    .op(op::I64_AND)
                    .i64c(0)
                    .op(op::I64_NE);
                fb.if_();
                fb.g(0)
                    .load64(16 + 8 * i as u32)
                    .op(op::I32_WRAP_I64)
                    .call(release_idx);
                fb.end();
            }
            funcs.push(Func {
                name: Some("lpp_drop_tuple".to_string()),
                params: vec![Val::I32],
                results: vec![],
                extra: fb.extras.clone(),
                body: fb.body,
            });
        }
    }

    // 5D2b slice 3: the thunks. `__lpp_task_thunk{n}` adapts the task
    // env tuple to a call of the (async or zero-parameter sync)
    // function; `__lpp_closure_thunk{n}` adapts it to a call of the
    // zero-parameter closure. Both are the task-code shape
    // `(env) -> i64 result word` and take dispatch slots after the
    // user functions. The dispatch maps are filled here: this is the
    // first point at which the thunk module indices exist.
    //
    // The user functions lower AFTER the thunks (a task call embeds
    // its thunk's module index), but their `funcs` slots come first:
    // reserve the user positions, fill the slots after the thunk
    // bodies are known.
    // Emission mirrors the plan order the maps above were built
    // from (task thunks, then closure thunks): the i-th emitted
    // thunk lands in table slot user_count + i.
    for &thunk_id in &plan.task_thunks {
        let (params, results, extra, body) = lower_task_thunk(&env, thunk_id)?;
        let index = base + funcs.len() as u32;
        thunk_func_indices.push(index);
        funcs.push(Func {
            name: Some(format!("__lpp_task_thunk{}", thunk_id.raw())),
            params,
            results,
            extra,
            body,
        });
    }
    for &thunk_id in &plan.closure_thunks {
        let (params, results, extra, body) = lower_closure_thunk(&env, thunk_id)?;
        let index = base + funcs.len() as u32;
        thunk_func_indices.push(index);
        funcs.push(Func {
            name: Some(format!("__lpp_closure_thunk{}", thunk_id.raw())),
            params,
            results,
            extra,
            body,
        });
    }
    // Now that every dispatch index exists, lower the user functions
    // into their reserved slots.
    for (slot, fn_id) in funcs.iter_mut().zip(user_ids.iter()) {
        let (name, params, results, extra, body) = lower_user_function(&env, *fn_id)?;
        *slot = Func {
            name,
            params,
            results,
            extra,
            body,
        };
    }

    // The funcref table: [no-op, drops in MirAggregateId order, list,
    // closure env dtors, closure destroyer, task dtor, tuple dtor].
    let mut table_fns: Vec<u32> = Vec::new();
    if has_heap {
        if let Some(noop) = noop_drop_index {
            table_fns.push(noop);
        }
        table_fns.extend(drop_func_indices.iter().copied());
        if let Some(list_drop) = list_drop_index {
            table_fns.push(list_drop);
        }
        table_fns.extend(closure_dtor_indices.iter().copied());
        if let Some(destroy) = closure_destroy_index {
            table_fns.push(destroy);
        }
        if let Some(dt) = task_dtor_index {
            table_fns.push(dt);
        }
        if let Some(tt) = tuple_dtor_index {
            table_fns.push(tt);
        }
    }

    let func_tys: Vec<u32> = funcs
        .iter()
        .map(|f| reg_type(&mut type_map, &mut type_list, &f.params, &f.results))
        .collect();

    // ---- Type section (1) ----
    let mut tsec = Vec::new();
    uleb(&mut tsec, type_list.len() as u64);
    for (params, results) in &type_list {
        tsec.push(0x60);
        uleb(&mut tsec, params.len() as u64);
        for p in params {
            tsec.push(p.byte());
        }
        uleb(&mut tsec, results.len() as u64);
        for r in results {
            tsec.push(r.byte());
        }
    }

    // ---- Import section (2) ----
    let mut isec = Vec::new();
    uleb(&mut isec, import_fields.len() as u64);
    for (i, field) in import_fields.iter().enumerate() {
        enc_name(&mut isec, "wasi_snapshot_preview1");
        enc_name(&mut isec, field);
        isec.push(0x00); // func import
        uleb(&mut isec, import_tys[i] as u64);
    }

    // ---- Function section (3) ----
    let mut fsec = Vec::new();
    uleb(&mut fsec, funcs.len() as u64);
    for idx in &func_tys {
        uleb(&mut fsec, *idx as u64);
    }

    // ---- Table section (4) ---- (5D2a destructor table; 5D2b adds the
    // dispatch table as table 1). Bare function values (slice 2) and
    // task thunks (slice 3) also need the dispatch table.
    let has_closures = !plan.closures.is_empty() || plan.has_function_value || plan.has_tasks;
    let dispatch_len = user_ids.len() + thunk_func_indices.len();
    let mut table_sec: Vec<u8> = Vec::new();
    if has_heap && has_closures {
        uleb(&mut table_sec, 2);
        table_sec.push(0x70); // funcref
        table_sec.push(0x00); // limits: min only
        uleb(&mut table_sec, table_fns.len() as u64);
        table_sec.push(0x70);
        table_sec.push(0x00);
        uleb(&mut table_sec, dispatch_len as u64);
    } else if has_heap {
        uleb(&mut table_sec, 1);
        table_sec.push(0x70); // funcref
        table_sec.push(0x00); // limits: min only
        uleb(&mut table_sec, table_fns.len() as u64);
    } else if has_closures {
        uleb(&mut table_sec, 1);
        table_sec.push(0x70); // funcref (dispatch only)
        table_sec.push(0x00);
        uleb(&mut table_sec, dispatch_len as u64);
    } else {
        uleb(&mut table_sec, 0);
    }

    // ---- Memory section (5) ----
    let mut msec = Vec::new();
    uleb(&mut msec, 1);
    msec.push(0x00); // limits: min only
    uleb(&mut msec, min_pages as u64);

    // ---- Global section (6) ---- (heap bump pointer + ARC size-class
    // free-list heads). The heap pointer remains global 0; ARC bins are
    // globals 1..=27 and are emitted only in heap-using modules.
    let mut gsec = Vec::new();
    if has_heap {
        uleb(&mut gsec, 1 + ARC_FREE_BIN_COUNT as u64);
        gsec.push(0x7f); // heap pointer: i32
        gsec.push(0x01); // mutable
        gsec.push(0x41); // i32.const
        sleb(&mut gsec, heap_start as i64);
        gsec.push(0x0b); // end
        for _ in 0..ARC_FREE_BIN_COUNT {
            gsec.push(0x7f); // free-list head: i32
            gsec.push(0x01); // mutable
            gsec.push(0x41);
            sleb(&mut gsec, 0);
            gsec.push(0x0b);
        }
    } else {
        uleb(&mut gsec, 0);
    }

    // ---- Export section (7) ----
    let mut export_vec: Vec<(String, u32, u8)> = Vec::new(); // (name, index, kind)
    export_vec.push(("_start".to_string(), start_index, 0x00));
    export_vec.push(("memory".to_string(), 0, 0x02));
    for (pos, f) in funcs.iter().enumerate().take(user_count as usize) {
        if let Some(name) = &f.name {
            export_vec.push((name.clone(), base + pos as u32, 0x00));
        }
    }
    let mut esec = Vec::new();
    uleb(&mut esec, export_vec.len() as u64);
    for (name, index, kind) in &export_vec {
        enc_name(&mut esec, name);
        esec.push(*kind);
        uleb(&mut esec, *index as u64);
    }

    // ---- Code section (10) ----
    let mut bodies: Vec<Vec<u8>> = Vec::new();
    for f in &funcs {
        let mut body = Vec::new();
        enc_locals(&mut body, &f.extra);
        body.extend_from_slice(&f.body);
        body.push(op::END);
        bodies.push(body);
    }
    let mut csec = Vec::new();
    uleb(&mut csec, bodies.len() as u64);
    for body in &bodies {
        uleb(&mut csec, body.len() as u64);
        csec.extend_from_slice(body);
    }

    // ---- Element section (9) ---- (destructor table; 5D2b dispatch
    // table in `MirFunctionId` order)
    let mut e9sec = Vec::new();
    if has_heap || has_closures {
        let entries = (has_heap as u64) + (has_closures as u64);
        uleb(&mut e9sec, entries);
        if has_heap {
            e9sec.push(0x00); // active, table 0
            e9sec.push(0x41); // i32.const
            sleb(&mut e9sec, 0);
            e9sec.push(0x0b); // end
            uleb(&mut e9sec, table_fns.len() as u64);
            for idx in &table_fns {
                uleb(&mut e9sec, *idx as u64);
            }
        }
        if has_closures {
            // The dispatch table: table 1 (table 0 when there is no
            // drop table). Element flags: 0x00 = active in table 0,
            // 0x02 = active with an explicit table index.
            if has_heap {
                e9sec.push(0x02); // active, explicit table index
                e9sec.push(0x01); // table 1
            } else {
                e9sec.push(0x00); // active, table 0
            }
            e9sec.push(0x41); // i32.const
            sleb(&mut e9sec, 0);
            e9sec.push(0x0b); // end
            if has_heap {
                // The 0x02 encoding carries an element-kind byte after
                // the offset expression (0x00 = funcref).
                e9sec.push(0x00);
            }
            // User functions first (their MODULE indices occupy the
            // first user_count table positions), then the thunks in
            // emission order (thunk i sits at table position
            // user_count + i — exactly what the dispatch maps store).
            uleb(&mut e9sec, dispatch_len as u64);
            for &fn_id in &user_ids {
                uleb(&mut e9sec, env.user_index[&fn_id] as u64);
            }
            for &idx in &thunk_func_indices {
                uleb(&mut e9sec, idx as u64);
            }
        }
    } else {
        uleb(&mut e9sec, 0);
    }

    // ---- Data section (11) ----
    let mut dsec = Vec::new();
    uleb(&mut dsec, 1);
    dsec.push(0x00); // active, memory 0
    dsec.push(0x41); // i32.const
    sleb(&mut dsec, POOL_START as i64);
    dsec.push(0x0b); // end
    uleb(&mut dsec, pool_bytes.len() as u64);
    dsec.extend_from_slice(&pool_bytes);

    // ---- Name section (custom, 0) ---- (function names for the census;
    // user functions plus the generated destructors)
    let mut named: Vec<(String, u32)> = Vec::new();
    for (pos, f) in funcs.iter().enumerate() {
        if let Some(name) = &f.name {
            named.push((name.clone(), base + pos as u32));
        }
    }
    let mut name_sec: Vec<u8> = Vec::new();
    if !named.is_empty() {
        enc_name(&mut name_sec, "name");
        let mut sub = Vec::new();
        uleb(&mut sub, named.len() as u64);
        for (name, index) in &named {
            enc_name(&mut sub, name);
            uleb(&mut sub, *index as u64);
        }
        name_sec.push(1); // subsection: function names
        uleb(&mut name_sec, sub.len() as u64);
        name_sec.extend_from_slice(&sub);
    }

    // ---- Assemble the module ----
    let mut module = Vec::new();
    module.extend_from_slice(&b"\0asm"[..]);
    uleb(&mut module, 1); // version
    uleb(&mut module, 0); // (reserved low byte)
    uleb(&mut module, 0);
    uleb(&mut module, 0);
    enc_section(&mut module, 1, &tsec);
    enc_section(&mut module, 2, &isec);
    enc_section(&mut module, 3, &fsec);
    enc_section(&mut module, 4, &table_sec);
    enc_section(&mut module, 5, &msec);
    enc_section(&mut module, 6, &gsec);
    enc_section(&mut module, 7, &esec);
    enc_section(&mut module, 9, &e9sec);
    enc_section(&mut module, 10, &csec);
    enc_section(&mut module, 11, &dsec);
    if !name_sec.is_empty() {
        enc_section(&mut module, 0, &name_sec);
    }

    // Symbol sets for the census.
    let mut exported_symbols = BTreeSet::new();
    exported_symbols.insert("_start".to_string());
    exported_symbols.insert("memory".to_string());
    for f in funcs.iter().take(user_count as usize) {
        if let Some(name) = &f.name {
            exported_symbols.insert(name.clone());
        }
    }
    let mut imported_symbols = BTreeSet::new();
    for field in &import_fields {
        imported_symbols.insert(format!("wasi_snapshot_preview1.{field}"));
    }

    if let Ok(path) = _zzz_out {
        std::fs::write(path, &module).unwrap();
    }

    Ok(CompiledModule {
        target: Target::Wasm32Wasi,
        object: module,
        exported_symbols,
        imported_symbols,
        link_libraries: program.link_libraries(),
        entry: Some("main".to_string()),
    })
}

// ---------------------------------------------------------------------------
// Pre-scan: typed rejections + helper/string collection.
// ---------------------------------------------------------------------------
