use super::super::*;

pub(super) fn emit(
    fb: &mut FB,
    h: H,
    env: &Env<'_>,
    _fd_helper_idx: u32,
    _fd_write_import_idx: u32,
    _fd_read_import_idx: u32,
    _proc_exit_import_idx: u32,
) -> Result<(), CodegenError> {
    let idx = |h: H| {
        *env.helper_index
            .get(&h)
            .expect("pre-scan closed the helper needs")
    };
    match h {
        H::StrReplace => {
            let lt = fb.scratch(Val::I32);
            let lo = fb.scratch(Val::I32);
            let ln = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let m = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let k = fb.scratch(Val::I32);
            let o = fb.scratch(Val::I32);
            let ca = fb.scratch(Val::I32);
            let cb = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(lt);
            fb.g(1).load32(0).s(lo);
            fb.g(2).load32(0).s(ln);
            // The oracle returns the text unchanged when old is empty. The
            // result is a distinct owner, so retain the aliased allocation.
            fb.g(lo).i32c(0).op(op::I32_EQ).if_();
            fb.g(0).call(idx(H::Retain));
            fb.op(op::RETURN);
            fb.end();
            // Pass 1: the result length.
            fb.i32c(0).s(i);
            fb.i32c(0).s(rlen);
            fb.loop_(); // $p1
            fb.g(i).g(lt).op(op::I32_LT_U).if_(); // $p1in
            fb.i32c(0).s(m);
            fb.g(i).g(lo).op(op::I32_ADD).g(lt).op(op::I32_LE_U).if_(); // $can
            fb.i32c(0).s(j);
            fb.i32c(1).s(m);
            fb.block(); // $match
            fb.loop_(); // $ml
            fb.g(j).g(lo).op(op::I32_GE_U).br_if(1); // full match
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
            fb.g(ca).g(cb).op(op::I32_NE).if_(); // $mis
            fb.i32c(0).s(m);
            fb.end(); // $mis
            fb.g(m).i32c(0).op(op::I32_EQ).br_if(1); // mismatch
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.br(0); // $ml
            fb.end(); // $ml
            fb.end(); // $match
            fb.end(); // $can
            fb.g(m).if_();
            fb.g(rlen).g(ln).op(op::I32_ADD).s(rlen);
            fb.g(i).g(lo).op(op::I32_ADD).s(i);
            fb.else_();
            fb.g(rlen).i32c(1).op(op::I32_ADD).s(rlen);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.end();
            fb.br(1); // $p1
            fb.end(); // $p1in
            fb.end(); // $p1
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
            // Pass 2: the copy.
            fb.i32c(0).s(i);
            fb.i32c(0).s(o);
            fb.loop_(); // $p2
            fb.g(i).g(lt).op(op::I32_LT_U).if_(); // $p2in
            fb.i32c(0).s(m);
            fb.g(i).g(lo).op(op::I32_ADD).g(lt).op(op::I32_LE_U).if_(); // $can
            fb.i32c(0).s(j);
            fb.i32c(1).s(m);
            fb.block(); // $match
            fb.loop_(); // $ml
            fb.g(j).g(lo).op(op::I32_GE_U).br_if(1); // full match
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
            fb.g(ca).g(cb).op(op::I32_NE).if_(); // $mis
            fb.i32c(0).s(m);
            fb.end(); // $mis
            fb.g(m).i32c(0).op(op::I32_EQ).br_if(1); // mismatch
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.br(0); // $ml
            fb.end(); // $ml
            fb.end(); // $match
            fb.end(); // $can
            fb.g(m).if_();
            fb.i32c(0).s(k);
            fb.loop_(); // $cpn
            fb.g(k).g(ln).op(op::I32_LT_U).if_(); // $cpnin
            fb.g(base)
                .i32c(28)
                .op(op::I32_ADD)
                .g(o)
                .op(op::I32_ADD)
                .g(k)
                .op(op::I32_ADD);
            fb.g(2)
                .i32c(4)
                .op(op::I32_ADD)
                .g(k)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end(); // $cpnin
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.g(k).g(ln).op(op::I32_LT_U).br_if(0); // $cpn
            fb.end(); // $cpn
            fb.g(o).g(ln).op(op::I32_ADD).s(o);
            fb.g(i).g(lo).op(op::I32_ADD).s(i);
            fb.else_();
            fb.g(base).i32c(28).op(op::I32_ADD).g(o).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.g(o).i32c(1).op(op::I32_ADD).s(o);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.end();
            fb.br(1); // $p2
            fb.end(); // $p2in
            fb.end(); // $p2
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrTrim => {
            let la = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            let k = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            // Skip leading spaces/tabs/newlines/carriage returns.
            fb.i32c(0).s(i);
            fb.loop_(); // $f
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $fin
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c)
                .i32c(32)
                .op(op::I32_EQ)
                .g(c)
                .i32c(9)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(10)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(13)
                .op(op::I32_EQ)
                .op(op::I32_OR);
            fb.if_(); // $ws
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(2); // continue $f
            fb.end(); // $ws
            fb.br(0); // break $f
            fb.end(); // $fin
            fb.end(); // $f
            // Skip the same set from the end.
            fb.g(la).s(j);
            fb.loop_(); // $b
            fb.g(j).i32c(0).op(op::I32_GT_U).if_(); // $bin
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .i32c(1)
                .op(op::I32_SUB)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c)
                .i32c(32)
                .op(op::I32_EQ)
                .g(c)
                .i32c(9)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(10)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(13)
                .op(op::I32_EQ)
                .op(op::I32_OR);
            fb.if_(); // $ws
            fb.g(j).i32c(1).op(op::I32_SUB).s(j);
            fb.br(2); // continue $b
            fb.end(); // $ws
            fb.br(0); // break $b
            fb.end(); // $bin
            fb.end(); // $b
            fb.g(j).g(i).op(op::I32_SUB).s(rlen);
            // All-whitespace text: i == la > 0 == j would underflow.
            fb.g(rlen).i32c(0).op(op::I32_LT_S).if_();
            fb.i32c(0).s(rlen);
            fb.end();
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
            fb.i32c(0).s(k);
            fb.loop_(); // $cp
            fb.g(k).g(rlen).op(op::I32_LT_U).if_(); // $cpin
            fb.g(base).i32c(28).op(op::I32_ADD).g(k).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .g(k)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end(); // $cpin
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.g(k).g(rlen).op(op::I32_LT_U).br_if(0); // $cp
            fb.end(); // $cp
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrLower => {
            let la = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.i32c(4)
                .g(la)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(string_payload);
            fb.g(string_payload)
                .i32c(STR_HEADER as i64)
                .op(op::I32_SUB)
                .s(base);
            fb.g(base).g(la).store32(24);
            fb.i32c(0).s(i);
            fb.loop_(); // $c
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $in
            fb.g(base).i32c(28).op(op::I32_ADD).g(i).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c).i32c(32).op(op::I32_ADD);
            fb.g(c);
            fb.g(c)
                .i32c(65)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(90)
                .op(op::I32_LE_U)
                .op(op::I32_AND);
            fb.op(op::SELECT);
            fb.store8(0);
            fb.end(); // $in
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.g(i).g(la).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrUpper => {
            let la = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.i32c(4)
                .g(la)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(string_payload);
            fb.g(string_payload)
                .i32c(STR_HEADER as i64)
                .op(op::I32_SUB)
                .s(base);
            fb.g(base).g(la).store32(24);
            fb.i32c(0).s(i);
            fb.loop_(); // $c
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $in
            fb.g(base).i32c(28).op(op::I32_ADD).g(i).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c).i32c(32).op(op::I32_SUB);
            fb.g(c);
            fb.g(c)
                .i32c(97)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(122)
                .op(op::I32_LE_U)
                .op(op::I32_AND);
            fb.op(op::SELECT);
            fb.store8(0);
            fb.end(); // $in
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.g(i).g(la).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::IntToStr => {
            let neg = fb.scratch(Val::I32);
            let u = fb.scratch(Val::I64);
            let cnt = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let pos = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let u2 = fb.scratch(Val::I64);
            fb.g(0).i64c(0).op(op::I64_LT_S).s(neg);
            // u = |v| in u64 (wrapping negation handles i64::MIN).
            fb.i64c(0).g(0).op(op::I64_SUB);
            fb.g(0);
            fb.g(neg);
            fb.op(op::SELECT);
            fb.s(u);
            fb.i32c(1).s(cnt);
            fb.g(u).s(u2);
            fb.loop_(); // $dc
            fb.g(u2).i64c(10).op(op::I64_GE_U).if_(); // $dci
            fb.g(u2).i64c(10).op(op::I64_DIV_U).s(u2);
            fb.g(cnt).i32c(1).op(op::I32_ADD).s(cnt);
            fb.br(1); // continue $dc (br 0 from inside the if would EXIT the loop)
            fb.end(); // $dci
            fb.end(); // $dc
            fb.g(cnt).g(neg).op(op::I32_ADD).s(rlen);
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
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .i32c(1)
                .op(op::I32_SUB)
                .s(pos); // offset, not address
            fb.block(); // $done
            fb.loop_(); // $dig
            fb.g(u).i64c(10).op(op::I64_REM_U).op(op::I32_WRAP_I64).s(d);
            fb.g(base).g(pos).op(op::I32_ADD);
            fb.g(d).i32c(48).op(op::I32_ADD);
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_SUB).s(pos);
            fb.g(u).i64c(10).op(op::I64_DIV_U).s(u);
            fb.g(u).op(op::I64_EQZ).br_if(1); // $done
            fb.br(0); // $dig
            fb.end(); // $dig
            fb.end(); // $done
            fb.g(neg).if_();
            fb.g(base).i32c(28).op(op::I32_ADD);
            fb.i32c(45);
            fb.store8(0);
            fb.end();
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::U64ToStr => {
            let u = fb.scratch(Val::I64);
            let u2 = fb.scratch(Val::I64);
            let cnt = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let pos = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            fb.g(0).s(u);
            fb.i32c(1).s(cnt);
            fb.g(u).s(u2);
            fb.loop_(); // $dc
            fb.g(u2).i64c(10).op(op::I64_GE_U).if_(); // $dci
            fb.g(u2).i64c(10).op(op::I64_DIV_U).s(u2);
            fb.g(cnt).i32c(1).op(op::I32_ADD).s(cnt);
            fb.br(1); // continue $dc (br 0 from inside the if would EXIT the loop)
            fb.end(); // $dci
            fb.end(); // $dc
            fb.g(cnt).s(rlen);
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
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .i32c(1)
                .op(op::I32_SUB)
                .s(pos); // offset, not address
            fb.block(); // $done
            fb.loop_(); // $dig
            fb.g(u).i64c(10).op(op::I64_REM_U).op(op::I32_WRAP_I64).s(d);
            fb.g(base).g(pos).op(op::I32_ADD);
            fb.g(d).i32c(48).op(op::I32_ADD);
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_SUB).s(pos);
            fb.g(u).i64c(10).op(op::I64_DIV_U).s(u);
            fb.g(u).op(op::I64_EQZ).br_if(1); // $done
            fb.br(0); // $dig
            fb.end(); // $dig
            fb.end(); // $done
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::U64ToHex => {
            let u = fb.scratch(Val::I64);
            let u2 = fb.scratch(Val::I64);
            let cnt = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let pos = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            fb.g(0).s(u);
            fb.i32c(1).s(cnt);
            fb.g(u).s(u2);
            fb.loop_(); // $dc
            fb.g(u2).i64c(16).op(op::I64_GE_U).if_(); // $dci
            fb.g(u2).i64c(16).op(op::I64_DIV_U).s(u2);
            fb.g(cnt).i32c(1).op(op::I32_ADD).s(cnt);
            fb.br(1); // continue $dc (br 0 from inside the if would EXIT the loop)
            fb.end(); // $dci
            fb.end(); // $dc
            fb.g(cnt).s(rlen);
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
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .i32c(1)
                .op(op::I32_SUB)
                .s(pos); // offset, not address
            fb.block(); // $done
            fb.loop_(); // $dig
            fb.g(u).i64c(16).op(op::I64_REM_U).op(op::I32_WRAP_I64).s(d);
            fb.g(base).g(pos).op(op::I32_ADD);
            // 0-9 -> '0'-'9', 10-15 -> 'a'-'f'
            fb.g(d).i32c(48).op(op::I32_ADD);
            fb.g(d).i32c(87).op(op::I32_ADD);
            fb.g(d).i32c(10).op(op::I32_LT_U);
            fb.op(op::SELECT);
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_SUB).s(pos);
            fb.g(u).i64c(16).op(op::I64_DIV_U).s(u);
            fb.g(u).op(op::I64_EQZ).br_if(1); // $done
            fb.br(0); // $dig
            fb.end(); // $dig
            fb.end(); // $done
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::BoolToStr => {
            let lit = |s: &str| *env.string_ptr.get(s).expect("bool to str literal pooled") + 4;
            let t = lit("true");
            let f = lit("false");
            let len = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let src = fb.scratch(Val::I32);
            let k = fb.scratch(Val::I32);
            // "true" = 4 chars, "false" = 5 chars: len = 5 - v
            fb.i32c(5).g(0).op(op::I32_SUB).s(len);
            fb.i32c(4)
                .g(len)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(string_payload);
            fb.g(string_payload)
                .i32c(STR_HEADER as i64)
                .op(op::I32_SUB)
                .s(base);
            fb.g(base).g(len).store32(24);
            fb.i32c(t as i64);
            fb.i32c(f as i64);
            fb.g(0);
            fb.op(op::SELECT); // v ? "false" : "true"
            fb.s(src);
            fb.i32c(0).s(k);
            fb.loop_(); // $c
            fb.g(k).g(len).op(op::I32_LT_U).if_(); // $in
            fb.g(base).i32c(28).op(op::I32_ADD).g(k).op(op::I32_ADD);
            fb.g(src).g(k).op(op::I32_ADD).load8(0);
            fb.store8(0);
            fb.end(); // $in
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.g(k).g(len).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrToInt => {
            let la = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let v = fb.scratch(Val::I64);
            let sgn = fb.scratch(Val::I32);
            let converted = fb.scratch(Val::I32);
            let limit = fb.scratch(Val::I64);
            fb.g(0).load32(0).s(la);
            fb.i32c(0).s(i);
            fb.i32c(0).s(converted);
            fb.i64c(0).s(v);
            fb.i32c(1).s(sgn);
            // Skip C strtoll whitespace.
            fb.block(); // $wsb
            fb.loop_(); // $ws
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $wsi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c)
                .i32c(32)
                .op(op::I32_EQ)
                .g(c)
                .i32c(9)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(10)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(11)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(12)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(13)
                .op(op::I32_EQ)
                .op(op::I32_OR);
            fb.if_(); // $isws
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(2); // continue $ws loop: 0=$isws, 1=$wsi, 2=$ws (br to the IF exits the loop in V8)
            fb.end(); // $isws
            fb.br(2); // break $ws: 0=$wsi, 1=$ws, 2=$wsb
            fb.end(); // $wsi
            fb.end(); // $ws
            fb.end(); // $wsb
            // Optional sign.
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $sgi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c).i32c(45).op(op::I32_EQ).if_(); // '-'
            fb.i32c(-1).s(sgn);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.else_();
            fb.g(c).i32c(43).op(op::I32_EQ).if_(); // '+'
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.end();
            fb.end();
            fb.end(); // $sgi
            // Leading decimal digits, saturating at i64::MAX.
            fb.block(); // $dgb
            fb.loop_(); // $dg
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $dgi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .i32c(48)
                .op(op::I32_SUB)
                .s(d);
            fb.g(d)
                .i32c(0)
                .op(op::I32_GE_S)
                .g(d)
                .i32c(9)
                .op(op::I32_LE_S)
                .op(op::I32_AND)
                .if_(); // $dig
            fb.i64c(9223372036854775807)
                .g(d)
                .op(op::I64_EXTEND_I32_S)
                .op(op::I64_SUB)
                .i64c(10)
                .op(op::I64_DIV_S)
                .s(limit);
            fb.g(v).g(limit).op(op::I64_GT_S).if_(); // overflow
            fb.i64c(9223372036854775807).s(v);
            fb.else_();
            fb.g(v)
                .i64c(10)
                .op(op::I64_MUL)
                .g(d)
                .op(op::I64_EXTEND_I32_S)
                .op(op::I64_ADD)
                .s(v);
            fb.end();
            fb.i32c(1).s(converted);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.else_();
            fb.br(3); // non-digit: break $dg: 0=$dig,1=$dgi,2=$dg,3=$dgb
            fb.end(); // $dig
            fb.br(1); // digit consumed: $dgi end (loop top)
            fb.end(); // $dgi
            fb.end(); // $dg
            fb.end(); // $dgb
            fb.g(converted).op(op::I32_EQZ).if_();
            fb.i64c(0);
            fb.op(op::RETURN);
            fb.end();
            // Result-typed ifs: the branches produce the i64 result.
            fb.g(sgn).i32c(0).op(op::I32_LT_S).if_i64(); // $neg
            fb.g(v).i64c(9223372036854775807).op(op::I64_EQ).if_i64(); // $ismax
            fb.i64c(-9223372036854775807).i64c(1).op(op::I64_SUB);
            fb.else_();
            fb.i64c(0).g(v).op(op::I64_SUB);
            fb.end(); // $ismax
            fb.else_(); // $neg
            fb.g(v);
            fb.end(); // $neg
        }
        H::StrToU64 => {
            let la = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            let c2 = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let v = fb.scratch(Val::I64);
            let hex = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.i32c(0).s(i);
            fb.i64c(0).s(v);
            fb.i32c(0).s(hex);
            // Skip whitespace (no VT/FF here: the v1 u64 parser).
            fb.block(); // $wsb
            fb.loop_(); // $ws
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $wsi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c)
                .i32c(32)
                .op(op::I32_EQ)
                .g(c)
                .i32c(9)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(10)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(13)
                .op(op::I32_EQ)
                .op(op::I32_OR);
            fb.if_(); // $isws
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(2); // continue $ws loop: 0=$isws, 1=$wsi, 2=$ws (br to the IF exits the loop in V8)
            fb.end(); // $isws
            fb.br(2); // break $ws: 0=$wsi, 1=$ws, 2=$wsb
            fb.end(); // $wsi
            fb.end(); // $ws
            fb.end(); // $wsb
            // Optional 0x / 0X prefix.
            fb.g(i).i32c(1).op(op::I32_ADD).g(la).op(op::I32_LT_U).if_(); // $hx
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c).i32c(48).op(op::I32_EQ).if_(); // '0'
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .i32c(1)
                .op(op::I32_ADD)
                .op(op::I32_ADD)
                .load8(0)
                .s(c2);
            fb.g(c2)
                .i32c(120)
                .op(op::I32_EQ)
                .g(c2)
                .i32c(88)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .if_();
            fb.g(i).i32c(2).op(op::I32_ADD).s(i);
            fb.i32c(1).s(hex);
            fb.end();
            fb.end();
            fb.end(); // $hx
            // Digits (wrapping arithmetic, the v1 contract).
            fb.block(); // $dgb
            fb.loop_(); // $dg
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $dgi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(hex).if_(); // hex digits
            fb.i32c(-1).s(d);
            fb.g(c)
                .i32c(48)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(57)
                .op(op::I32_LE_U)
                .op(op::I32_AND)
                .if_();
            fb.g(c).i32c(48).op(op::I32_SUB).s(d);
            fb.else_();
            fb.g(c)
                .i32c(97)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(102)
                .op(op::I32_LE_U)
                .op(op::I32_AND)
                .if_();
            fb.g(c).i32c(87).op(op::I32_SUB).s(d);
            fb.else_();
            fb.g(c)
                .i32c(65)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(70)
                .op(op::I32_LE_U)
                .op(op::I32_AND)
                .if_();
            fb.g(c).i32c(55).op(op::I32_SUB).s(d);
            fb.end();
            fb.end();
            fb.end();
            fb.g(d).i32c(0).op(op::I32_GE_S).if_(); // $hd
            fb.g(v)
                .i64c(4)
                .op(op::I64_SHL)
                .g(d)
                .op(op::I64_EXTEND_I32_S)
                .op(op::I64_OR)
                .s(v);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.else_();
            fb.br(4); // not a hex digit: break $dg: 0=$hd,1=hex,2=$dgi,3=$dg,4=$dgb
            fb.end(); // $hd
            fb.else_(); // decimal digits
            fb.g(c).i32c(48).op(op::I32_SUB).s(d);
            fb.g(d)
                .i32c(0)
                .op(op::I32_GE_S)
                .g(d)
                .i32c(9)
                .op(op::I32_LE_S)
                .op(op::I32_AND)
                .if_();
            fb.g(v)
                .i64c(10)
                .op(op::I64_MUL)
                .g(d)
                .op(op::I64_EXTEND_I32_S)
                .op(op::I64_ADD)
                .s(v);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.else_();
            fb.br(4); // not a decimal digit: break $dg: 0=$dd,1=hex,2=$dgi,3=$dg,4=$dgb
            fb.end(); // $dd
            fb.end();
            fb.br(1); // digit consumed: $dgi end (loop top)
            fb.end(); // $dgi
            fb.end(); // $dg
            fb.end(); // $dgb
            fb.g(v);
        }
        _ => unreachable!("helper dispatched to the wrong body group"),
    }
    Ok(())
}
