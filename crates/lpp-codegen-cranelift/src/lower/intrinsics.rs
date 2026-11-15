use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{InstBuilder, MemFlags, Type as CLType, Value, types as cltypes};
use cranelift_frontend::FunctionBuilder;
use lpp_codegen_api::CodegenError;
use lpp_mir::{KernelIntrinsic, Operand};
use lpp_types::{PrimitiveType, TypeId, TypeKind};

use super::{
    Inst, Lowering, check_value_type, coerce_to, machine_type, operand_language_kind,
    static_type_size_align, unsupported,
};

pub(super) fn intrinsic_arch_import(intrinsic: KernelIntrinsic) -> Option<&'static str> {
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
        KernelIntrinsic::SetNew => "lpp_set_new",
        KernelIntrinsic::SetAdd => "lpp_set_add",
        KernelIntrinsic::SetHas => "lpp_set_has",
        KernelIntrinsic::SetRemove => "lpp_set_remove",
        KernelIntrinsic::SetAddStr => "lpp_set_add_str",
        KernelIntrinsic::SetHasStr => "lpp_set_has_str",
        KernelIntrinsic::SetRemoveStr => "lpp_set_remove_str",
        KernelIntrinsic::SetLen => "lpp_set_len",
        KernelIntrinsic::SetClear => "lpp_set_clear",
        KernelIntrinsic::SetFree => "lpp_set_free",
        KernelIntrinsic::SetUnion => "lpp_set_union",
        KernelIntrinsic::SetIntersect => "lpp_set_intersect",
        KernelIntrinsic::DequeNew => "lpp_deque_new",
        KernelIntrinsic::DequePushBack => "lpp_deque_push_back",
        KernelIntrinsic::DequePushFront => "lpp_deque_push_front",
        KernelIntrinsic::DequePopBack => "lpp_deque_pop_back",
        KernelIntrinsic::DequePopFront => "lpp_deque_pop_front",
        KernelIntrinsic::DequePeekFront => "lpp_deque_peek_front",
        KernelIntrinsic::DequePeekBack => "lpp_deque_peek_back",
        KernelIntrinsic::DequeGet => "lpp_deque_get",
        KernelIntrinsic::DequeLen => "lpp_deque_len",
        KernelIntrinsic::DequeClear => "lpp_deque_clear",
        KernelIntrinsic::DequeFree => "lpp_deque_free",
        _ => return None,
    })
}

pub(super) fn arch_import_signature(name: &str) -> Option<(&'static [CLType], Option<CLType>)> {
    Some(match name {
        "lpp_arch_port_in8" => (&[cltypes::I16], Some(cltypes::I8)),
        "lpp_arch_port_in16" => (&[cltypes::I16], Some(cltypes::I16)),
        "lpp_arch_port_in32" => (&[cltypes::I16], Some(cltypes::I32)),
        "lpp_arch_port_out8" => (&[cltypes::I16, cltypes::I8], None),
        "lpp_arch_port_out16" => (&[cltypes::I16, cltypes::I16], None),
        "lpp_arch_port_out32" => (&[cltypes::I16, cltypes::I32], None),
        "lpp_arch_cpu_cli" | "lpp_arch_cpu_sti" | "lpp_arch_cpu_hlt" => (&[], None),
        "lpp_arch_rdtsc" | "lpp_arch_read_cr0" | "lpp_arch_read_cr2" | "lpp_arch_read_cr3"
        | "lpp_arch_read_cr4" => (&[], Some(cltypes::I64)),
        "lpp_arch_write_cr3" | "lpp_arch_write_cr4" | "lpp_arch_lgdt" | "lpp_arch_lidt"
        | "lpp_arch_invlpg" => (&[cltypes::I64], None),
        "lpp_arch_rdmsr" => (&[cltypes::I32], Some(cltypes::I64)),
        "lpp_arch_wrmsr" => (&[cltypes::I32, cltypes::I64], None),
        "lpp_set_new" | "lpp_deque_new" => (&[], Some(cltypes::I64)),
        "lpp_set_add" | "lpp_set_has" | "lpp_set_remove" | "lpp_set_add_str"
        | "lpp_set_has_str" | "lpp_set_remove_str" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::I8))
        }
        "lpp_set_len"
        | "lpp_deque_pop_back"
        | "lpp_deque_pop_front"
        | "lpp_deque_peek_front"
        | "lpp_deque_peek_back"
        | "lpp_deque_len" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_set_clear" | "lpp_set_free" | "lpp_deque_clear" | "lpp_deque_free" => {
            (&[cltypes::I64], None)
        }
        "lpp_set_union" | "lpp_set_intersect" | "lpp_deque_get" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::I64))
        }
        "lpp_deque_push_back" | "lpp_deque_push_front" => (&[cltypes::I64, cltypes::I64], None),
        _ => return None,
    })
}

fn is_signed_ty(kind: TypeKind) -> bool {
    matches!(
        kind,
        TypeKind::Primitive(
            PrimitiveType::Int
                | PrimitiveType::I8
                | PrimitiveType::I16
                | PrimitiveType::I32
                | PrimitiveType::ISize
        )
    )
}

impl<'m> Lowering<'m> {
    pub(super) fn lower_cast(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        operand: &Operand,
        target_ty: TypeId,
    ) -> Result<Value, CodegenError> {
        let src = self.operand_value(inst, builder, operand)?;
        let dst_cl = machine_type(inst.types, target_ty)
            .ok_or_else(|| unsupported(inst.mir_id, "unsupported cast target"))?;
        let src_cl = builder.func.dfg.value_type(src);
        if src_cl == dst_cl {
            return Ok(src);
        }
        let src_kind = operand_language_kind(inst.program, inst.types, operand);
        let dst_kind = inst.types.kind(target_ty);
        let src_signed = src_kind.is_some_and(is_signed_ty);

        let value = match (src_cl, dst_cl) {
            (cltypes::F64, t) if t.is_int() => {
                if is_signed_ty(dst_kind) {
                    builder.ins().fcvt_to_sint(t, src)
                } else {
                    builder.ins().fcvt_to_uint(t, src)
                }
            }
            (s, cltypes::F64) if s.is_int() => {
                if src_signed {
                    builder.ins().fcvt_from_sint(cltypes::F64, src)
                } else {
                    builder.ins().fcvt_from_uint(cltypes::F64, src)
                }
            }
            _ if matches!(dst_kind, TypeKind::Primitive(PrimitiveType::Bool))
                && src_cl.is_int() =>
            {
                let zero = builder.ins().iconst(src_cl, 0);
                let ne = builder.ins().icmp(IntCC::NotEqual, src, zero);
                let one = builder.ins().iconst(cltypes::I8, 1);
                let zero_b = builder.ins().iconst(cltypes::I8, 0);
                builder.ins().select(ne, one, zero_b)
            }
            (s, d) if s.is_int() && d.is_int() => {
                if s.bits() > d.bits() {
                    builder.ins().ireduce(d, src)
                } else if src_signed {
                    builder.ins().sextend(d, src)
                } else {
                    builder.ins().uextend(d, src)
                }
            }
            _ => return Err(unsupported(inst.mir_id, "unsupported cast combination")),
        };
        Ok(value)
    }

    pub(super) fn lower_kernel_intrinsic(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        intrinsic: KernelIntrinsic,
        type_argument: Option<TypeId>,
        arguments: &[Operand],
        target_ty: TypeId,
    ) -> Result<Option<Value>, CodegenError> {
        let mut args = Vec::with_capacity(arguments.len());
        for op in arguments {
            args.push(self.operand_value(inst, builder, op)?);
        }
        if let Some(import_name) = intrinsic_arch_import(intrinsic) {
            let func_ref = self.import_ref(builder, import_name);
            let call = builder.ins().call(func_ref, &args);
            return Ok(
                machine_type(inst.types, target_ty).map(|_| builder.func.dfg.first_result(call))
            );
        }

        let result = match intrinsic {
            KernelIntrinsic::SizeOf | KernelIntrinsic::AlignOf => {
                let ty = type_argument.unwrap_or(target_ty);
                let (sz, al) = static_type_size_align(inst.program, inst.types, ty);
                let val = if intrinsic == KernelIntrinsic::SizeOf {
                    i64::from(sz)
                } else {
                    i64::from(al)
                };
                Some(builder.ins().iconst(cltypes::I64, val))
            }
            KernelIntrinsic::PtrNull => Some(builder.ins().iconst(cltypes::I64, 0)),
            KernelIntrinsic::PtrIsNull => {
                let ptr = coerce_to(builder, args[0], cltypes::I64);
                let zero = builder.ins().iconst(cltypes::I64, 0);
                let eq = builder.ins().icmp(IntCC::Equal, ptr, zero);
                let one = builder.ins().iconst(cltypes::I8, 1);
                let zero_b = builder.ins().iconst(cltypes::I8, 0);
                Some(builder.ins().select(eq, one, zero_b))
            }
            KernelIntrinsic::PtrAdd => {
                let ptr = coerce_to(builder, args[0], cltypes::I64);
                let off = coerce_to(builder, args[1], cltypes::I64);
                Some(builder.ins().iadd(ptr, off))
            }
            KernelIntrinsic::PtrOffset => {
                let ptr = coerce_to(builder, args[0], cltypes::I64);
                let idx = coerce_to(builder, args[1], cltypes::I64);
                let elem_size = type_argument.map_or(1_i64, |ty| {
                    i64::from(
                        static_type_size_align(inst.program, inst.types, ty)
                            .0
                            .max(1),
                    )
                });
                let scaled = if elem_size == 1 {
                    idx
                } else {
                    builder.ins().imul_imm(idx, elem_size)
                };
                Some(builder.ins().iadd(ptr, scaled))
            }
            KernelIntrinsic::PtrRead => {
                let ptr = coerce_to(builder, args[0], cltypes::I64);
                let cl_ty = check_value_type(inst.types, target_ty, inst.mir_id)?;
                Some(builder.ins().load(cl_ty, MemFlags::trusted(), ptr, 0))
            }
            KernelIntrinsic::PtrWrite => {
                let ptr = coerce_to(builder, args[0], cltypes::I64);
                builder.ins().store(MemFlags::trusted(), args[1], ptr, 0);
                None
            }
            KernelIntrinsic::MmioRead8 => {
                let addr = coerce_to(builder, args[0], cltypes::I64);
                Some(
                    builder
                        .ins()
                        .load(cltypes::I8, MemFlags::trusted(), addr, 0),
                )
            }
            KernelIntrinsic::MmioRead16 => {
                let addr = coerce_to(builder, args[0], cltypes::I64);
                Some(
                    builder
                        .ins()
                        .load(cltypes::I16, MemFlags::trusted(), addr, 0),
                )
            }
            KernelIntrinsic::MmioRead32 => {
                let addr = coerce_to(builder, args[0], cltypes::I64);
                Some(
                    builder
                        .ins()
                        .load(cltypes::I32, MemFlags::trusted(), addr, 0),
                )
            }
            KernelIntrinsic::MmioRead64 => {
                let addr = coerce_to(builder, args[0], cltypes::I64);
                Some(
                    builder
                        .ins()
                        .load(cltypes::I64, MemFlags::trusted(), addr, 0),
                )
            }
            KernelIntrinsic::MmioWrite8 => {
                let addr = coerce_to(builder, args[0], cltypes::I64);
                let val = coerce_to(builder, args[1], cltypes::I8);
                builder.ins().store(MemFlags::trusted(), val, addr, 0);
                None
            }
            KernelIntrinsic::MmioWrite16 => {
                let addr = coerce_to(builder, args[0], cltypes::I64);
                let val = coerce_to(builder, args[1], cltypes::I16);
                builder.ins().store(MemFlags::trusted(), val, addr, 0);
                None
            }
            KernelIntrinsic::MmioWrite32 => {
                let addr = coerce_to(builder, args[0], cltypes::I64);
                let val = coerce_to(builder, args[1], cltypes::I32);
                builder.ins().store(MemFlags::trusted(), val, addr, 0);
                None
            }
            KernelIntrinsic::MmioWrite64 => {
                let addr = coerce_to(builder, args[0], cltypes::I64);
                let val = coerce_to(builder, args[1], cltypes::I64);
                builder.ins().store(MemFlags::trusted(), val, addr, 0);
                None
            }
            KernelIntrinsic::MemoryBarrier => {
                builder.ins().fence();
                None
            }
            KernelIntrinsic::MemcpyRaw => {
                let dst = coerce_to(builder, args[0], cltypes::I64);
                let src = coerce_to(builder, args[1], cltypes::I64);
                let len = coerce_to(builder, args[2], cltypes::I64);
                emit_memcpy_forward(builder, dst, src, len);
                Some(dst)
            }
            KernelIntrinsic::MemmoveRaw => {
                let dst = coerce_to(builder, args[0], cltypes::I64);
                let src = coerce_to(builder, args[1], cltypes::I64);
                let len = coerce_to(builder, args[2], cltypes::I64);
                emit_memmove_loop(builder, dst, src, len);
                Some(dst)
            }
            KernelIntrinsic::MemsetRaw => {
                let dst = coerce_to(builder, args[0], cltypes::I64);
                let byte = coerce_to(builder, args[1], cltypes::I8);
                let len = coerce_to(builder, args[2], cltypes::I64);
                emit_memset_loop(builder, dst, byte, len);
                Some(dst)
            }
            KernelIntrinsic::MemcmpRaw => {
                let a = coerce_to(builder, args[0], cltypes::I64);
                let b = coerce_to(builder, args[1], cltypes::I64);
                let len = coerce_to(builder, args[2], cltypes::I64);
                Some(emit_memcmp_loop(builder, a, b, len))
            }
            _ => unreachable!("all remaining intrinsics handled via arch imports"),
        };
        Ok(result)
    }
}

fn emit_memcpy_forward(builder: &mut FunctionBuilder, dst: Value, src: Value, len: Value) {
    let hdr = builder.create_block();
    let body = builder.create_block();
    let done = builder.create_block();
    let idx = builder.append_block_param(hdr, cltypes::I64);
    let zero = builder.ins().iconst(cltypes::I64, 0);
    builder.ins().jump(hdr, &[zero]);

    builder.switch_to_block(hdr);
    let cond = builder.ins().icmp(IntCC::UnsignedLessThan, idx, len);
    builder.ins().brif(cond, body, &[], done, &[]);

    builder.seal_block(body);
    builder.switch_to_block(body);
    let s_ptr = builder.ins().iadd(src, idx);
    let d_ptr = builder.ins().iadd(dst, idx);
    let byte = builder
        .ins()
        .load(cltypes::I8, MemFlags::trusted(), s_ptr, 0);
    builder.ins().store(MemFlags::trusted(), byte, d_ptr, 0);
    let next = builder.ins().iadd_imm(idx, 1);
    builder.ins().jump(hdr, &[next]);

    builder.seal_block(hdr);
    builder.seal_block(done);
    builder.switch_to_block(done);
}

fn emit_memmove_loop(builder: &mut FunctionBuilder, dst: Value, src: Value, len: Value) {
    let fwd = builder.create_block();
    let bwd_hdr = builder.create_block();
    let bwd_body = builder.create_block();
    let done = builder.create_block();
    let idx = builder.append_block_param(bwd_hdr, cltypes::I64);

    let go_fwd = builder.ins().icmp(IntCC::UnsignedLessThanOrEqual, dst, src);
    builder.ins().brif(go_fwd, fwd, &[], bwd_hdr, &[len]);

    builder.seal_block(fwd);
    builder.switch_to_block(fwd);
    emit_memcpy_forward(builder, dst, src, len);
    builder.ins().jump(done, &[]);

    builder.switch_to_block(bwd_hdr);
    let zero = builder.ins().iconst(cltypes::I64, 0);
    let cond = builder.ins().icmp(IntCC::UnsignedGreaterThan, idx, zero);
    builder.ins().brif(cond, bwd_body, &[], done, &[]);

    builder.seal_block(bwd_body);
    builder.switch_to_block(bwd_body);
    let prev = builder.ins().iadd_imm(idx, -1);
    let s_ptr = builder.ins().iadd(src, prev);
    let d_ptr = builder.ins().iadd(dst, prev);
    let byte = builder
        .ins()
        .load(cltypes::I8, MemFlags::trusted(), s_ptr, 0);
    builder.ins().store(MemFlags::trusted(), byte, d_ptr, 0);
    builder.ins().jump(bwd_hdr, &[prev]);

    builder.seal_block(bwd_hdr);
    builder.seal_block(done);
    builder.switch_to_block(done);
}

fn emit_memset_loop(builder: &mut FunctionBuilder, dst: Value, byte: Value, len: Value) {
    let hdr = builder.create_block();
    let body = builder.create_block();
    let done = builder.create_block();
    let idx = builder.append_block_param(hdr, cltypes::I64);
    let zero = builder.ins().iconst(cltypes::I64, 0);
    builder.ins().jump(hdr, &[zero]);

    builder.switch_to_block(hdr);
    let cond = builder.ins().icmp(IntCC::UnsignedLessThan, idx, len);
    builder.ins().brif(cond, body, &[], done, &[]);

    builder.seal_block(body);
    builder.switch_to_block(body);
    let d_ptr = builder.ins().iadd(dst, idx);
    builder.ins().store(MemFlags::trusted(), byte, d_ptr, 0);
    let next = builder.ins().iadd_imm(idx, 1);
    builder.ins().jump(hdr, &[next]);

    builder.seal_block(hdr);
    builder.seal_block(done);
    builder.switch_to_block(done);
}

fn emit_memcmp_loop(builder: &mut FunctionBuilder, a: Value, b: Value, len: Value) -> Value {
    let hdr = builder.create_block();
    let body = builder.create_block();
    let mismatch = builder.create_block();
    let eq_done = builder.create_block();
    let merge = builder.create_block();
    let idx = builder.append_block_param(hdr, cltypes::I64);
    let res = builder.append_block_param(merge, cltypes::I32);

    let zero = builder.ins().iconst(cltypes::I64, 0);
    builder.ins().jump(hdr, &[zero]);

    builder.switch_to_block(hdr);
    let cond = builder.ins().icmp(IntCC::UnsignedLessThan, idx, len);
    builder.ins().brif(cond, body, &[], eq_done, &[]);

    builder.seal_block(body);
    builder.switch_to_block(body);
    let ba = builder.ins().load(cltypes::I8, MemFlags::trusted(), a, 0);
    let a_ptr = builder.ins().iadd(a, idx);
    let b_ptr = builder.ins().iadd(b, idx);
    let _ = ba;
    let va = builder
        .ins()
        .load(cltypes::I8, MemFlags::trusted(), a_ptr, 0);
    let vb = builder
        .ins()
        .load(cltypes::I8, MemFlags::trusted(), b_ptr, 0);
    let eq = builder.ins().icmp(IntCC::Equal, va, vb);
    let next_block = builder.create_block();
    builder.ins().brif(eq, next_block, &[], mismatch, &[]);

    builder.seal_block(next_block);
    builder.switch_to_block(next_block);
    let next = builder.ins().iadd_imm(idx, 1);
    builder.ins().jump(hdr, &[next]);

    builder.seal_block(mismatch);
    builder.switch_to_block(mismatch);
    let ua = builder.ins().uextend(cltypes::I32, va);
    let ub = builder.ins().uextend(cltypes::I32, vb);
    let diff = builder.ins().isub(ua, ub);
    builder.ins().jump(merge, &[diff]);

    builder.seal_block(eq_done);
    builder.switch_to_block(eq_done);
    let zero_r = builder.ins().iconst(cltypes::I32, 0);
    builder.ins().jump(merge, &[zero_r]);

    builder.seal_block(hdr);
    builder.seal_block(merge);
    builder.switch_to_block(merge);
    res
}
