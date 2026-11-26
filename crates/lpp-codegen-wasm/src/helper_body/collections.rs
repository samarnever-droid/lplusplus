use super::super::*;

pub(super) fn emit(
    fb: &mut FB,
    h: H,
    env: &Env<'_>,
    _fd_helper_idx: u32,
    _fd_write_import_idx: u32,
    _fd_read_import_idx: u32,
    proc_exit_import_idx: u32,
) -> Result<(), CodegenError> {
    let idx = |h: H| {
        *env.helper_index
            .get(&h)
            .expect("pre-scan closed the helper needs")
    };
    match h {
        H::ListNew => {
            let l = fb.scratch(Val::I32);
            let slot = env.list_drop_slot.expect("list destroyer registered");
            fb.i32c(32).i32c(slot as i64).call(idx(H::ArcAlloc)).s(l);
            fb.g(l).i64c(0).store64(LIST_DATA as u32);
            fb.g(l).i64c(0).store64(LIST_LEN as u32);
            fb.g(l).i64c(0).store64(LIST_CAP as u32);
            fb.g(l)
                .g(0)
                .op(op::I64_EXTEND_I32_U)
                .store64(LIST_IS_ARC as u32);
            fb.g(l);
        }
        H::ListPushI64 => {
            list_push_body(fb, idx, false, false);
        }
        H::ListPushF64 => {
            list_push_body(fb, idx, true, false);
        }
        H::ListPushPtr => {
            list_push_body(fb, idx, false, true);
        }
        H::ListLen => {
            fb.g(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
            fb.g(0).load64(LIST_LEN as u32);
        }
        H::ListGetI64 => {
            list_bounds_check(fb);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(0);
        }
        H::ListGetF64 => {
            list_bounds_check(fb);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .loadf64(0);
        }
        H::ListGetPtr => {
            list_bounds_check(fb);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(0)
                .op(op::I32_WRAP_I64);
        }
        H::ListSetI64 => {
            list_bounds_check(fb);
            let addr = fb.scratch(Val::I32);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(addr);
            fb.g(addr).g(2).store64(0);
        }
        H::ListSetF64 => {
            list_bounds_check(fb);
            let addr = fb.scratch(Val::I32);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(addr);
            fb.g(addr).g(2).storef64(0);
        }
        H::ListSetPtr => {
            // Releases the replaced element and retains the new one
            // (runtime-internal, mirroring `lpp_list_set_arc`).
            list_bounds_check(fb);
            let addr = fb.scratch(Val::I32);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(addr);
            fb.g(addr)
                .load64(0)
                .op(op::I32_WRAP_I64)
                .call(idx(H::Release));
            // Retain passes the value through; drop the pass-through
            // (the store below re-pushes it).
            fb.g(2).call(idx(H::Retain)).op(op::DROP);
            // Stores pop (value, addr): address first, value on top.
            fb.g(addr);
            fb.g(2).op(op::I64_EXTEND_I32_U).store64(0);
        }
        // ── borrowed list/string slices ──────────────────────────────
        H::SliceNewList => {
            slice_new_body(fb, idx(H::ListLen), idx(H::Alloc));
        }
        H::SliceNewStr => {
            slice_new_body(fb, idx(H::StrLen), idx(H::Alloc));
        }
        H::SliceLen => {
            fb.g(0).op(op::I32_EQZ).if_().op(op::UNREACHABLE).end();
            fb.g(0).load64(SLICE_LEN as u32);
        }
        H::SliceGetI64 => {
            slice_source_and_index(fb);
            fb.call(idx(H::ListGetI64));
        }
        H::SliceGetF64 => {
            slice_source_and_index(fb);
            fb.call(idx(H::ListGetF64));
        }
        H::SliceGetPtr => {
            slice_source_and_index(fb);
            fb.call(idx(H::ListGetPtr));
        }
        H::StrSliceGet => {
            slice_bounds_check(fb);
            let source = fb.scratch(Val::I32);
            let absolute = fb.scratch(Val::I64);
            let base = fb.scratch(Val::I32);
            let payload = fb.scratch(Val::I32);
            fb.g(0).load32(SLICE_BASE as u32).s(source);
            fb.g(0)
                .load64(SLICE_START as u32)
                .g(1)
                .op(op::I64_ADD)
                .s(absolute);
            fb.i32c(5).i32c(0).call(idx(H::ArcAlloc)).s(payload);
            fb.g(payload)
                .i32c(STR_HEADER as i64)
                .op(op::I32_SUB)
                .s(base);
            fb.g(payload).i32c(1).store32(0);
            fb.g(payload).i32c(4).op(op::I32_ADD);
            fb.g(source)
                .i32c(4)
                .op(op::I32_ADD)
                .g(absolute)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.g(payload);
        }
        H::StrSliceToStr => {
            let source = fb.scratch(Val::I32);
            let start = fb.scratch(Val::I64);
            let length = fb.scratch(Val::I64);
            let base = fb.scratch(Val::I32);
            let payload = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I64);
            fb.g(0).op(op::I32_EQZ).if_().op(op::UNREACHABLE).end();
            fb.g(0).load32(SLICE_BASE as u32).s(source);
            fb.g(0).load64(SLICE_START as u32).s(start);
            fb.g(0).load64(SLICE_LEN as u32).s(length);
            fb.i32c(4)
                .g(length)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(payload);
            fb.g(payload)
                .i32c(STR_HEADER as i64)
                .op(op::I32_SUB)
                .s(base);
            fb.g(payload).g(length).op(op::I32_WRAP_I64).store32(0);
            fb.i64c(0).s(i);
            fb.loop_();
            fb.g(i).g(length).op(op::I64_LT_U).if_();
            fb.g(payload)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD);
            fb.g(source)
                .i32c(4)
                .op(op::I32_ADD)
                .g(start)
                .g(i)
                .op(op::I64_ADD)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end();
            fb.g(i).i64c(1).op(op::I64_ADD).s(i);
            fb.g(i).g(length).op(op::I64_LT_U).br_if(0);
            fb.end();
            fb.g(payload);
        }
        // ── deterministic Int/String-key maps with Int values ────────
        H::MapNew => {
            let map = fb.scratch(Val::I32);
            fb.i32c(MAP_SIZE).call(idx(H::Alloc)).s(map);
            fb.g(map).i64c(0).store64(MAP_DATA as u32);
            fb.g(map).i64c(0).store64(MAP_LEN as u32);
            fb.g(map).i64c(0).store64(MAP_CAP as u32);
            fb.g(map)
                .g(0)
                .op(op::I64_EXTEND_I32_U)
                .store64(MAP_STR_KEYS as u32);
            fb.g(map);
        }
        H::MapEnsure => {
            let cap = fb.scratch(Val::I64);
            let new_cap = fb.scratch(Val::I64);
            let old_data = fb.scratch(Val::I32);
            let new_data = fb.scratch(Val::I32);
            let len = fb.scratch(Val::I64);
            let i = fb.scratch(Val::I64);
            fb.g(0).load64(MAP_LEN as u32).s(len);
            fb.g(0).load64(MAP_CAP as u32).t(cap);
            // `cap` remains on the stack from `tee`; trap only if `cap < len`.
            fb.g(len).op(op::I64_LT_U).if_().op(op::UNREACHABLE).end();
            fb.g(len).g(cap).op(op::I64_LT_U).if_().op(op::RETURN).end();
            fb.g(cap).op(op::I64_EQZ).if_();
            fb.i64c(MAP_INITIAL_CAP).s(new_cap);
            fb.else_();
            fb.g(cap).i64c(2).op(op::I64_MUL).s(new_cap);
            fb.end();
            fb.g(0)
                .load64(MAP_DATA as u32)
                .op(op::I32_WRAP_I64)
                .s(old_data);
            fb.g(new_cap)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .call(idx(H::Alloc))
                .s(new_data);
            fb.i64c(0).s(i);
            fb.loop_();
            fb.g(i).g(len).op(op::I64_LT_U).if_();
            fb.g(new_data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD);
            fb.g(old_data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(0);
            fb.store64(0);
            fb.g(new_data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD);
            fb.g(old_data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(8);
            fb.store64(8);
            fb.end();
            fb.g(i).i64c(1).op(op::I64_ADD).s(i);
            fb.g(i).g(len).op(op::I64_LT_U).br_if(0);
            fb.end();
            fb.g(0)
                .g(new_data)
                .op(op::I64_EXTEND_I32_U)
                .store64(MAP_DATA as u32);
            fb.g(0).g(new_cap).store64(MAP_CAP as u32);
        }
        H::MapFindI64 | H::MapFindStr => {
            let len = fb.scratch(Val::I64);
            let data = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I64);
            let entry = fb.scratch(Val::I32);
            fb.g(0).op(op::I32_EQZ).if_().i64c(-1).op(op::RETURN).end();
            fb.g(0).load64(MAP_LEN as u32).s(len);
            fb.g(0).load64(MAP_DATA as u32).op(op::I32_WRAP_I64).s(data);
            fb.i64c(0).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).g(len).op(op::I64_GE_U).br_if(1);
            fb.g(data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(entry);
            if h == H::MapFindStr {
                fb.g(entry)
                    .load64(0)
                    .op(op::I32_WRAP_I64)
                    .g(1)
                    .call(idx(H::StrEq));
            } else {
                fb.g(entry).load64(0).g(1).op(op::I64_EQ);
            }
            fb.if_().g(i).op(op::RETURN).end();
            fb.g(i).i64c(1).op(op::I64_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.i64c(-1);
        }
        H::MapPutI64 | H::MapPutStr => {
            let found = fb.scratch(Val::I64);
            let len = fb.scratch(Val::I64);
            let data = fb.scratch(Val::I32);
            let entry = fb.scratch(Val::I32);
            fb.g(0)
                .g(1)
                .call(idx(if h == H::MapPutStr {
                    H::MapFindStr
                } else {
                    H::MapFindI64
                }))
                .s(found);
            fb.g(found).i64c(0).op(op::I64_GE_S).if_();
            fb.g(0)
                .load64(MAP_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(found)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .g(2)
                .store64(8);
            fb.else_();
            fb.g(0).call(idx(H::MapEnsure));
            fb.g(0).load64(MAP_LEN as u32).s(len);
            fb.g(0).load64(MAP_DATA as u32).op(op::I32_WRAP_I64).s(data);
            fb.g(data)
                .g(len)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(entry);
            fb.g(entry);
            fb.g(1);
            if h == H::MapPutStr {
                fb.op(op::I64_EXTEND_I32_U);
            }
            fb.store64(0);
            fb.g(entry).g(2).store64(8);
            fb.g(0)
                .g(len)
                .i64c(1)
                .op(op::I64_ADD)
                .store64(MAP_LEN as u32);
            fb.end();
        }
        H::MapGetI64 | H::MapGetStr => {
            let found = fb.scratch(Val::I64);
            fb.g(0)
                .g(1)
                .call(idx(if h == H::MapGetStr {
                    H::MapFindStr
                } else {
                    H::MapFindI64
                }))
                .s(found);
            fb.g(found).i64c(0).op(op::I64_LT_S).if_();
            fb.i64c(0).op(op::RETURN);
            fb.end();
            fb.g(0)
                .load64(MAP_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(found)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(8);
        }
        H::MapHasI64 | H::MapHasStr => {
            fb.g(0).g(1).call(idx(if h == H::MapHasStr {
                H::MapFindStr
            } else {
                H::MapFindI64
            }));
            fb.i64c(0).op(op::I64_GE_S).op(op::I64_EXTEND_I32_U);
        }
        H::MapRemoveI64 | H::MapRemoveStr => {
            let found = fb.scratch(Val::I64);
            let last = fb.scratch(Val::I64);
            let data = fb.scratch(Val::I32);
            let destination = fb.scratch(Val::I32);
            let source = fb.scratch(Val::I32);
            fb.g(0)
                .g(1)
                .call(idx(if h == H::MapRemoveStr {
                    H::MapFindStr
                } else {
                    H::MapFindI64
                }))
                .s(found);
            fb.g(found).i64c(0).op(op::I64_GE_S).if_();
            fb.g(0)
                .load64(MAP_LEN as u32)
                .i64c(1)
                .op(op::I64_SUB)
                .s(last);
            fb.g(0).load64(MAP_DATA as u32).op(op::I32_WRAP_I64).s(data);
            fb.g(found).g(last).op(op::I64_NE).if_();
            fb.g(data)
                .g(found)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(destination);
            fb.g(data)
                .g(last)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(source);
            fb.g(destination).g(source).load64(0).store64(0);
            fb.g(destination).g(source).load64(8).store64(8);
            fb.end();
            fb.g(0).g(last).store64(MAP_LEN as u32);
            fb.end();
        }
        H::MapLen => {
            fb.g(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
            fb.g(0).load64(MAP_LEN as u32);
        }
        // ── 5D2b slice 3: the task runtime ──
        H::TaskNew => {
            // NULL code or environment exits 101 (shim parity), then a
            // 32-byte ARC node: {code i64@0, env i64@8, result i64@16,
            // state i32@24, result_managed i32@28}.
            fb.g(0).i32c(0).op(op::I32_EQ);
            fb.g(1).i32c(0).op(op::I32_EQ);
            fb.op(op::I32_OR);
            fb.if_();
            fb.i32c(101);
            fb.call(proc_exit_import_idx);
            fb.end();
            fb.i32c(32);
            fb.i32c(env.task_drop_slot.expect("task slot planned") as i64);
            fb.call(idx(H::ArcAlloc));
            let node = fb.scratch(Val::I32);
            fb.t(node);
            fb.g(node).g(0).op(op::I64_EXTEND_I32_S).store64(0);
            fb.g(node).g(1).op(op::I64_EXTEND_I32_S).store64(8);
            fb.g(node).i64c(0).store64(16);
            fb.g(node).i32c(0).store32(24);
            fb.g(node).g(2).store32(28);
            // The ArcAlloc result (the tee'd node pointer) is the
            // result: widen it to the i64 result word.
            fb.op(op::I64_EXTEND_I32_U);
        }
        H::TaskPoll => {
            // state 0→1, run the task code, store the result, state 2.
            // Polling a non-fresh (or NULL) task exits 101.
            fb.g(0).i32c(0).op(op::I32_EQ);
            fb.g(0).load32(24).i32c(0).op(op::I32_NE);
            fb.op(op::I32_OR);
            fb.if_();
            fb.i32c(101);
            fb.call(proc_exit_import_idx);
            fb.end();
            fb.g(0).i32c(1).store32(24);
            // Task-code call: the thunk takes (env) -> result word;
            // the callee's dispatch index (the stored code word) is
            // the call_indirect element index.
            // Task-code call: thunk (env) -> result word, element
            // index = the stored code word (on top of the stack).
            // Task-code call: thunk (env) -> result word; the
            // stored code word is the call_indirect element index
            // (pushed last = on top of the stack).
            fb.g(0).load64(8).op(op::I32_WRAP_I64);
            fb.g(0).load64(0).op(op::I32_WRAP_I64);
            fb.call_indirect(env.task_code_call_type, env.dispatch_table);
            let result = fb.scratch(Val::I64);
            // Tee the word, DROP the stack copy (the local is the
            // canonical slot), store it into the node, and return a
            // re-read of the local — the exit stack is exactly the
            // one return word.
            fb.t(result);
            fb.op(op::DROP);
            fb.g(0).g(result).store64(16);
            fb.g(0).i32c(2).store32(24);
            fb.g(result);
        }
        H::TaskAwait => {
            // A NULL task (or one with a NULL env) exits 101; a fresh
            // task is polled; a managed result is retained for the
            // awaiter (the task keeps its share).
            fb.g(0).i32c(0).op(op::I32_EQ);
            fb.g(0).load64(8).op(op::I64_EQZ);
            fb.op(op::I32_OR);
            fb.if_();
            fb.i32c(101);
            fb.call(proc_exit_import_idx);
            fb.end();
            fb.g(0).load32(24).i32c(0).op(op::I32_EQ);
            fb.if_();
            fb.g(0).call(idx(H::TaskPoll)).op(op::DROP);
            fb.end();
            fb.g(0).load32(28).i32c(0).op(op::I32_NE);
            fb.if_();
            fb.g(0).load64(16).op(op::I32_WRAP_I64).call(idx(H::Retain));
            // Retain passes the value through; the result word is
            // re-read below, so drop the retained copy.
            fb.op(op::DROP);
            fb.end();
            fb.g(0).load64(16);
        }
        H::TaskDestroy => {
            // Release the task node (its destructor frees the env
            // tuple and, once resolved, the managed result).
            fb.g(0).call(idx(H::Release));
        }
        // ── 5D2b slice 4, batch 3: string builtins ────────────────────
        // A new string is an immortal ARC node: `[rc][drop 0][magic]`
        // + `[len i32][utf8]`, returned as `base + 24`. The output
        // is composed into the NUM_BUF scratch first, then copied.
        _ => unreachable!("helper dispatched to the wrong body group"),
    }
    Ok(())
}
