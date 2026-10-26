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
        H::FdWrite => {
            fb.i32c(IOVEC_BUF as i64);
            fb.g(0);
            fb.store32(0);
            fb.i32c((IOVEC_BUF + 4) as i64);
            fb.g(1);
            fb.store32(0);
            fb.i32c(1); // fd = stdout
            fb.i32c(IOVEC_BUF as i64);
            fb.i32c(1); // iovec count
            fb.i32c(FD_IO_OUT as i64);
            fb.call(fd_write_import_idx);
            fb.op(op::DROP);
        }
        H::FdRead => {
            fb.i32c(IOVEC_BUF as i64);
            fb.g(0);
            fb.store32(0);
            fb.i32c((IOVEC_BUF + 4) as i64);
            fb.g(1);
            fb.store32(0);
            fb.i32c(0); // fd = stdin
            fb.i32c(IOVEC_BUF as i64);
            fb.i32c(1); // iovec count
            fb.i32c(FD_IO_OUT as i64);
            fb.call(fd_read_import_idx);
            // A WASI read error is an empty read at the language boundary.
            fb.if_();
            fb.i32c(0).op(op::RETURN);
            fb.end();
            fb.i32c(FD_IO_OUT as i64).load32(0);
        }
        H::Input => {
            const INPUT_CAP: i64 = 4096;
            let raw = fb.scratch(Val::I32);
            let len = fb.scratch(Val::I32);
            let out = fb.scratch(Val::I32);
            fb.i32c(INPUT_CAP).i32c(0).call(idx(H::ArcAlloc)).s(raw);
            fb.i32c(0).s(len);
            // Read one byte at a time so one input() call cannot consume the
            // following line. The 4096-byte cap is explicit and deterministic.
            fb.block();
            fb.loop_();
            fb.g(len).i32c(INPUT_CAP).op(op::I32_GE_U).br_if(1);
            fb.g(raw)
                .g(len)
                .op(op::I32_ADD)
                .i32c(1)
                .call(idx(H::FdRead))
                .op(op::I32_EQZ)
                .br_if(1);
            fb.g(raw)
                .g(len)
                .op(op::I32_ADD)
                .load8(0)
                .i32c(10)
                .op(op::I32_EQ)
                .br_if(1);
            fb.g(len).i32c(1).op(op::I32_ADD).s(len);
            fb.br(0);
            fb.end();
            fb.end();
            // Strip the CR in a CRLF line.
            fb.g(len).op(op::I32_EQZ).if_();
            fb.else_();
            fb.g(raw)
                .g(len)
                .op(op::I32_ADD)
                .i32c(1)
                .op(op::I32_SUB)
                .load8(0)
                .i32c(13)
                .op(op::I32_EQ)
                .if_();
            fb.g(len).i32c(1).op(op::I32_SUB).s(len);
            fb.end();
            fb.end();
            fb.g(len)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(len).store32(0);
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(raw);
            fb.g(len);
            fb.memory_copy();
            fb.g(raw).call(idx(H::Release));
            fb.g(out);
        }
        H::PrintInt => {
            let mag = fb.scratch(Val::I64);
            let neg = fb.scratch(Val::I32);
            let cur = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            // neg = v < 0
            fb.g(0);
            fb.i64c(0);
            fb.op(op::I64_LT_S);
            fb.s(neg);
            // mag = v; if neg, mag = 0 - v
            fb.g(0);
            fb.s(mag);
            fb.g(neg);
            fb.if_();
            fb.i64c(0);
            fb.g(0);
            fb.op(op::I64_SUB);
            fb.s(mag);
            fb.end();
            // write digits downward from NUM_BUF_END. `mag` holds the
            // unsigned magnitude (wrapping negation already handled
            // i64::MIN, whose magnitude 2^63 only fits as u64), so the
            // digit loop must divide unsigned.
            fb.i32c(NUM_BUF_END as i64);
            fb.s(cur);
            fb.block(); // $done
            fb.loop_(); // $d
            fb.g(mag);
            fb.i64c(10);
            fb.op(op::I64_REM_U);
            fb.op(op::I32_WRAP_I64);
            fb.s(d);
            fb.g(mag);
            fb.i64c(10);
            fb.op(op::I64_DIV_U);
            fb.s(mag);
            fb.g(cur);
            fb.i32c(1);
            fb.op(op::I32_SUB);
            fb.g(d);
            fb.i32c(48);
            fb.op(op::I32_ADD);
            fb.store8(0);
            fb.g(cur);
            fb.i32c(1);
            fb.op(op::I32_SUB);
            fb.s(cur);
            fb.g(mag);
            fb.op(op::I64_EQZ);
            fb.br_if(1); // $done
            fb.br(0); // $d
            fb.end(); // $d
            fb.end(); // $done
            // sign
            fb.g(neg);
            fb.if_();
            fb.g(cur);
            fb.i32c(1);
            fb.op(op::I32_SUB); // addr = cur - 1
            fb.i32c(45); // '-' byte
            fb.store8(0);
            fb.g(cur);
            fb.i32c(1);
            fb.op(op::I32_SUB);
            fb.s(cur);
            fb.end();
            // write [cur .. NUM_BUF_END)
            fb.g(cur);
            fb.i32c(NUM_BUF_END as i64);
            fb.g(cur);
            fb.op(op::I32_SUB);
            fb.call(fd_helper_idx);
            // newline
            fb.i32c(NUM_BUF as i64);
            fb.i32c(10);
            fb.store8(0);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(1);
            fb.call(fd_helper_idx);
        }
        H::PrintBool => {
            fb.g(0);
            fb.i32c(1);
            fb.op(op::I32_EQ);
            fb.if_();
            fb.i32c(NUM_BUF as i64);
            fb.i32c(49);
            fb.store8(0);
            fb.i32c((NUM_BUF + 1) as i64);
            fb.i32c(10);
            fb.store8(0);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(2);
            fb.call(fd_helper_idx);
            fb.else_();
            fb.i32c(NUM_BUF as i64);
            fb.i32c(48);
            fb.store8(0);
            fb.i32c((NUM_BUF + 1) as i64);
            fb.i32c(10);
            fb.store8(0);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(2);
            fb.call(fd_helper_idx);
            fb.end();
        }
        H::PrintStr => {
            fb.g(0);
            fb.i32c(4);
            fb.op(op::I32_ADD);
            fb.g(0);
            fb.load32(0);
            fb.call(fd_helper_idx);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(10);
            fb.store8(0);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(1);
            fb.call(fd_helper_idx);
        }
        H::WriteStr => {
            fb.g(0);
            fb.i32c(4);
            fb.op(op::I32_ADD);
            fb.g(0);
            fb.load32(0);
            fb.call(fd_helper_idx);
        }
        H::StrEq => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let ba = fb.scratch(Val::I32);
            let bb = fb.scratch(Val::I32);
            fb.g(0);
            fb.load32(0);
            fb.s(la);
            fb.g(1);
            fb.load32(0);
            fb.s(lb);
            fb.g(la);
            fb.g(lb);
            fb.op(op::I32_NE);
            fb.if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.i32c(0);
            fb.s(i);
            fb.block(); // $done
            fb.loop_(); // $c
            fb.g(i);
            fb.g(la);
            fb.op(op::I32_GE_S);
            fb.br_if(1);
            fb.g(0);
            fb.i32c(4);
            fb.op(op::I32_ADD);
            fb.g(i);
            fb.op(op::I32_ADD);
            fb.load8(0);
            fb.s(ba);
            fb.g(1);
            fb.i32c(4);
            fb.op(op::I32_ADD);
            fb.g(i);
            fb.op(op::I32_ADD);
            fb.load8(0);
            fb.s(bb);
            fb.g(ba);
            fb.g(bb);
            fb.op(op::I32_NE);
            fb.if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.g(i);
            fb.i32c(1);
            fb.op(op::I32_ADD);
            fb.s(i);
            fb.br(0);
            fb.end(); // $c
            fb.end(); // $done
            fb.i32c(1);
            fb.op(op::RETURN);
        }
        H::StrLen => {
            fb.g(0);
            fb.load32(0);
            fb.op(op::I64_EXTEND_I32_U);
        }
        // ── freestanding numeric helpers ────────────────────────────────
        H::Log2 => {
            let bits = fb.scratch(Val::I64);
            let exponent = fb.scratch(Val::I64);
            let mantissa = fb.scratch(Val::F64);
            let t = fb.scratch(Val::F64);
            let t2 = fb.scratch(Val::F64);
            let term = fb.scratch(Val::F64);
            let sum = fb.scratch(Val::F64);
            let i = fb.scratch(Val::I32);
            fb.g(0).op(op::I64_REINTERPRET_F64).s(bits);
            fb.g(bits)
                .i64c(52)
                .op(op::I64_SHR_U)
                .i64c(2047)
                .op(op::I64_AND)
                .i64c(1023)
                .op(op::I64_SUB)
                .s(exponent);
            fb.g(bits)
                .i64c(0x800f_ffff_ffff_ffffu64 as i64)
                .op(op::I64_AND);
            fb.i64c(1023).i64c(52).op(op::I64_SHL).op(op::I64_OR);
            fb.op(op::F64_REINTERPRET_I64).s(mantissa);
            fb.g(mantissa).f64c(1.0).op(op::F64_SUB);
            fb.g(mantissa)
                .f64c(1.0)
                .op(op::F64_ADD)
                .op(op::F64_DIV)
                .s(t);
            fb.g(t).g(t).op(op::F64_MUL).s(t2);
            fb.g(t).s(term);
            fb.g(t).s(sum);
            fb.i32c(3).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).i32c(25).op(op::I32_GE_S).br_if(1);
            fb.g(term).g(t2).op(op::F64_MUL).s(term);
            fb.g(sum)
                .g(term)
                .g(i)
                .op(op::F64_CONVERT_I32_S)
                .op(op::F64_DIV)
                .op(op::F64_ADD)
                .s(sum);
            fb.g(i).i32c(2).op(op::I32_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(exponent).op(op::F64_CONVERT_I64_S);
            fb.g(sum)
                .f64c(2.0)
                .op(op::F64_MUL)
                .f64c(std::f64::consts::LN_2)
                .op(op::F64_DIV)
                .op(op::F64_ADD);
        }
        H::Exp2 => {
            let integer = fb.scratch(Val::I64);
            let fraction = fb.scratch(Val::F64);
            let u = fb.scratch(Val::F64);
            let term = fb.scratch(Val::F64);
            let sum = fb.scratch(Val::F64);
            let i = fb.scratch(Val::I32);
            fb.g(0).f64c(1024.0).op(op::F64_GE).if_();
            fb.i64c(0x7ff0_0000_0000_0000)
                .op(op::F64_REINTERPRET_I64)
                .op(op::RETURN);
            fb.end();
            fb.g(0).f64c(-1075.0).op(op::F64_LT).if_();
            fb.f64c(0.0).op(op::RETURN);
            fb.end();
            fb.g(0).op(op::F64_FLOOR).op(op::I64_TRUNC_F64_S).s(integer);
            fb.g(0)
                .g(integer)
                .op(op::F64_CONVERT_I64_S)
                .op(op::F64_SUB)
                .s(fraction);
            fb.g(integer).i64c(-1022).op(op::I64_LT_S).if_();
            fb.i64c(-1022).s(integer);
            fb.end();
            fb.g(integer).i64c(1023).op(op::I64_GT_S).if_();
            fb.i64c(1023).s(integer);
            fb.end();
            fb.g(fraction)
                .f64c(std::f64::consts::LN_2)
                .op(op::F64_MUL)
                .s(u);
            fb.f64c(1.0).s(sum);
            fb.f64c(1.0).s(term);
            fb.i32c(1).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).i32c(15).op(op::I32_GE_S).br_if(1);
            fb.g(term)
                .g(u)
                .op(op::F64_MUL)
                .g(i)
                .op(op::F64_CONVERT_I32_S)
                .op(op::F64_DIV)
                .s(term);
            fb.g(sum).g(term).op(op::F64_ADD).s(sum);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(sum);
            fb.g(integer)
                .i64c(1023)
                .op(op::I64_ADD)
                .i64c(52)
                .op(op::I64_SHL)
                .op(op::F64_REINTERPRET_I64);
            fb.op(op::F64_MUL);
        }
        H::Pow => {
            let integer_exp = fb.scratch(Val::I64);
            let result = fb.scratch(Val::F64);
            fb.g(1).f64c(0.0).op(op::F64_EQ).if_();
            fb.f64c(1.0).op(op::RETURN);
            fb.end();
            fb.g(0).f64c(1.0).op(op::F64_EQ).if_();
            fb.f64c(1.0).op(op::RETURN);
            fb.end();
            fb.g(0).f64c(0.0).op(op::F64_EQ).if_();
            fb.g(1).f64c(0.0).op(op::F64_LT).if_();
            fb.i64c(0x7ff0_0000_0000_0000)
                .op(op::F64_REINTERPRET_I64)
                .op(op::RETURN);
            fb.end();
            fb.f64c(0.0).op(op::RETURN);
            fb.end();
            fb.g(0).f64c(0.0).op(op::F64_LT).if_();
            fb.g(1).op(op::F64_TRUNC).g(1).op(op::F64_NE).if_();
            fb.f64c(f64::NAN).op(op::RETURN);
            fb.end();
            fb.g(1).op(op::I64_TRUNC_F64_S).s(integer_exp);
            fb.g(0).op(op::F64_NEG).call(idx(H::Log2));
            fb.g(1).op(op::F64_MUL).call(idx(H::Exp2)).s(result);
            fb.g(integer_exp)
                .i64c(1)
                .op(op::I64_AND)
                .op(op::I32_WRAP_I64)
                .if_();
            fb.g(result).op(op::F64_NEG).s(result);
            fb.end();
            fb.g(result).op(op::RETURN);
            fb.end();
            fb.g(0).call(idx(H::Log2));
            fb.g(1).op(op::F64_MUL).call(idx(H::Exp2));
        }
        H::Sin | H::Cos => {
            let reduced = fb.scratch(Val::F64);
            let square = fb.scratch(Val::F64);
            let term = fb.scratch(Val::F64);
            let sum = fb.scratch(Val::F64);
            let k = fb.scratch(Val::I32);
            // Reduce to [-pi, pi) before evaluating a bounded Taylor series.
            fb.g(0).f64c(std::f64::consts::PI).op(op::F64_ADD);
            fb.f64c(std::f64::consts::TAU)
                .op(op::F64_DIV)
                .op(op::F64_FLOOR)
                .f64c(std::f64::consts::TAU)
                .op(op::F64_MUL)
                .s(reduced);
            fb.g(0).g(reduced).op(op::F64_SUB).s(reduced);
            fb.g(reduced).g(reduced).op(op::F64_MUL).s(square);
            if h == H::Sin {
                fb.g(reduced).s(term);
                fb.g(reduced).s(sum);
            } else {
                fb.f64c(1.0).s(term);
                fb.f64c(1.0).s(sum);
            }
            fb.i32c(1).s(k);
            fb.block();
            fb.loop_();
            fb.g(k).i32c(12).op(op::I32_GE_S).br_if(1);
            fb.g(term).g(square).op(op::F64_MUL).op(op::F64_NEG);
            if h == H::Sin {
                fb.g(k).i32c(2).op(op::I32_MUL).op(op::F64_CONVERT_I32_S);
                fb.g(k)
                    .i32c(2)
                    .op(op::I32_MUL)
                    .i32c(1)
                    .op(op::I32_ADD)
                    .op(op::F64_CONVERT_I32_S);
            } else {
                fb.g(k)
                    .i32c(2)
                    .op(op::I32_MUL)
                    .i32c(1)
                    .op(op::I32_SUB)
                    .op(op::F64_CONVERT_I32_S);
                fb.g(k).i32c(2).op(op::I32_MUL).op(op::F64_CONVERT_I32_S);
            }
            fb.op(op::F64_MUL).op(op::F64_DIV).s(term);
            fb.g(sum).g(term).op(op::F64_ADD).s(sum);
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(sum);
        }
        // ── 5D2a ARC heap ───────────────────────────────────────────────
        H::Alloc => {
            let hptr = fb.scratch(Val::I32);
            let new = fb.scratch(Val::I32);
            let need = fb.scratch(Val::I32);
            // size = (size + 7) & -8
            fb.g(0)
                .i32c(7)
                .op(op::I32_ADD)
                .i32c(-8)
                .op(op::I32_AND)
                .s(0);
            // h = heap; new = h + size
            fb.gget(GLOBAL_HEAP).s(hptr);
            fb.g(hptr).g(0).op(op::I32_ADD).s(new);
            // if (u32)new > memory.size() << 16: grow max(need, 32)
            fb.g(new)
                .memory_size()
                .i32c(16)
                .op(op::I32_SHL)
                .op(op::I32_GT_U)
                .if_();
            // need = (new + 65535) >>> 16 - memory.size()
            fb.g(new)
                .i32c(65535)
                .op(op::I32_ADD)
                .i32c(16)
                .op(op::I32_SHR_U)
                .memory_size()
                .op(op::I32_SUB)
                .s(need);
            fb.g(need)
                .i32c(32)
                .g(need)
                .i32c(32)
                .op(op::I32_GT_U)
                .op(op::SELECT)
                .memory_grow();
            fb.i32c(-1).op(op::I32_EQ).if_().op(op::UNREACHABLE).end();
            fb.end();
            // heap = new; return h
            fb.g(new).gset(GLOBAL_HEAP);
            fb.g(hptr);
        }
        H::ArcAlloc => {
            let base = fb.scratch(Val::I32);
            let total = fb.scratch(Val::I32);
            let exponent = fb.scratch(Val::I32);
            // Round each ARC allocation to a power-of-two size class,
            // with a 32-byte minimum (header + payload). On final
            // release, blocks return to one of these segregated bins.
            // The class exponent is packed into the high 32 bits of the
            // otherwise-unused ARC magic word at header+16; the low word
            // retains ARC_MAGIC for diagnostics. Free blocks use payload
            // word 0 as the intrusive next pointer.
            fb.g(0).i32c(ARC_HEADER).op(op::I32_ADD).s(total);
            fb.i32c(32)
                .g(total)
                .i32c(1)
                .op(op::I32_SUB)
                .op(op::I32_CLZ)
                .op(op::I32_SUB)
                .s(exponent);
            // For the smallest request (24-byte header + zero payload),
            // ceil(log2(24)) is already 5. Reject unrepresentable
            // >2-GiB nodes rather than wrapping the free-list index.
            fb.g(exponent)
                .i32c(ARC_MAX_EXP)
                .op(op::I32_GT_U)
                .if_()
                .op(op::UNREACHABLE)
                .end();

            // Pop the head of the exact-size class, if present.
            fb.i32c(0).s(base);
            for class_index in 0..ARC_FREE_BIN_COUNT {
                let class_exp = ARC_MIN_EXP + class_index as i64;
                fb.g(exponent).i32c(class_exp).op(op::I32_EQ).if_();
                let head_global = ARC_FREE_BIN_BASE + class_index;
                fb.gget(head_global).s(base);
                fb.g(base).if_();
                fb.g(base)
                    .i32c(ARC_HEADER)
                    .op(op::I32_ADD)
                    .load32(0)
                    .gset(head_global);
                fb.end();
                fb.end();
            }
            // A miss allocates one class-sized block from the linear
            // memory bump heap. Same-shape ARC churn reuses in O(1).
            fb.g(base).op(op::I32_EQZ).if_();
            fb.i32c(1)
                .g(exponent)
                .op(op::I32_SHL)
                .call(idx(H::Alloc))
                .s(base);
            fb.end();

            fb.g(base).i64c(1).store64(0);
            fb.g(base).g(1).op(op::I64_EXTEND_I32_U).store64(8);
            fb.g(base)
                .g(exponent)
                .op(op::I64_EXTEND_I32_U)
                .i64c(32)
                .op(op::I64_SHL)
                .i64c(ARC_MAGIC)
                .op(op::I64_OR)
                .store64(16);
            // payload = base + 24
            fb.g(base).i32c(ARC_HEADER).op(op::I32_ADD);
        }
        H::Retain => {
            fb.g(0).if_();
            // if rc != IMMORTAL: rc += 1
            fb.g(0)
                .i32c(24)
                .op(op::I32_SUB)
                .load64(0)
                .i64c(IMMORTAL_RC)
                .op(op::I64_NE)
                .if_();
            fb.g(0).i32c(24).op(op::I32_SUB); // addr
            fb.g(0).i32c(24).op(op::I32_SUB).load64(0); // rc
            fb.i64c(1).op(op::I64_ADD).store64(0);
            fb.end();
            fb.end();
            // Pass the value through: the caller's copy is still owned.
            fb.g(0);
        }
        H::Release => {
            let rc = fb.scratch(Val::I64);
            let drop_idx = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let exponent = fb.scratch(Val::I32);
            fb.g(0).if_();
            fb.g(0).i32c(24).op(op::I32_SUB).load64(0).s(rc);
            fb.g(rc).i64c(IMMORTAL_RC).op(op::I64_NE).if_();
            // rc < 1 is a double release: fail loudly.
            fb.g(rc)
                .i64c(1)
                .op(op::I64_LT_S)
                .if_()
                .op(op::UNREACHABLE)
                .end();
            fb.g(rc).i64c(1).op(op::I64_EQ).if_();
            // Dying: mark, then dispatch the destructor if present.
            fb.g(0).i32c(24).op(op::I32_SUB).s(base);
            fb.g(base).i64c(0).store64(0);
            fb.g(base)
                .load64(16)
                .i64c(32)
                .op(op::I64_SHR_U)
                .op(op::I32_WRAP_I64)
                .s(exponent);
            fb.g(base).load64(8).op(op::I32_WRAP_I64).s(drop_idx);
            fb.g(drop_idx).if_();
            fb.g(0).g(drop_idx).call_indirect(env.drop_call_type, 0); // drop table
            fb.end();
            // Destructor recursion is complete; this block is now safe
            // to publish to its size-class free list.
            for class_index in 0..ARC_FREE_BIN_COUNT {
                let class_exp = ARC_MIN_EXP + class_index as i64;
                fb.g(exponent).i32c(class_exp).op(op::I32_EQ).if_();
                let head_global = ARC_FREE_BIN_BASE + class_index;
                fb.g(base)
                    .i32c(ARC_HEADER)
                    .op(op::I32_ADD)
                    .gget(head_global)
                    .store32(0);
                fb.g(base).gset(head_global);
                fb.end();
            }
            fb.else_();
            fb.g(0)
                .i32c(24)
                .op(op::I32_SUB)
                .g(rc)
                .i64c(1)
                .op(op::I64_SUB)
                .store64(0);
            fb.end();
            fb.end();
            fb.end();
        }
        H::PrintFloat => {
            // The oracle prints floats with Rust `{:.6}`: `NaN`, `inf`,
            // `-inf`, a sign for negative zero, and no exponent form.
            // string_ptr points at the entry's len field; the raw bytes
            // start 4 bytes past it (PrintStr applies the same +4).
            let lit = |s: &str| *env.string_ptr.get(s).expect("print float literal pooled") + 4;
            let dash = lit("-");
            let dot = lit(".");
            let nan = lit("NaN\n");
            let inf = lit("inf\n");
            let neginf = lit("-inf\n");
            let zeros = lit(".000000\n");
            let neg = fb.scratch(Val::I32);
            let n = fb.scratch(Val::I64);
            let ip = fb.scratch(Val::I64);
            let cur = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let pos = fb.scratch(Val::I32);
            macro_rules! write_u64 {
                ($v:expr) => {{
                    // digits written downward from NUM_BUF_END
                    fb.i32c(NUM_BUF_END as i64).s(cur);
                    fb.block(); // $done
                    fb.loop_(); // $d
                    fb.g($v)
                        .i64c(10)
                        .op(op::I64_REM_U)
                        .op(op::I32_WRAP_I64)
                        .s(d);
                    fb.g($v).i64c(10).op(op::I64_DIV_U).s($v);
                    fb.g(cur)
                        .i32c(1)
                        .op(op::I32_SUB)
                        .g(d)
                        .i32c(48)
                        .op(op::I32_ADD)
                        .store8(0);
                    fb.g(cur).i32c(1).op(op::I32_SUB).s(cur);
                    fb.g($v).op(op::I64_EQZ).br_if(1); // $done
                    fb.br(0); // $d
                    fb.end(); // $d
                    fb.end(); // $done
                    // write [cur .. NUM_BUF_END)
                    fb.g(cur)
                        .i32c(NUM_BUF_END as i64)
                        .g(cur)
                        .op(op::I32_SUB)
                        .call(fd_helper_idx);
                }};
            }
            // NaN?
            fb.g(0).g(0).op(op::F64_NE).if_();
            fb.i32c(nan as i64).i32c(4).call(fd_helper_idx);
            fb.op(op::RETURN);
            fb.end();
            // ±inf?
            fb.g(0)
                .f64c(0.0)
                .op(op::F64_MUL)
                .f64c(0.0)
                .op(op::F64_NE)
                .if_();
            fb.g(0).f64c(0.0).op(op::F64_LT).if_();
            fb.i32c(neginf as i64).i32c(5).call(fd_helper_idx);
            fb.else_();
            fb.i32c(inf as i64).i32c(4).call(fd_helper_idx);
            fb.end();
            fb.op(op::RETURN);
            fb.end();
            // sign (a sign-bit test: Rust's `{:.6}` prints `-0.000000`)
            fb.g(0)
                .op(op::I64_REINTERPRET_F64)
                .i64c(i64::MIN)
                .op(op::I64_AND)
                .i64c(0)
                .op(op::I64_NE)
                .s(neg);
            fb.g(0).op(op::F64_ABS).s(0);
            fb.g(neg).if_();
            fb.i32c(dash as i64).i32c(1).call(fd_helper_idx);
            fb.end();
            // fixed 6-digit fraction for |x| < 9e12
            fb.g(0).f64c(9.0e12).op(op::F64_LT).if_();
            fb.g(0)
                .f64c(1_000_000.0)
                .op(op::F64_MUL)
                .f64c(0.5)
                .op(op::F64_ADD)
                .op(op::I64_TRUNC_F64_U)
                .s(n);
            fb.g(n).i64c(1_000_000).op(op::I64_DIV_U).s(ip);
            write_u64!(ip);
            fb.i32c(dot as i64).i32c(1).call(fd_helper_idx);
            // frac = n % 1e6, exactly 6 digits into NUM_BUF
            fb.g(n).i64c(1_000_000).op(op::I64_REM_U).s(n);
            fb.i32c(6).s(pos);
            fb.loop_();
            fb.g(pos).i32c(1).op(op::I32_SUB).t(pos);
            fb.i32c(NUM_BUF as i64).op(op::I32_ADD);
            fb.g(n)
                .i64c(10)
                .op(op::I64_REM_U)
                .op(op::I32_WRAP_I64)
                .i32c(48)
                .op(op::I32_ADD)
                .store8(0);
            fb.g(n).i64c(10).op(op::I64_DIV_U).s(n);
            fb.g(pos).br_if(0);
            fb.end();
            fb.i32c(NUM_BUF as i64).i32c(6).call(fd_helper_idx);
            // newline
            fb.i32c(NUM_BUF as i64).i32c(10).store8(0);
            fb.i32c(NUM_BUF as i64).i32c(1).call(fd_helper_idx);
            fb.else_();
            // |x| >= 9e12: integer part + ".000000\n"
            fb.g(0).op(op::I64_TRUNC_F64_U).s(ip);
            write_u64!(ip);
            fb.i32c(zeros as i64).i32c(8).call(fd_helper_idx);
            fb.end();
        }
        // ── 5D2a lists (payloads of i64 slots) ─────────────────────────
        _ => unreachable!("helper dispatched to the wrong body group"),
    }
    Ok(())
}
