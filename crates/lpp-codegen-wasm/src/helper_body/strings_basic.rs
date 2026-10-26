use super::super::*;

pub(super) fn emit(
    fb: &mut FB,
    h: H,
    env: &Env<'_>,
    fd_helper_idx: u32,
    fd_write_import_idx: u32,
    fd_read_import_idx: u32,
    proc_exit_import_idx: u32,
) -> Result<(), CodegenError> {
    let idx = |h: H| {
        *env.helper_index
            .get(&h)
            .expect("pre-scan closed the helper needs")
    };
    match h {
        H::StrConcat => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(la).g(lb).op(op::I32_ADD).s(rlen);
            fb.i32c(4)
                .g(rlen)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(string_payload);
            fb.g(string_payload)
                .i32c(STR_HEADER as i64)
                .op(op::I32_SUB)
                .s(base);
            fb.g(base).g(rlen).store32(24);
            // left bytes
            fb.i32c(0).s(i);
            fb.loop_(); // $c
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $in
            fb.g(base).i32c(28).op(op::I32_ADD).g(i).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end(); // $in
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.g(i).g(la).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            // right bytes
            fb.i32c(0).s(i);
            fb.loop_(); // $c
            fb.g(i).g(lb).op(op::I32_LT_U).if_(); // $in
            fb.g(base)
                .i32c(28)
                .op(op::I32_ADD)
                .g(la)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end(); // $in
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.g(i).g(lb).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrSubstr => {
            let slen = fb.scratch(Val::I32);
            let copy = fb.scratch(Val::I32);
            let start = fb.scratch(Val::I32);
            let out = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(slen);
            fb.i32c(0).s(copy);
            fb.i32c(0).s(start);
            fb.g(1).i64c(0).op(op::I64_GE_S);
            fb.g(2).i64c(0).op(op::I64_GT_S).op(op::I32_AND);
            fb.g(1)
                .g(slen)
                .op(op::I64_EXTEND_I32_U)
                .op(op::I64_LT_U)
                .op(op::I32_AND)
                .if_();
            fb.g(1).op(op::I32_WRAP_I64).s(start);
            fb.g(slen).g(start).op(op::I32_SUB).s(copy);
            fb.g(2)
                .g(copy)
                .op(op::I64_EXTEND_I32_U)
                .op(op::I64_LT_U)
                .if_();
            fb.g(2).op(op::I32_WRAP_I64).s(copy);
            fb.end();
            fb.end();
            fb.g(copy)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(copy).store32(0);
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(0).i32c(4).op(op::I32_ADD).g(start).op(op::I32_ADD);
            fb.g(copy);
            fb.memory_copy();
            fb.g(out);
        }
        H::StrRepeat => {
            let slen = fb.scratch(Val::I32);
            let total = fb.scratch(Val::I64);
            let out = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I64);
            fb.g(0).load32(0).s(slen);
            fb.i64c(0).s(total);
            fb.g(1).i64c(0).op(op::I64_GT_S).if_();
            fb.g(slen)
                .op(op::I64_EXTEND_I32_U)
                .g(1)
                .op(op::I64_MUL)
                .s(total);
            fb.g(total).i64c(0x7fff_fff0).op(op::I64_GT_U).if_();
            fb.op(op::UNREACHABLE);
            fb.end();
            fb.end();
            fb.g(total)
                .op(op::I32_WRAP_I64)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(total).op(op::I32_WRAP_I64).store32(0);
            fb.g(total).op(op::I64_EQZ).if_();
            fb.g(out).op(op::RETURN);
            fb.end();
            fb.i64c(0).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).g(1).op(op::I64_GE_S).br_if(1);
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(i)
                .g(slen)
                .op(op::I64_EXTEND_I32_U)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD);
            fb.g(0).i32c(4).op(op::I32_ADD);
            fb.g(slen);
            fb.memory_copy();
            fb.g(i).i64c(1).op(op::I64_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(out);
        }
        H::StrSplit => {
            let list = fb.scratch(Val::I32);
            let len = fb.scratch(Val::I32);
            let start = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let piece_len = fb.scratch(Val::I32);
            let piece = fb.scratch(Val::I32);
            let boundary = fb.scratch(Val::I32);
            fb.i32c(1).call(idx(H::ListNew)).s(list);
            fb.g(0).load32(0).s(len);
            fb.i32c(0).s(start);
            fb.i32c(0).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).g(len).op(op::I32_EQ).s(boundary);
            fb.g(i).g(len).op(op::I32_LT_U).if_();
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .g(1)
                .op(op::I32_WRAP_I64)
                .op(op::I32_EQ)
                .if_();
            fb.i32c(1).s(boundary);
            fb.end();
            fb.end();
            fb.g(boundary).if_();
            fb.g(i).g(start).op(op::I32_SUB).s(piece_len);
            fb.g(piece_len)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(piece);
            fb.g(piece).g(piece_len).store32(0);
            fb.g(piece).i32c(4).op(op::I32_ADD);
            fb.g(0).i32c(4).op(op::I32_ADD).g(start).op(op::I32_ADD);
            fb.g(piece_len);
            fb.memory_copy();
            fb.g(list).g(piece).call(idx(H::ListPushPtr));
            fb.g(i).i32c(1).op(op::I32_ADD).s(start);
            fb.end();
            fb.g(i).g(len).op(op::I32_EQ).br_if(1);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(list);
        }
        H::CharAt => {
            let len = fb.scratch(Val::I32);
            let out_len = fb.scratch(Val::I32);
            let out = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(len);
            fb.i32c(0).s(out_len);
            fb.g(1).i64c(0).op(op::I64_GE_S);
            fb.g(1)
                .g(len)
                .op(op::I64_EXTEND_I32_U)
                .op(op::I64_LT_U)
                .op(op::I32_AND)
                .if_();
            fb.i32c(1).s(out_len);
            fb.end();
            fb.g(out_len)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(out_len).store32(0);
            fb.g(out_len).if_();
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(1)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end();
            fb.g(out);
        }
        H::Ord => {
            let len = fb.scratch(Val::I32);
            let lead = fb.scratch(Val::I32);
            let extra = fb.scratch(Val::I32);
            let code = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let byte = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(len);
            fb.g(len).op(op::I32_EQZ).if_();
            fb.i64c(0).op(op::RETURN);
            fb.end();
            fb.g(0).i32c(4).op(op::I32_ADD).load8(0).s(lead);
            fb.g(lead).i32c(0x80).op(op::I32_LT_U).if_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.g(lead)
                .i32c(0xe0)
                .op(op::I32_AND)
                .i32c(0xc0)
                .op(op::I32_EQ)
                .if_();
            fb.i32c(1).s(extra);
            fb.g(lead).i32c(0x1f).op(op::I32_AND).s(code);
            fb.else_();
            fb.g(lead)
                .i32c(0xf0)
                .op(op::I32_AND)
                .i32c(0xe0)
                .op(op::I32_EQ)
                .if_();
            fb.i32c(2).s(extra);
            fb.g(lead).i32c(0x0f).op(op::I32_AND).s(code);
            fb.else_();
            fb.g(lead)
                .i32c(0xf8)
                .op(op::I32_AND)
                .i32c(0xf0)
                .op(op::I32_EQ)
                .if_();
            fb.i32c(3).s(extra);
            fb.g(lead).i32c(0x07).op(op::I32_AND).s(code);
            fb.else_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.end();
            fb.end();
            fb.g(len).g(extra).op(op::I32_LE_U).if_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.i32c(1).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).g(extra).op(op::I32_GT_U).br_if(1);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(byte);
            fb.g(byte)
                .i32c(0xc0)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_NE)
                .if_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.g(code)
                .i32c(6)
                .op(op::I32_SHL)
                .g(byte)
                .i32c(0x3f)
                .op(op::I32_AND)
                .op(op::I32_OR)
                .s(code);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            // Reject overlong encodings, surrogates, and out-of-range scalars.
            fb.g(extra).i32c(1).op(op::I32_EQ);
            fb.g(code).i32c(0x80).op(op::I32_LT_U).op(op::I32_AND);
            fb.g(extra).i32c(2).op(op::I32_EQ);
            fb.g(code).i32c(0x800).op(op::I32_LT_U).op(op::I32_AND);
            fb.op(op::I32_OR);
            fb.g(extra).i32c(3).op(op::I32_EQ);
            fb.g(code).i32c(0x10000).op(op::I32_LT_U).op(op::I32_AND);
            fb.op(op::I32_OR);
            fb.g(code).i32c(0x10ffff).op(op::I32_GT_U).op(op::I32_OR);
            fb.g(code).i32c(0xd800).op(op::I32_GE_U);
            fb.g(code).i32c(0xdfff).op(op::I32_LE_U).op(op::I32_AND);
            fb.op(op::I32_OR).if_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.g(code).op(op::I64_EXTEND_I32_U);
        }
        H::Chr => {
            let valid = fb.scratch(Val::I32);
            let code = fb.scratch(Val::I32);
            let len = fb.scratch(Val::I32);
            let out = fb.scratch(Val::I32);
            fb.g(0).i64c(0).op(op::I64_GE_S);
            fb.g(0)
                .i64c(0x10ffff)
                .op(op::I64_LE_U)
                .op(op::I32_AND)
                .s(valid);
            fb.g(0).op(op::I32_WRAP_I64).s(code);
            fb.g(code).i32c(0xd800).op(op::I32_GE_U);
            fb.g(code)
                .i32c(0xdfff)
                .op(op::I32_LE_U)
                .op(op::I32_AND)
                .if_();
            fb.i32c(0).s(valid);
            fb.end();
            fb.i32c(0).s(len);
            fb.g(valid).if_();
            fb.g(code).i32c(0x80).op(op::I32_LT_U).if_();
            fb.i32c(1).s(len);
            fb.else_();
            fb.g(code).i32c(0x800).op(op::I32_LT_U).if_();
            fb.i32c(2).s(len);
            fb.else_();
            fb.g(code).i32c(0x10000).op(op::I32_LT_U).if_();
            fb.i32c(3).s(len);
            fb.else_();
            fb.i32c(4).s(len);
            fb.end();
            fb.end();
            fb.end();
            fb.end();
            fb.g(len)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(len).store32(0);
            fb.g(len).i32c(1).op(op::I32_EQ).if_();
            fb.g(out).i32c(4).op(op::I32_ADD).g(code).store8(0);
            fb.end();
            fb.g(len).i32c(2).op(op::I32_EQ).if_();
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(code)
                .i32c(6)
                .op(op::I32_SHR_U)
                .i32c(0xc0)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(5).op(op::I32_ADD);
            fb.g(code)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.end();
            fb.g(len).i32c(3).op(op::I32_EQ).if_();
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(code)
                .i32c(12)
                .op(op::I32_SHR_U)
                .i32c(0xe0)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(5).op(op::I32_ADD);
            fb.g(code)
                .i32c(6)
                .op(op::I32_SHR_U)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(6).op(op::I32_ADD);
            fb.g(code)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.end();
            fb.g(len).i32c(4).op(op::I32_EQ).if_();
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(code)
                .i32c(18)
                .op(op::I32_SHR_U)
                .i32c(0xf0)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(5).op(op::I32_ADD);
            fb.g(code)
                .i32c(12)
                .op(op::I32_SHR_U)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(6).op(op::I32_ADD);
            fb.g(code)
                .i32c(6)
                .op(op::I32_SHR_U)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(7).op(op::I32_ADD);
            fb.g(code)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.end();
            fb.g(out);
        }
        H::StrContains => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(lb).i32c(0).op(op::I32_EQ).if_();
            fb.i32c(1);
            fb.op(op::RETURN);
            fb.end();
            fb.g(lb).g(la).op(op::I32_GT_U).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.i32c(0).s(i);
            fb.block(); // $outer
            fb.loop_(); // $outl
            fb.g(i)
                .g(la)
                .g(lb)
                .op(op::I32_SUB)
                .op(op::I32_GT_S)
                .br_if(1); // exhausted: exit $outer
            fb.i32c(0).s(j);
            fb.block(); // $inner
            fb.loop_(); // $inl
            fb.g(j).g(lb).op(op::I32_GE_U).br_if(1); // full match: exit $inner
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .op(op::I32_NE)
                .br_if(1); // mismatch: exit $inner
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.br(0); // $inl
            fb.end(); // $inl
            fb.end(); // $inner
            fb.g(j).g(lb).op(op::I32_EQ).if_(); // matched
            fb.i32c(1);
            fb.op(op::RETURN);
            fb.end();
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0); // $outl
            fb.end(); // $outl
            fb.end(); // $outer
            fb.i32c(0);
        }
        H::StrStartsWith => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let ca = fb.scratch(Val::I32);
            let cb = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(lb).g(la).op(op::I32_GT_U).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.i32c(0).s(j);
            fb.loop_(); // $c
            fb.g(j).g(lb).op(op::I32_LT_U).if_(); // $in
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(ca);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(cb);
            fb.g(ca).g(cb).op(op::I32_NE).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.end(); // $in
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.g(j).g(lb).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.i32c(1);
        }
        H::StrEndsWith => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let off = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let ca = fb.scratch(Val::I32);
            let cb = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(lb).g(la).op(op::I32_GT_U).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.g(la).g(lb).op(op::I32_SUB).s(off);
            fb.i32c(0).s(j);
            fb.loop_(); // $c
            fb.g(j).g(lb).op(op::I32_LT_U).if_(); // $in
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(off)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(ca);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(cb);
            fb.g(ca).g(cb).op(op::I32_NE).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.end(); // $in
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.g(j).g(lb).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.i32c(1);
        }
        H::StrFind => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let ca = fb.scratch(Val::I32);
            let cb = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(lb).i32c(0).op(op::I32_EQ).if_();
            fb.i64c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.g(lb).g(la).op(op::I32_GT_U).if_();
            fb.i64c(-1);
            fb.op(op::RETURN);
            fb.end();
            fb.i32c(0).s(i);
            fb.block(); // $outer
            fb.loop_(); // $outl
            fb.g(i)
                .g(la)
                .g(lb)
                .op(op::I32_SUB)
                .op(op::I32_GT_S)
                .br_if(1); // exhausted: exit $outer
            fb.i32c(0).s(j);
            fb.block(); // $inner
            fb.loop_(); // $inl
            fb.g(j).g(lb).op(op::I32_GE_U).br_if(1); // full match: exit $inner
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(ca);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(cb);
            fb.g(ca).g(cb).op(op::I32_NE).br_if(1); // mismatch: exit $inner
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.br(0); // $inl
            fb.end(); // $inl
            fb.end(); // $inner
            fb.g(j).g(lb).op(op::I32_EQ).if_(); // matched
            fb.g(i).op(op::I64_EXTEND_I32_U);
            fb.op(op::RETURN);
            fb.end();
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0); // $outl
            fb.end(); // $outl
            fb.end(); // $outer
            fb.i64c(-1);
        }
        _ => unreachable!("helper dispatched to the wrong body group"),
    }
    Ok(())
}
