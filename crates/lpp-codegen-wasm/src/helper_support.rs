use super::*;

pub(super) fn block_layout(
    program: &MirProgram,
    function: &lpp_mir::MirFunction,
) -> (Vec<(u32, BasicBlockId)>, HashMap<BasicBlockId, u32>) {
    let entry = function.entry;
    let mut seen = HashSet::new();
    let mut stack: Vec<(BasicBlockId, bool)> = vec![(entry, false)];
    let mut post: Vec<BasicBlockId> = Vec::new();
    while let Some((blk, processed)) = stack.pop() {
        if processed {
            post.push(blk);
        } else if seen.insert(blk) {
            stack.push((blk, true));
            if let Some(block) = program.block(blk) {
                for succ in successors(&block.terminator, program) {
                    if !seen.contains(&succ) {
                        stack.push((succ, false));
                    }
                }
            }
        }
    }
    post.reverse();
    let order = post
        .iter()
        .enumerate()
        .map(|(i, &b)| (i as u32, b))
        .collect();
    let positions: HashMap<BasicBlockId, u32> = post
        .iter()
        .enumerate()
        .map(|(i, &b)| (b, i as u32))
        .collect();
    (order, positions)
}

pub(super) fn successors(terminator: &Terminator, program: &MirProgram) -> Vec<BasicBlockId> {
    match terminator {
        Terminator::Goto(t) => vec![*t],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::SwitchEnum { targets, .. } => program.switch_targets(*targets).to_vec(),
        Terminator::Return(_) | Terminator::Unreachable => vec![],
    }
}

// ---------------------------------------------------------------------------
// Runtime helpers.
// ---------------------------------------------------------------------------

/// Box a call result of user type `ty` into the task result word
/// (`i64`): Int as-is, Float bit-cast, Bool/Char/managed zero-extended,
/// Void as 0.
pub(super) fn box_result_word(fb: &mut FB, types: &TypeInterner, ty: TypeId) {
    if is_void(types, ty) {
        fb.i64c(0);
        return;
    }
    match local_val(types, ty).expect("thunk result types lower") {
        Val::I64 => {}
        Val::F64 => {
            fb.op(op::I64_REINTERPRET_F64);
        }
        Val::I32 => {
            fb.op(op::I64_EXTEND_I32_U);
        }
    }
}

/// `__lpp_task_thunk{n}` (5D2b slice 3) — the task code for the async
/// (or zero-parameter sync, for `spawn`) function `n`: load the
/// argument slots of the task env tuple (offset `16+8i`; managed slots
/// retained for the call), call the function with `env = NULL`, and
/// box the result into the task result word.
pub(super) fn lower_task_thunk(
    env: &Env<'_>,
    function_id: MirFunctionId,
) -> Result<(Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let function = env.program.function(function_id).unwrap();
    let user_params: Vec<MirLocalId> = env
        .program
        .function_parameters(function)
        .iter()
        .copied()
        .filter(|&p| {
            !matches!(
                env.program.local(p).unwrap().kind,
                lpp_mir::MirLocalKind::Capture
            )
        })
        .collect();
    let callee_index = *env
        .user_index
        .get(&function_id)
        .ok_or_else(|| unsupported("undispatched thunk callee", Some(function_id)))?;
    let mut fb = FB::new(1);
    // The task env tuple is the thunk's parameter (local 0); the
    // callee's dispatch-ABI env is NULL (plain functions ignore it).
    fb.i32c(0);
    for (i, &param) in user_params.iter().enumerate() {
        let ty = env.program.local(param).unwrap().ty;
        let offset = 16 + 8 * i as u32;
        if is_managed(env.types, ty) {
            // Managed slot: i64 word → i32 pointer, retained for the
            // call (the tuple keeps its own reference).
            fb.g(0).load64(offset).op(op::I32_WRAP_I64);
            fb.call(env.helper_index[&H::Retain]);
        } else {
            match local_val(env.types, ty)? {
                Val::I64 => {
                    fb.g(0).load64(offset);
                }
                Val::F64 => {
                    fb.g(0).loadf64(offset);
                }
                Val::I32 => {
                    fb.g(0).load64(offset).op(op::I32_WRAP_I64);
                }
            }
        }
    }
    // The thunk takes (env) -> i64 result word; the callee's
    // dispatch index is the call_indirect element index, applied by
    // the TaskPoll pass.
    fb.call(callee_index);
    box_result_word(&mut fb, env.types, function.return_type);
    Ok((vec![Val::I32], vec![Val::I64], fb.extras, fb.body))
}

/// `__lpp_closure_thunk{n}` (5D2b slice 3) — the task code for the
/// zero-parameter closure `n`: the wrapper tuple's slot 16 holds the
/// closure env; invoke the closure through the dispatch table and box
/// the result into the task result word.
pub(super) fn lower_closure_thunk(
    env: &Env<'_>,
    function_id: MirFunctionId,
) -> Result<(Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let function = env.program.function(function_id).unwrap();
    let call_type = *env
        .closure_call_type
        .get(&function.ty)
        .ok_or_else(|| unsupported("unregistered closure thunk type", Some(function_id)))?;
    let mut fb = FB::new(1);
    // The closure env: the wrapper tuple's slot 16 holds the closure
    // capsule pointer; the env word is the capsule's env slot. (A
    // bare-function wrapper has slot 16 = 0: NULL env.)
    let cap = fb.scratch(Val::I32);
    fb.g(0).load64(16).op(op::I32_WRAP_I64).s(cap);
    // The capsule's code word is the closure's TABLE position
    // (user_count + its user slot). call_indirect pops the element
    // index LAST (top of the stack), so push the env first and the
    // index on top.
    fb.g(cap).load64(8).op(op::I32_WRAP_I64);
    fb.g(cap).load64(0).op(op::I32_WRAP_I64);
    fb.call_indirect(call_type, env.dispatch_table);
    box_result_word(&mut fb, env.types, function.return_type);
    Ok((vec![Val::I32], vec![Val::I64], fb.extras, fb.body))
}

pub(super) fn lower_helper(
    env: &Env<'_>,
    h: H,
    fd_helper_idx: u32,
    fd_write_import_idx: u32,
    fd_read_import_idx: u32,
    proc_exit_import_idx: u32,
    import_index: &BTreeMap<&str, u32>,
) -> Result<(Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let mut fb = match h {
        H::FdWrite | H::FdRead => FB::new(2),
        H::Input => FB::new(0),
        H::PrintInt => FB::new(1),
        H::PrintBool => FB::new(1),
        H::PrintStr => FB::new(1),
        H::PrintFloat => FB::new(1),
        H::WriteStr => FB::new(1),
        // (path_ptr) / (path_ptr, payload_ptr) / (path_ptr)
        H::FileExists => FB::new(1),
        H::ReadFile => FB::new(1),
        H::WriteFile | H::AppendFile => FB::new(2),
        H::DeleteFile => FB::new(1),
        H::StrEq => FB::new(2),
        H::StrLen => FB::new(1),
        H::Alloc => FB::new(1),
        H::ArcAlloc => FB::new(2),
        H::Retain => FB::new(1),
        H::Release => FB::new(1),
        H::ListNew => FB::new(1),
        H::ListPushI64 => FB::new(2),
        H::ListPushF64 => FB::new(2),
        H::ListPushPtr => FB::new(2),
        H::ListLen => FB::new(1),
        H::ListGetI64 => FB::new(2),
        H::ListGetF64 => FB::new(2),
        H::ListGetPtr => FB::new(2),
        H::ListSetI64 => FB::new(3),
        H::ListSetF64 => FB::new(3),
        H::ListSetPtr => FB::new(3),
        H::SliceNewList | H::SliceNewStr => FB::new(3),
        H::SliceLen | H::StrSliceToStr => FB::new(1),
        H::SliceGetI64 | H::SliceGetF64 | H::SliceGetPtr | H::StrSliceGet => FB::new(2),
        H::MapNew | H::MapEnsure | H::MapLen => FB::new(1),
        H::MapFindI64
        | H::MapFindStr
        | H::MapGetI64
        | H::MapGetStr
        | H::MapHasI64
        | H::MapHasStr
        | H::MapRemoveI64
        | H::MapRemoveStr => FB::new(2),
        H::MapPutI64 | H::MapPutStr => FB::new(3),
        H::TaskNew => FB::new(3),
        H::TaskPoll => FB::new(1),
        H::TaskAwait => FB::new(1),
        H::TaskDestroy => FB::new(1),
        // ── 5D2b slice 4, batch 3: string builtins ──
        H::StrConcat => FB::new(2),
        H::StrContains => FB::new(2),
        H::StrStartsWith => FB::new(2),
        H::StrEndsWith => FB::new(2),
        H::StrFind => FB::new(2),
        H::StrReplace | H::StrSubstr => FB::new(3),
        H::StrRepeat | H::CharAt => FB::new(2),
        H::StrSplit => FB::new(2),
        H::Ord => FB::new(1),
        H::Chr => FB::new(1),
        H::StrTrim => FB::new(1),
        H::StrLower => FB::new(1),
        H::StrUpper => FB::new(1),
        H::IntToStr => FB::new(1),
        H::StrToInt => FB::new(1),
        H::FloatToStr => FB::new(1),
        H::BoolToStr => FB::new(1),
        H::U64ToStr => FB::new(1),
        H::U64ToHex => FB::new(1),
        H::StrToU64 => FB::new(1),
        H::Log2 | H::Exp2 | H::Sin | H::Cos => FB::new(1),
        H::Pow => FB::new(2),
    };
    let results = match h {
        H::FdRead => vec![Val::I32],
        H::Input => vec![Val::I32],
        // A file read yields a fresh ARC string pointer (wrapped to the i64
        // word the helper returns); the rest report an i64 errno/flag.
        H::FileExists | H::ReadFile | H::WriteFile | H::AppendFile | H::DeleteFile => {
            vec![Val::I64]
        }
        H::StrEq => vec![Val::I32],
        H::StrLen => vec![Val::I64],
        // Retain passes the value through (callers keep using it).
        H::Alloc | H::ArcAlloc | H::Retain | H::ListNew | H::ListGetPtr => vec![Val::I32],
        H::ListLen | H::ListGetI64 | H::SliceLen | H::SliceGetI64 => vec![Val::I64],
        H::ListGetF64 | H::SliceGetF64 => vec![Val::F64],
        H::SliceNewList
        | H::SliceNewStr
        | H::SliceGetPtr
        | H::StrSliceGet
        | H::StrSliceToStr
        | H::MapNew => vec![Val::I32],
        H::MapFindI64
        | H::MapFindStr
        | H::MapGetI64
        | H::MapGetStr
        | H::MapHasI64
        | H::MapHasStr
        | H::MapLen => vec![Val::I64],
        // TaskNew returns the node pointer as an i64 word (zero-
        // extended in the body; callers narrow for the wasm ABI).
        H::TaskNew => vec![Val::I64],
        H::TaskPoll | H::TaskAwait => vec![Val::I64],
        // ── 5D2b slice 4, batch 3: string builtins ──
        H::StrConcat
        | H::StrReplace
        | H::StrSubstr
        | H::StrRepeat
        | H::CharAt
        | H::Chr
        | H::StrTrim
        | H::StrLower
        | H::StrUpper
        | H::IntToStr
        | H::FloatToStr
        | H::BoolToStr
        | H::U64ToStr
        | H::U64ToHex => vec![Val::I32],
        H::StrSplit => vec![Val::I32],
        H::StrContains | H::StrStartsWith | H::StrEndsWith => vec![Val::I32],
        H::StrFind | H::StrToInt | H::StrToU64 | H::Ord => vec![Val::I64],
        H::Log2 | H::Exp2 | H::Pow | H::Sin | H::Cos => vec![Val::F64],
        _ => vec![],
    };
    let params = match h {
        H::FdWrite | H::FdRead => vec![Val::I32, Val::I32],
        H::Input => vec![],
        H::PrintInt => vec![Val::I64],
        H::PrintBool => vec![Val::I32],
        H::PrintStr => vec![Val::I32],
        H::PrintFloat => vec![Val::F64],
        H::WriteStr => vec![Val::I32],
        // A path (and, for a write, the payload) is a string pointer — the
        // same `Val::I32` shape `print_str` takes.
        H::FileExists | H::ReadFile | H::DeleteFile => vec![Val::I32],
        H::WriteFile | H::AppendFile => vec![Val::I32, Val::I32],
        H::StrEq => vec![Val::I32, Val::I32],
        H::StrLen => vec![Val::I32],
        H::Alloc => vec![Val::I32],
        H::ArcAlloc => vec![Val::I32, Val::I32],
        H::Retain => vec![Val::I32],
        H::Release => vec![Val::I32],
        H::ListNew => vec![Val::I32],
        H::ListPushI64 | H::ListGetI64 => vec![Val::I32, Val::I64],
        H::ListPushF64 => vec![Val::I32, Val::F64],
        // The getter's second parameter is the index (an Int, i64) —
        // the bodies multiply it by 8 in i64.
        H::ListGetF64 | H::ListGetPtr => vec![Val::I32, Val::I64],
        H::ListPushPtr => vec![Val::I32, Val::I32],
        H::ListLen => vec![Val::I32],
        H::ListSetI64 => vec![Val::I32, Val::I64, Val::I64],
        H::ListSetF64 => vec![Val::I32, Val::I64, Val::F64],
        H::ListSetPtr => vec![Val::I32, Val::I64, Val::I32],
        H::SliceNewList | H::SliceNewStr => vec![Val::I32, Val::I64, Val::I64],
        H::SliceLen | H::StrSliceToStr => vec![Val::I32],
        H::SliceGetI64 | H::SliceGetF64 | H::SliceGetPtr | H::StrSliceGet => {
            vec![Val::I32, Val::I64]
        }
        H::MapNew => vec![Val::I32],
        H::MapEnsure | H::MapLen => vec![Val::I32],
        H::MapFindI64 | H::MapGetI64 | H::MapHasI64 | H::MapRemoveI64 => vec![Val::I32, Val::I64],
        H::MapFindStr | H::MapGetStr | H::MapHasStr | H::MapRemoveStr => vec![Val::I32, Val::I32],
        H::MapPutI64 => vec![Val::I32, Val::I64, Val::I64],
        H::MapPutStr => vec![Val::I32, Val::I32, Val::I64],
        H::TaskNew => vec![Val::I32, Val::I32, Val::I32],
        H::TaskPoll => vec![Val::I32],
        H::TaskAwait => vec![Val::I32],
        H::TaskDestroy => vec![Val::I32],
        // ── 5D2b slice 4, batch 3: string builtins ──
        H::StrConcat | H::StrContains | H::StrStartsWith | H::StrEndsWith | H::StrFind => {
            vec![Val::I32, Val::I32]
        }
        H::StrReplace => vec![Val::I32, Val::I32, Val::I32],
        H::StrSubstr => vec![Val::I32, Val::I64, Val::I64],
        H::StrRepeat | H::CharAt => vec![Val::I32, Val::I64],
        H::StrSplit => vec![Val::I32, Val::I64],
        H::Ord => vec![Val::I32],
        H::Chr => vec![Val::I64],
        H::StrTrim | H::StrLower | H::StrUpper | H::StrToInt | H::StrToU64 => {
            vec![Val::I32]
        }
        H::IntToStr | H::U64ToStr | H::U64ToHex => vec![Val::I64],
        H::FloatToStr => vec![Val::F64],
        H::BoolToStr => vec![Val::I32],
        H::Log2 | H::Exp2 | H::Sin | H::Cos => vec![Val::F64],
        H::Pow => vec![Val::F64, Val::F64],
    };
    emit_helper_body(
        &mut fb,
        h,
        env,
        fd_helper_idx,
        fd_write_import_idx,
        fd_read_import_idx,
        proc_exit_import_idx,
        import_index,
    )?;
    Ok((params, results, fb.extras, fb.body))
}

/// Store `w` digits of `src` (i64 local), most-significant first with
/// leading-zero padding, at NUM_BUF+pos..+pos+w. Digits are extracted
/// least-significant first and written to the final slots in place.
/// Bounded: exactly `w` iterations (w <= 6 by construction).
pub(super) fn emit_digits(fb: &mut FB, pos: u32, w: u32, k: u32, d: u32, src: u32) {
    fb.i32c(0).s(k);
    fb.loop_(); // $dg
    fb.g(k).g(w).op(op::I32_LT_U).if_(); // $dgi
    fb.g(src)
        .i64c(10)
        .op(op::I64_REM_U)
        .op(op::I32_WRAP_I64)
        .s(d);
    fb.g(src).i64c(10).op(op::I64_DIV_U).s(src);
    fb.i32c(NUM_BUF as i64)
        .g(pos)
        .op(op::I32_ADD)
        .g(w)
        .op(op::I32_ADD)
        .i32c(1)
        .op(op::I32_SUB)
        .g(k)
        .op(op::I32_SUB);
    fb.g(d).i32c(48).op(op::I32_ADD);
    fb.store8(0);
    fb.g(k).i32c(1).op(op::I32_ADD).s(k);
    fb.br(1); // continue $dg
    fb.end(); // $dgi
    fb.end(); // $dg
    fb.g(pos).g(w).op(op::I32_ADD).s(pos);
}

/// Store `w` zero bytes at NUM_BUF+pos.. and advance pos by `w`.
/// Bounded: at most `w` iterations (w <= 5 by construction).
pub(super) fn emit_zeros(fb: &mut FB, pos: u32, w: u32, k: u32) {
    fb.i32c(0).s(k);
    fb.loop_(); // $zr
    fb.g(k).g(w).op(op::I32_LT_U).if_(); // $zri
    fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
    fb.i32c(48);
    fb.store8(0);
    fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
    fb.g(k).i32c(1).op(op::I32_ADD).s(k);
    fb.br(1); // continue $zr
    fb.end(); // $zri
    fb.end(); // $zr
}

/// p10 = 10^w exactly (w <= 6 by construction). Bounded: `w` iterations.
pub(super) fn emit_pow10(fb: &mut FB, w: u32, p10: u32, k: u32) {
    fb.i64c(1).s(p10);
    fb.i32c(0).s(k);
    fb.loop_(); // $pw
    fb.g(k).g(w).op(op::I32_LT_U).if_(); // $pwi
    fb.g(p10).i64c(10).op(op::I64_MUL).s(p10);
    fb.g(k).i32c(1).op(op::I32_ADD).s(k);
    fb.br(1); // continue $pw
    fb.end(); // $pwi
    fb.end(); // $pw
}

/// `null → trap; index out of [0, len) → trap` — the loud-failure
/// contract shared by every list read/write.
pub(super) fn slice_new_body(fb: &mut FB, source_len_helper: u32, alloc_helper: u32) {
    let source_len = fb.scratch(Val::I64);
    let view = fb.scratch(Val::I32);

    fb.g(0).op(op::I32_EQZ);
    fb.g(1).i64c(0).op(op::I64_LT_S).op(op::I32_OR);
    fb.g(2).i64c(0).op(op::I64_LT_S).op(op::I32_OR);
    fb.if_().op(op::UNREACHABLE).end();

    fb.g(0).call(source_len_helper).s(source_len);
    // Check `start <= source_len && len <= source_len - start` without
    // forming `start + len`, so signed overflow cannot bypass the guard.
    fb.g(1).g(source_len).op(op::I64_GT_S);
    fb.g(2)
        .g(source_len)
        .g(1)
        .op(op::I64_SUB)
        .op(op::I64_GT_S)
        .op(op::I32_OR);
    fb.if_().op(op::UNREACHABLE).end();

    fb.i32c(SLICE_SIZE).call(alloc_helper).s(view);
    fb.g(view).g(0).store32(SLICE_BASE as u32);
    fb.g(view).g(1).store64(SLICE_START as u32);
    fb.g(view).g(2).store64(SLICE_LEN as u32);
    fb.g(view);
}

pub(super) fn slice_bounds_check(fb: &mut FB) {
    fb.g(0).op(op::I32_EQZ);
    fb.g(1).i64c(0).op(op::I64_LT_S).op(op::I32_OR);
    fb.g(1)
        .g(0)
        .load64(SLICE_LEN as u32)
        .op(op::I64_GE_S)
        .op(op::I32_OR);
    fb.if_().op(op::UNREACHABLE).end();
}

/// Leave `(source, absolute_index)` on the stack after checking the view index.
pub(super) fn slice_source_and_index(fb: &mut FB) {
    slice_bounds_check(fb);
    fb.g(0).load32(SLICE_BASE as u32);
    fb.g(0).load64(SLICE_START as u32).g(1).op(op::I64_ADD);
}

pub(super) fn list_bounds_check(fb: &mut FB) {
    fb.g(0).op(op::I32_EQZ).if_().op(op::UNREACHABLE).end();
    let len = fb.scratch(Val::I64);
    fb.g(0).load64(LIST_LEN as u32).s(len);
    fb.g(1)
        .g(len)
        .op(op::I64_GE_S)
        .if_()
        .op(op::UNREACHABLE)
        .end();
    fb.g(1)
        .i64c(0)
        .op(op::I64_LT_S)
        .if_()
        .op(op::UNREACHABLE)
        .end();
}

/// The shared push prologue (grow ×2 from 8 with a copied bump
/// "realloc") plus the per-class element store. Params: (list, value).
pub(super) fn list_push_body(fb: &mut FB, idx: impl Fn(H) -> u32, is_f64: bool, is_ptr: bool) {
    let len = fb.scratch(Val::I64);
    let cap = fb.scratch(Val::I64);
    let data = fb.scratch(Val::I32);
    let newdata = fb.scratch(Val::I32);
    let newcap = fb.scratch(Val::I64);
    fb.g(0).op(op::I32_EQZ).if_().op(op::UNREACHABLE).end();
    fb.g(0).load64(LIST_LEN as u32).s(len);
    fb.g(0).load64(LIST_CAP as u32).s(cap);
    fb.g(0)
        .load64(LIST_DATA as u32)
        .op(op::I32_WRAP_I64)
        .s(data);
    fb.g(len).g(cap).op(op::I64_EQ).if_();
    // newcap = cap == 0 ? 8 : cap * 2
    fb.i64c(LIST_NEW_CAP)
        .g(cap)
        .i64c(2)
        .op(op::I64_MUL)
        .g(cap)
        .op(op::I64_EQZ)
        .op(op::SELECT)
        .s(newcap);
    fb.g(newcap)
        .i64c(LIST_CAP_MAX)
        .op(op::I64_GT_S)
        .if_()
        .op(op::UNREACHABLE)
        .end();
    // newdata = Alloc(newcap * 8); copy oldcap * 8 bytes
    // The list's backing store is internal heap memory with the same
    // lifecycle as the list. Allocate it as a no-destructor ARC block
    // so final list drop and subsequent growth can return it to the
    // shared size-class free bins.
    fb.g(newcap)
        .i64c(8)
        .op(op::I64_MUL)
        .op(op::I32_WRAP_I64)
        .i32c(0)
        .call(idx(H::ArcAlloc))
        .s(newdata);
    fb.g(newdata)
        .g(data)
        .g(cap)
        .i64c(8)
        .op(op::I64_MUL)
        .op(op::I32_WRAP_I64)
        .memory_copy();
    // The new buffer owns a copied bitwise representation of the live
    // slots; after the copy the old backing block is no longer reachable.
    fb.g(data).call(idx(H::Release));
    fb.g(0)
        .g(newdata)
        .op(op::I64_EXTEND_I32_U)
        .store64(LIST_DATA as u32);
    fb.g(0).g(newcap).store64(LIST_CAP as u32);
    fb.g(newdata).s(data);
    fb.end();
    // data[len] = value; len += 1
    fb.g(data)
        .g(len)
        .op(op::I32_WRAP_I64)
        .i32c(8)
        .op(op::I32_MUL)
        .op(op::I32_ADD);
    fb.g(1);
    if is_f64 {
        fb.storef64(0);
    } else if is_ptr {
        fb.op(op::I64_EXTEND_I32_U);
        fb.store64(0);
    } else {
        fb.store64(0);
    }
    fb.g(0)
        .g(len)
        .i64c(1)
        .op(op::I64_ADD)
        .store64(LIST_LEN as u32);
}

// ---------------------------------------------------------------------------
// Generated destructors (5D2a): one per nominal, `lpp_drop_s{raw}` /
// `lpp_drop_e{raw}`, plus `lpp_drop_list`. Each takes the payload
// pointer and releases the managed children; `Release` dispatches
// nested destructors through the table.
// ---------------------------------------------------------------------------

pub(super) fn lower_destructor(
    env: &Env<'_>,
    aggregate: MirAggregateId,
) -> Result<(String, Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let layout = env
        .layouts
        .get(&aggregate)
        .ok_or_else(|| unsupported("unlaid-out nominal", None))?
        .clone();
    let agg = env.program.aggregate(aggregate).unwrap();
    let tag = match agg.kind {
        lpp_mir::MirAggregateKind::Struct => 's',
        lpp_mir::MirAggregateKind::Enum => 'e',
    };
    let name = format!("lpp_drop_{tag}{}", aggregate.raw());
    let release_idx = *env
        .helper_index
        .get(&H::Release)
        .expect("Release registered");
    let mut fb = FB::new(1);
    match layout.kind {
        lpp_mir::MirAggregateKind::Struct => {
            for slot in &layout.struct_fields {
                if is_managed(env.types, slot.ty) {
                    fb.g(0);
                    fb.load32(slot.offset);
                    fb.call(release_idx);
                }
            }
        }
        lpp_mir::MirAggregateKind::Enum => {
            let n = layout.variant_fields.len();
            let tag_local = fb.scratch(Val::I32);
            fb.g(0).load32(ENUM_TAG as u32).s(tag_local);
            for (ordinal, slots) in layout.variant_fields.iter().enumerate() {
                fb.g(tag_local);
                fb.i32c(ordinal as i64);
                fb.op(op::I32_EQ);
                fb.if_();
                for slot in slots {
                    if is_managed(env.types, slot.ty) {
                        fb.g(0);
                        fb.load32(slot.offset);
                        fb.call(release_idx);
                    }
                }
                if ordinal + 1 < n {
                    fb.else_();
                }
            }
            // Each `if` (including the last, else-less one) needs its `end`:
            // the chain nests, so all ends come after the final branch.
            for _ in 0..n {
                fb.end();
            }
        }
    }
    Ok((name, vec![Val::I32], vec![], fb.extras, fb.body))
}

pub(super) fn lower_list_destructor(
    env: &Env<'_>,
) -> Result<(String, Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let release_idx = *env
        .helper_index
        .get(&H::Release)
        .expect("Release registered");
    let mut fb = FB::new(1);
    let i = fb.scratch(Val::I64);
    // if is_arc: for each element, Release (the container's own
    // reference to each element)
    fb.g(0)
        .load64(LIST_IS_ARC as u32)
        .op(op::I32_WRAP_I64)
        .if_();
    fb.i64c(0).s(i);
    fb.block(); // $done
    fb.loop_(); // $d
    fb.g(0).load64(LIST_LEN as u32).g(i).op(op::I64_EQ).br_if(1); // $done
    fb.g(0)
        .load64(LIST_DATA as u32)
        .op(op::I32_WRAP_I64)
        .g(i)
        .i64c(8)
        .op(op::I64_MUL)
        .op(op::I32_WRAP_I64)
        .op(op::I32_ADD)
        .load64(0)
        .op(op::I32_WRAP_I64)
        .call(release_idx);
    fb.g(i).i64c(1).op(op::I64_ADD).s(i);
    fb.br(0); // $d
    fb.end(); // $d
    fb.end(); // $done
    fb.end(); // if
    // Release the backing array even when the list stores scalars (or
    // is empty). List data is an ARC block with a zero drop index.
    fb.g(0)
        .load64(LIST_DATA as u32)
        .op(op::I32_WRAP_I64)
        .call(release_idx);
    Ok((
        "lpp_drop_list".to_string(),
        vec![Val::I32],
        vec![],
        fb.extras,
        fb.body,
    ))
}
