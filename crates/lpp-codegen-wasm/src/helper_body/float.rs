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
        H::FloatToStr => {
            // The oracle's format_percent_g: C `%g` with six significant
            // digits — trailing zeros stripped, exponent form when the
            // exponent is below -4 or at/above 6.
            //
            // Modeled on rustc's flt2dec (library/core/src/num/imp/flt2dec):
            // classify first, then generate exact digits, then render.
            // Every loop here has a provably bounded trip count, so no
            // finite input can spin (rustc requires totality; we have it
            // by construction).
            let lit = |s: &str| *env.string_ptr.get(s).expect("float to str literal pooled") + 4;
            let nan = lit("nan");
            let inf = lit("inf");
            let neginf = lit("-inf");
            let negzero = lit("-0");
            let pos = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let string_payload = fb.scratch(Val::I32);
            let exp = fb.scratch(Val::I32);
            let t = fb.scratch(Val::F64);
            let sigval = fb.scratch(Val::I64);
            let frac = fb.scratch(Val::F64);
            let gt = fb.scratch(Val::I32);
            let eq = fb.scratch(Val::I32);
            let odd = fb.scratch(Val::I32);
            let up = fb.scratch(Val::I32);
            let sig = fb.scratch(Val::I32);
            let p10 = fb.scratch(Val::I64);
            let d0 = fb.scratch(Val::I64);
            let rest = fb.scratch(Val::I64);
            let head = fb.scratch(Val::I64);
            let tail = fb.scratch(Val::I64);
            let k = fb.scratch(Val::I32);
            let w = fb.scratch(Val::I32);
            let z = fb.scratch(Val::I32);
            let ae = fb.scratch(Val::I32);
            let neg = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let split = fb.scratch(Val::I32);
            // ── phase 0: classify; each branch writes NUM_BUF, sets pos,
            // and exits through the if/else shape (no fall-through into
            // the normalizing loops). ──
            fb.g(0).g(0).op(op::F64_NE).if_(); // NaN
            for (off, src) in [(0u32, nan), (1u32, nan), (2u32, nan)] {
                fb.i32c((NUM_BUF + off) as i64);
                fb.i32c(src as i64)
                    .i32c(off as i64)
                    .op(op::I32_ADD)
                    .load8(0);
                fb.store8(0);
            }
            fb.i32c(3).s(pos);
            fb.else_();
            fb.g(0)
                .f64c(0.0)
                .op(op::F64_MUL)
                .f64c(0.0)
                .op(op::F64_NE)
                .if_(); // ±inf
            fb.g(0)
                .op(op::I64_REINTERPRET_F64)
                .i64c(i64::MIN)
                .op(op::I64_AND)
                .i64c(0)
                .op(op::I64_NE)
                .s(neg);
            fb.g(neg).if_();
            for (off, src) in [
                (0u32, neginf),
                (1u32, neginf),
                (2u32, neginf),
                (3u32, neginf),
            ] {
                fb.i32c((NUM_BUF + off) as i64);
                fb.i32c(src as i64)
                    .i32c(off as i64)
                    .op(op::I32_ADD)
                    .load8(0);
                fb.store8(0);
            }
            fb.i32c(4).s(pos);
            fb.else_();
            for (off, src) in [(0u32, inf), (1u32, inf), (2u32, inf)] {
                fb.i32c((NUM_BUF + off) as i64);
                fb.i32c(src as i64)
                    .i32c(off as i64)
                    .op(op::I32_ADD)
                    .load8(0);
                fb.store8(0);
            }
            fb.i32c(3).s(pos);
            fb.end(); // sign of ±inf
            fb.else_();
            // ── finite ──
            fb.g(0)
                .op(op::I64_REINTERPRET_F64)
                .i64c(i64::MIN)
                .op(op::I64_AND)
                .i64c(0)
                .op(op::I64_NE)
                .s(neg);
            fb.g(0).f64c(0.0).op(op::F64_EQ).if_(); // ±0
            fb.g(neg).if_();
            for (off, src) in [(0u32, negzero), (1u32, negzero)] {
                fb.i32c((NUM_BUF + off) as i64);
                fb.i32c(src as i64)
                    .i32c(off as i64)
                    .op(op::I32_ADD)
                    .load8(0);
                fb.store8(0);
            }
            fb.i32c(2).s(pos);
            fb.else_();
            fb.i32c(NUM_BUF as i64);
            fb.i32c(48);
            fb.store8(0);
            fb.i32c(1).s(pos);
            fb.end();
            fb.else_();
            // ── phase 1: |val| = 10^exp, 1 <= |val| < 10 (bounded: each
            // pass divides/multiplies by 10; a finite non-zero f64
            // converges). ──
            fb.g(0).op(op::F64_ABS).s(t);
            fb.i32c(0).s(exp);
            fb.loop_(); // $up
            fb.g(t).f64c(10.0).op(op::F64_GE).if_(); // $upi
            fb.g(t).f64c(10.0).op(op::F64_DIV).s(t);
            fb.g(exp).i32c(1).op(op::I32_ADD).s(exp);
            fb.br(1); // continue $up
            fb.end(); // $upi
            fb.end(); // $up
            fb.loop_(); // $dn
            fb.g(t).f64c(1.0).op(op::F64_LT).if_(); // $dni
            fb.g(t).f64c(10.0).op(op::F64_MUL).s(t);
            fb.g(exp).i32c(1).op(op::I32_SUB).s(exp);
            fb.br(1); // continue $dn
            fb.end(); // $dni
            fb.end(); // $dn
            // ── phase 2: six significant digits, correctly rounded ──
            fb.g(t).f64c(100000.0).op(op::F64_MUL).s(t);
            fb.g(t).op(op::I64_TRUNC_F64_U).s(sigval);
            fb.g(t)
                .g(sigval)
                .op(op::F64_CONVERT_I64_S)
                .op(op::F64_SUB)
                .s(frac);
            fb.g(frac).f64c(0.5).op(op::F64_GT).s(gt);
            fb.g(frac).f64c(0.5).op(op::F64_EQ).s(eq);
            fb.g(sigval)
                .i64c(1)
                .op(op::I64_REM_U)
                .i64c(1)
                .op(op::I64_EQ)
                .s(odd);
            fb.g(gt).g(eq).g(odd).op(op::I32_AND).op(op::I32_OR).s(up);
            fb.g(up).if_();
            fb.g(sigval).i64c(1).op(op::I64_ADD).s(sigval);
            fb.end();
            fb.g(sigval).i64c(1000000).op(op::I64_EQ).if_(); // carry
            fb.i64c(100000).s(sigval);
            fb.g(exp).i32c(1).op(op::I32_ADD).s(exp);
            fb.end();
            // Strip trailing zeros: sigval keeps `sig` digits (>= 1).
            fb.i32c(6).s(sig);
            fb.loop_(); // $tz
            fb.g(sigval)
                .i64c(10)
                .op(op::I64_REM_U)
                .i64c(0)
                .op(op::I64_EQ)
                .g(sig)
                .i32c(1)
                .op(op::I32_GT_S)
                .op(op::I32_AND)
                .if_(); // $tzi
            fb.g(sigval).i64c(10).op(op::I64_DIV_U).s(sigval);
            fb.g(sig).i32c(1).op(op::I32_SUB).s(sig);
            fb.br(1); // continue $tz
            fb.end(); // $tzi
            fb.end(); // $tz
            // ── phase 3: sign ──
            fb.g(neg).if_();
            fb.i32c(NUM_BUF as i64);
            fb.i32c(45);
            fb.store8(0);
            fb.i32c(1).s(pos);
            fb.else_();
            fb.i32c(0).s(pos);
            fb.end();
            // ── phase 4: render (C %g: exponent form when exp < -4 or
            // exp >= 6, else plain form) ──
            fb.g(exp)
                .i32c(-4)
                .op(op::I32_LT_S)
                .g(exp)
                .i32c(6)
                .op(op::I32_GE_S)
                .op(op::I32_OR)
                .if_(); // ── exponent form ──
            fb.g(sig).i32c(1).op(op::I32_SUB).s(z);
            emit_pow10(fb, z, p10, k); // p10 = 10^(sig-1)
            fb.g(sigval).g(p10).op(op::I64_DIV_U).s(d0);
            fb.g(sigval).g(p10).op(op::I64_REM_U).s(rest);
            fb.i32c(1).s(w);
            emit_digits(fb, pos, w, k, d, d0); // leading digit
            fb.g(sig).i32c(1).op(op::I32_GT_S).if_(); // more digits
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(46); // '.'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.g(sig).i32c(1).op(op::I32_SUB).s(w);
            emit_digits(fb, pos, w, k, d, rest); // padded to sig-1
            fb.end();
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(101); // 'e'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.g(exp).i32c(0).op(op::I32_LT_S).if_();
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(45); // '-'
            fb.store8(0);
            fb.else_();
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(43); // '+'
            fb.store8(0);
            fb.end();
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.g(exp);
            fb.i32c(0).g(exp).op(op::I32_SUB);
            fb.g(exp).i32c(0).op(op::I32_GE_S);
            fb.op(op::SELECT); // ae = |exp|
            fb.s(ae);
            fb.g(ae).i32c(100).op(op::I32_GE_S).if_();
            fb.i32c(3).s(w);
            fb.else_();
            fb.g(ae).i32c(10).op(op::I32_GE_S).if_();
            fb.i32c(2).s(w);
            fb.else_();
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(48); // leading '0'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.i32c(1).s(w);
            fb.end();
            fb.end();
            fb.g(ae).op(op::I64_EXTEND_I32_S).s(d0);
            emit_digits(fb, pos, w, k, d, d0);
            fb.else_(); // ── plain form ──
            fb.g(exp).i32c(0).op(op::I32_GE_S).if_(); // exp >= 0
            fb.g(exp)
                .g(sig)
                .i32c(1)
                .op(op::I32_SUB)
                .op(op::I32_GE_S)
                .if_(); // whole number
            fb.g(sig).s(w);
            emit_digits(fb, pos, w, k, d, sigval);
            fb.g(exp)
                .g(sig)
                .i32c(1)
                .op(op::I32_SUB)
                .op(op::I32_SUB)
                .s(z); // z = exp - sig + 1 (>= 0 here)
            emit_zeros(fb, pos, z, k);
            fb.else_(); // split: digits '.' digits
            fb.g(exp).i32c(1).op(op::I32_ADD).s(split);
            fb.g(sig).g(split).op(op::I32_SUB).s(z);
            emit_pow10(fb, z, p10, k); // p10 = 10^(sig-split)
            fb.g(sigval).g(p10).op(op::I64_DIV_U).s(head);
            fb.g(sigval).g(p10).op(op::I64_REM_U).s(tail);
            fb.g(split).s(w);
            emit_digits(fb, pos, w, k, d, head);
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(46); // '.'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.g(sig).g(split).op(op::I32_SUB).s(w);
            emit_digits(fb, pos, w, k, d, tail);
            fb.end(); // whole number
            fb.else_(); // 0.00…digits
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(48); // '0'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(46); // '.'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.i32c(0)
                .g(exp)
                .op(op::I32_SUB)
                .i32c(1)
                .op(op::I32_SUB)
                .s(z);
            emit_zeros(fb, pos, z, k);
            fb.g(sig).s(w);
            emit_digits(fb, pos, w, k, d, sigval);
            fb.end(); // exp >= 0
            fb.end(); // plain form
            fb.end(); // finite non-zero
            fb.end(); // inf
            fb.end(); // NaN
            // ── epilogue: the scratch became the string ──
            fb.g(pos).s(rlen);
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
            fb.i32c(NUM_BUF as i64).g(k).op(op::I32_ADD).load8(0);
            fb.store8(0);
            fb.end(); // $cpin
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.g(k).g(rlen).op(op::I32_LT_U).br_if(0); // $cp
            fb.end(); // $cp
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        _ => unreachable!("helper dispatched to the wrong body group"),
    }
    Ok(())
}
