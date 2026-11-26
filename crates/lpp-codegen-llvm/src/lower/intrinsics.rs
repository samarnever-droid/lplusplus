use lpp_codegen_api::CodegenError;
use lpp_codegen_api::layout::static_type_size_align;
use lpp_mir::{Constant, KernelIntrinsic, Operand};
use lpp_runtime_abi::generated::SemanticAbiType;
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeKind};

use super::function::FnCtx;
use super::plan::{builtin_descriptor_sig, intrinsic_arch_import, operand_kind};
use super::{
    ElementClass, IMP_LIST_GET, IMP_LIST_GET_ARC, IMP_LIST_GET_BOOL, IMP_LIST_GET_FLOAT,
    IMP_LIST_LEN, IMP_LIST_NEW, IMP_LIST_NEW_ARC, IMP_LIST_PUSH, IMP_LIST_PUSH_ARC,
    IMP_LIST_PUSH_BOOL, IMP_LIST_PUSH_FLOAT, IMP_LIST_SET, IMP_LIST_SET_ARC, IMP_LIST_SET_BOOL,
    IMP_LIST_SET_FLOAT, IMP_LIST_TRACE, IMP_PRINT_BOOL, IMP_PRINT_FLOAT, IMP_PRINT_INT,
    IMP_PRINT_STR, IMP_VEC_CHECKSUM, Val, element_class, is_managed, is_signed_integer_kind,
    unsupported, val_of_type,
};

impl<'a> FnCtx<'a> {
    pub(crate) fn lower_cast(
        &mut self,
        operand: &Operand,
        target_ty: TypeId,
    ) -> Result<(Val, String), CodegenError> {
        let (src_val, src_name) = self.operand_value(operand)?;
        let dst_val = val_of_type(self.types, target_ty, self.mir_id)?;
        let src_kind = operand_kind(self.program, self.types, operand);
        let dst_kind = self.types.kind(target_ty);
        let src_signed = is_signed_integer_kind(src_kind);
        let dst_signed = is_signed_integer_kind(Some(dst_kind));

        if src_val == dst_val {
            return Ok((dst_val, src_name));
        }

        let res = match (src_val, dst_val) {
            (Val::F64, t) if t.is_int() => {
                let r = self.fresh();
                let op = if dst_signed { "fptosi" } else { "fptoui" };
                self.line(format!("{r} = {op} double {src_name} to {}", t.llvm()));
                r
            }
            (s, Val::F64) if s.is_int() => {
                let r = self.fresh();
                let op = if src_signed { "sitofp" } else { "uitofp" };
                self.line(format!("{r} = {op} {} {src_name} to double", s.llvm()));
                r
            }
            _ if matches!(dst_kind, TypeKind::Primitive(PrimitiveType::Bool))
                && (src_val.is_int() || src_val == Val::Ptr) =>
            {
                let cmp = self.fresh();
                if src_val == Val::Ptr {
                    self.line(format!("{cmp} = icmp ne ptr {src_name}, null"));
                } else {
                    self.line(format!("{cmp} = icmp ne {} {src_name}, 0", src_val.llvm()));
                }
                let r = self.fresh();
                self.line(format!("{r} = zext i1 {cmp} to i8"));
                r
            }
            _ => self.coerce(&src_name, src_val, dst_val, src_signed),
        };
        Ok((dst_val, res))
    }

    pub(crate) fn lower_kernel_intrinsic(
        &mut self,
        intrinsic: KernelIntrinsic,
        type_argument: Option<TypeId>,
        arguments: &[Operand],
        target_ty: TypeId,
    ) -> Result<Option<(Val, String)>, CodegenError> {
        let mut arg_vals = Vec::with_capacity(arguments.len());
        for op in arguments {
            arg_vals.push(self.operand_value(op)?);
        }

        if let Some(sym) = intrinsic_arch_import(intrinsic) {
            let sig = &self.plan.imports[sym];
            let coerced: Vec<(Val, String)> = arg_vals
                .iter()
                .zip(sig.params.iter())
                .map(|((v, n), target)| (*target, self.coerce(n, *v, *target, false)))
                .collect();
            return Ok(self.emit_call_direct(sym, &coerced, sig.result));
        }

        let res = match intrinsic {
            KernelIntrinsic::SizeOf | KernelIntrinsic::AlignOf => {
                let ty = type_argument.unwrap_or(target_ty);
                let (sz, al) = static_type_size_align(self.program, self.types, ty);
                let v = if intrinsic == KernelIntrinsic::SizeOf {
                    sz
                } else {
                    al
                };
                Some((Val::I64, v.to_string()))
            }
            KernelIntrinsic::PtrNull => Some((Val::Ptr, "null".to_string())),
            KernelIntrinsic::PtrIsNull => {
                let ptr = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let cmp = self.fresh();
                self.line(format!("{cmp} = icmp eq ptr {ptr}, null"));
                let r = self.fresh();
                self.line(format!("{r} = zext i1 {cmp} to i8"));
                Some((Val::I8, r))
            }
            KernelIntrinsic::PtrAdd => {
                let ptr = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let off = self.coerce(&arg_vals[1].1, arg_vals[1].0, Val::I64, true);
                let r = self.fresh();
                self.line(format!("{r} = getelementptr i8, ptr {ptr}, i64 {off}"));
                Some((Val::Ptr, r))
            }
            KernelIntrinsic::PtrOffset => {
                let ptr = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let idx = self.coerce(&arg_vals[1].1, arg_vals[1].0, Val::I64, true);
                let elem_size = type_argument.map_or(1_u32, |ty| {
                    static_type_size_align(self.program, self.types, ty)
                        .0
                        .max(1)
                });
                let byte_off = if elem_size == 1 {
                    idx
                } else {
                    let s = self.fresh();
                    self.line(format!("{s} = mul i64 {idx}, {elem_size}"));
                    s
                };
                let r = self.fresh();
                self.line(format!("{r} = getelementptr i8, ptr {ptr}, i64 {byte_off}"));
                Some((Val::Ptr, r))
            }
            KernelIntrinsic::PtrRead => {
                let ptr = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let val_ty = val_of_type(self.types, target_ty, self.mir_id)?;
                let r = self.fresh();
                self.line(format!("{r} = load {}, ptr {ptr}, align 1", val_ty.llvm()));
                Some((val_ty, r))
            }
            KernelIntrinsic::PtrWrite => {
                let ptr = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let (vt, vn) = &arg_vals[1];
                self.line(format!("store {} {vn}, ptr {ptr}, align 1", vt.llvm()));
                None
            }
            KernelIntrinsic::MmioRead8 => Some(self.emit_mmio_read(&arg_vals[0], Val::I8, 1)),
            KernelIntrinsic::MmioRead16 => Some(self.emit_mmio_read(&arg_vals[0], Val::I16, 2)),
            KernelIntrinsic::MmioRead32 => Some(self.emit_mmio_read(&arg_vals[0], Val::I32, 4)),
            KernelIntrinsic::MmioRead64 => Some(self.emit_mmio_read(&arg_vals[0], Val::I64, 8)),
            KernelIntrinsic::MmioWrite8 => {
                self.emit_mmio_write(&arg_vals[0], &arg_vals[1], Val::I8, 1);
                None
            }
            KernelIntrinsic::MmioWrite16 => {
                self.emit_mmio_write(&arg_vals[0], &arg_vals[1], Val::I16, 2);
                None
            }
            KernelIntrinsic::MmioWrite32 => {
                self.emit_mmio_write(&arg_vals[0], &arg_vals[1], Val::I32, 4);
                None
            }
            KernelIntrinsic::MmioWrite64 => {
                self.emit_mmio_write(&arg_vals[0], &arg_vals[1], Val::I64, 8);
                None
            }
            KernelIntrinsic::MemoryBarrier => {
                self.line("fence seq_cst");
                None
            }
            KernelIntrinsic::MemcpyRaw => {
                let dst = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let src = self.coerce(&arg_vals[1].1, arg_vals[1].0, Val::Ptr, false);
                let len = self.coerce(&arg_vals[2].1, arg_vals[2].0, Val::I64, false);
                self.emit_raw_memcpy(&dst, &src, &len);
                Some((Val::Ptr, dst))
            }
            KernelIntrinsic::MemmoveRaw => {
                let dst = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let src = self.coerce(&arg_vals[1].1, arg_vals[1].0, Val::Ptr, false);
                let len = self.coerce(&arg_vals[2].1, arg_vals[2].0, Val::I64, false);
                self.emit_raw_memmove(&dst, &src, &len);
                Some((Val::Ptr, dst))
            }
            KernelIntrinsic::MemsetRaw => {
                let dst = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let byte = self.coerce(&arg_vals[1].1, arg_vals[1].0, Val::I8, false);
                let len = self.coerce(&arg_vals[2].1, arg_vals[2].0, Val::I64, false);
                self.emit_raw_memset(&dst, &byte, &len);
                Some((Val::Ptr, dst))
            }
            KernelIntrinsic::MemcmpRaw => {
                let a = self.coerce(&arg_vals[0].1, arg_vals[0].0, Val::Ptr, false);
                let b = self.coerce(&arg_vals[1].1, arg_vals[1].0, Val::Ptr, false);
                let len = self.coerce(&arg_vals[2].1, arg_vals[2].0, Val::I64, false);
                let res = self.emit_raw_memcmp(&a, &b, &len);
                Some((Val::I64, res))
            }
            _ => unreachable!("handled by arch import table"),
        };
        Ok(res)
    }

    fn emit_mmio_read(&mut self, addr: &(Val, String), width: Val, align: u32) -> (Val, String) {
        let p = self.coerce(&addr.1, addr.0, Val::Ptr, false);
        let r = self.fresh();
        self.line(format!(
            "{r} = load volatile {}, ptr {p}, align {align}",
            width.llvm()
        ));
        (width, r)
    }

    fn emit_mmio_write(
        &mut self,
        addr: &(Val, String),
        val: &(Val, String),
        width: Val,
        align: u32,
    ) {
        let p = self.coerce(&addr.1, addr.0, Val::Ptr, false);
        let v = self.coerce(&val.1, val.0, width, false);
        self.line(format!(
            "store volatile {} {v}, ptr {p}, align {align}",
            width.llvm()
        ));
    }

    fn emit_raw_memcpy(&mut self, dst: &str, src: &str, len: &str) {
        let entry_lbl = self.cur_block_label.clone();
        let hdr = self.fresh_label("mcpy_hdr");
        let body = self.fresh_label("mcpy_body");
        let done = self.fresh_label("mcpy_done");
        self.line(format!("br label %{hdr}"));

        self.start_label(&hdr);
        let idx = self.fresh();
        let next = self.fresh();
        self.line(format!(
            "{idx} = phi i64 [ 0, %{entry_lbl} ], [ {next}, %{body} ]"
        ));
        let cond = self.fresh();
        self.line(format!("{cond} = icmp ult i64 {idx}, {len}"));
        self.line(format!("br i1 {cond}, label %{body}, label %{done}"));

        self.start_label(&body);
        let sp = self.fresh();
        let dp = self.fresh();
        let bv = self.fresh();
        self.line(format!("{sp} = getelementptr i8, ptr {src}, i64 {idx}"));
        self.line(format!("{bv} = load i8, ptr {sp}, align 1"));
        self.line(format!("{dp} = getelementptr i8, ptr {dst}, i64 {idx}"));
        self.line(format!("store i8 {bv}, ptr {dp}, align 1"));
        self.line(format!("{next} = add i64 {idx}, 1"));
        self.line(format!("br label %{hdr}"));

        self.start_label(&done);
    }

    fn emit_raw_memmove(&mut self, dst: &str, src: &str, len: &str) {
        let di = self.fresh();
        let si = self.fresh();
        self.line(format!("{di} = ptrtoint ptr {dst} to i64"));
        self.line(format!("{si} = ptrtoint ptr {src} to i64"));
        let overlap = self.fresh();
        self.line(format!("{overlap} = icmp ugt i64 {di}, {si}"));
        let bwd_hdr = self.fresh_label("mmov_bwd_hdr");
        let bwd_body = self.fresh_label("mmov_bwd_body");
        let fwd_hdr = self.fresh_label("mmov_fwd_hdr");
        let fwd_body = self.fresh_label("mmov_fwd_body");
        let done = self.fresh_label("mmov_done");
        let entry_lbl = self.cur_block_label.clone();
        self.line(format!(
            "br i1 {overlap}, label %{bwd_hdr}, label %{fwd_hdr}"
        ));

        self.start_label(&bwd_hdr);
        let rem = self.fresh();
        let dec = self.fresh();
        self.line(format!(
            "{rem} = phi i64 [ {len}, %{entry_lbl} ], [ {dec}, %{bwd_body} ]"
        ));
        let has_more = self.fresh();
        self.line(format!("{has_more} = icmp ne i64 {rem}, 0"));
        self.line(format!(
            "br i1 {has_more}, label %{bwd_body}, label %{done}"
        ));
        self.start_label(&bwd_body);
        self.line(format!("{dec} = sub i64 {rem}, 1"));
        let sp = self.fresh();
        let dp = self.fresh();
        let bv = self.fresh();
        self.line(format!("{sp} = getelementptr i8, ptr {src}, i64 {dec}"));
        self.line(format!("{bv} = load i8, ptr {sp}, align 1"));
        self.line(format!("{dp} = getelementptr i8, ptr {dst}, i64 {dec}"));
        self.line(format!("store i8 {bv}, ptr {dp}, align 1"));
        self.line(format!("br label %{bwd_hdr}"));

        self.start_label(&fwd_hdr);
        let idx = self.fresh();
        let next = self.fresh();
        self.line(format!(
            "{idx} = phi i64 [ 0, %{entry_lbl} ], [ {next}, %{fwd_body} ]"
        ));
        let cond = self.fresh();
        self.line(format!("{cond} = icmp ult i64 {idx}, {len}"));
        self.line(format!("br i1 {cond}, label %{fwd_body}, label %{done}"));
        self.start_label(&fwd_body);
        let sp2 = self.fresh();
        let dp2 = self.fresh();
        let bv2 = self.fresh();
        self.line(format!("{sp2} = getelementptr i8, ptr {src}, i64 {idx}"));
        self.line(format!("{bv2} = load i8, ptr {sp2}, align 1"));
        self.line(format!("{dp2} = getelementptr i8, ptr {dst}, i64 {idx}"));
        self.line(format!("store i8 {bv2}, ptr {dp2}, align 1"));
        self.line(format!("{next} = add i64 {idx}, 1"));
        self.line(format!("br label %{fwd_hdr}"));

        self.start_label(&done);
    }

    fn emit_raw_memset(&mut self, dst: &str, byte: &str, len: &str) {
        let entry_lbl = self.cur_block_label.clone();
        let hdr = self.fresh_label("mset_hdr");
        let body = self.fresh_label("mset_body");
        let done = self.fresh_label("mset_done");
        self.line(format!("br label %{hdr}"));

        self.start_label(&hdr);
        let idx = self.fresh();
        let next = self.fresh();
        self.line(format!(
            "{idx} = phi i64 [ 0, %{entry_lbl} ], [ {next}, %{body} ]"
        ));
        let cond = self.fresh();
        self.line(format!("{cond} = icmp ult i64 {idx}, {len}"));
        self.line(format!("br i1 {cond}, label %{body}, label %{done}"));

        self.start_label(&body);
        let dp = self.fresh();
        self.line(format!("{dp} = getelementptr i8, ptr {dst}, i64 {idx}"));
        self.line(format!("store i8 {byte}, ptr {dp}, align 1"));
        self.line(format!("{next} = add i64 {idx}, 1"));
        self.line(format!("br label %{hdr}"));

        self.start_label(&done);
    }

    fn emit_raw_memcmp(&mut self, a: &str, b: &str, len: &str) -> String {
        let entry_lbl = self.cur_block_label.clone();
        let hdr = self.fresh_label("mcmp_hdr");
        let body = self.fresh_label("mcmp_body");
        let step = self.fresh_label("mcmp_step");
        let diff = self.fresh_label("mcmp_diff");
        let eq = self.fresh_label("mcmp_eq");
        let done = self.fresh_label("mcmp_done");
        self.line(format!("br label %{hdr}"));

        self.start_label(&hdr);
        let idx = self.fresh();
        let next = self.fresh();
        self.line(format!(
            "{idx} = phi i64 [ 0, %{entry_lbl} ], [ {next}, %{step} ]"
        ));
        let cond = self.fresh();
        self.line(format!("{cond} = icmp ult i64 {idx}, {len}"));
        self.line(format!("br i1 {cond}, label %{body}, label %{eq}"));

        self.start_label(&body);
        let ap = self.fresh();
        let bp = self.fresh();
        let av = self.fresh();
        let bv = self.fresh();
        let ax = self.fresh();
        let bx = self.fresh();
        let cmp = self.fresh();
        self.line(format!("{ap} = getelementptr i8, ptr {a}, i64 {idx}"));
        self.line(format!("{bp} = getelementptr i8, ptr {b}, i64 {idx}"));
        self.line(format!("{av} = load i8, ptr {ap}, align 1"));
        self.line(format!("{bv} = load i8, ptr {bp}, align 1"));
        self.line(format!("{ax} = zext i8 {av} to i64"));
        self.line(format!("{bx} = zext i8 {bv} to i64"));
        self.line(format!("{cmp} = icmp eq i64 {ax}, {bx}"));
        self.line(format!("br i1 {cmp}, label %{step}, label %{diff}"));

        self.start_label(&step);
        self.line(format!("{next} = add i64 {idx}, 1"));
        self.line(format!("br label %{hdr}"));

        self.start_label(&diff);
        let dval = self.fresh();
        self.line(format!("{dval} = sub i64 {ax}, {bx}"));
        self.line(format!("br label %{done}"));

        self.start_label(&eq);
        self.line(format!("br label %{done}"));

        self.start_label(&done);
        let out = self.fresh();
        self.line(format!("{out} = phi i64 [ {dval}, %{diff} ], [ 0, %{eq} ]"));
        out
    }

    pub(crate) fn lower_builtin(
        &mut self,
        builtin: BuiltinId,
        args: &[Operand],
        target_ty: TypeId,
    ) -> Result<Option<(Val, String)>, CodegenError> {
        let d = builtin.descriptor();
        let name = d.name;

        if name == "print_str" {
            let v = self.operand_value(&args[0])?;
            let p = self.coerce(&v.1, v.0, Val::Ptr, false);
            self.emit_call_direct(IMP_PRINT_STR, &[(Val::Ptr, p)], None);
            return Ok(None);
        }

        if name == "print" {
            let arg = &args[0];
            let kind = operand_kind(self.program, self.types, arg);
            let (ty, val) = self.operand_value(arg)?;
            match kind {
                Some(TypeKind::Primitive(PrimitiveType::Float)) => {
                    self.emit_call_direct(IMP_PRINT_FLOAT, &[(ty, val)], None);
                }
                Some(TypeKind::Primitive(PrimitiveType::Bool)) => {
                    let b = self.coerce(&val, ty, Val::I8, false);
                    self.emit_call_direct(IMP_PRINT_BOOL, &[(Val::I8, b)], None);
                }
                Some(TypeKind::Primitive(PrimitiveType::String)) => {
                    let p = self.coerce(&val, ty, Val::Ptr, false);
                    self.emit_call_direct(IMP_PRINT_STR, &[(Val::Ptr, p)], None);
                }
                _ => {
                    let signed = is_signed_integer_kind(kind);
                    let w = self.coerce(&val, ty, Val::I64, signed);
                    self.emit_call_direct(IMP_PRINT_INT, &[(Val::I64, w)], None);
                }
            }
            self.empty_moved_local(arg);
            return Ok(None);
        }

        if name == "list_new" {
            let TypeKind::List(elem) = self.types.kind(target_ty) else {
                return Err(unsupported(self.mir_id, "list_new of non-list"));
            };
            let sym = if is_managed(self.types, elem) {
                IMP_LIST_NEW_ARC
            } else {
                IMP_LIST_NEW
            };
            let list = self.emit_call_direct(sym, &[], Some(Val::Ptr)).unwrap();
            if self.plan.shared_types.contains(&target_ty) {
                self.mark_collectible(&list.1, IMP_LIST_TRACE);
            }
            return Ok(Some(list));
        }

        if name == "list_push" || name == "list_set" {
            return self.lower_list_mutation(name, args);
        }

        if name == "list_get" || name == "get" {
            return self.lower_list_get(args);
        }

        if name == "list_len" || name == "len" {
            let list = self.operand_value(&args[0])?;
            let p = self.coerce(&list.1, list.0, Val::Ptr, false);
            return Ok(self.emit_call_direct(IMP_LIST_LEN, &[(Val::Ptr, p)], Some(Val::I64)));
        }

        if name == "slice" || name == "str_slice" {
            return self.lower_slice_init(name, args);
        }

        if name == "slice_get"
            && args
                .first()
                .and_then(|op| operand_kind(self.program, self.types, op))
                == Some(TypeKind::Primitive(PrimitiveType::StrSlice))
        {
            let mut call_args = Vec::with_capacity(args.len());
            for (op, expected) in args.iter().zip([Val::Ptr, Val::I64]) {
                let (v, n) = self.operand_value(op)?;
                call_args.push((expected, self.coerce(&n, v, expected, true)));
            }
            return Ok(self.emit_call_direct("lpp_str_slice_get", &call_args, Some(Val::Ptr)));
        }

        if name.starts_with("vec_") {
            return self.lower_simd(builtin, name, args);
        }

        if !d.symbol.is_empty() {
            let sig = builtin_descriptor_sig(builtin);
            let mut call_args = Vec::with_capacity(args.len());
            for (op, expected) in args.iter().zip(sig.params.iter()) {
                let (v, n) = self.operand_value(op)?;
                let signed = is_signed_integer_kind(operand_kind(self.program, self.types, op));
                call_args.push((*expected, self.coerce(&n, v, *expected, signed)));
            }
            let raw = self.emit_call_direct(d.symbol, &call_args, sig.result);
            if name == "write_str" {
                self.empty_moved_local(&args[0]);
            }
            return Ok(match raw {
                Some((r_ty, r_name)) if matches!(d.semantic_result, SemanticAbiType::Bool) => {
                    let b = self.coerce(&r_name, r_ty, Val::I8, false);
                    Some((Val::I8, b))
                }
                other => other,
            });
        }

        Err(unsupported(self.mir_id, "unsupported builtin"))
    }

    fn lower_list_mutation(
        &mut self,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<(Val, String)>, CodegenError> {
        let list_op = self.operand_value(&args[0])?;
        let list = self.coerce(&list_op.1, list_op.0, Val::Ptr, false);
        let TypeKind::List(elem) = self.types.kind(
            self.program
                .local(match &args[0] {
                    Operand::Copy(l) => *l,
                    _ => return Err(unsupported(self.mir_id, "list mutation non-local")),
                })
                .expect("local")
                .ty,
        ) else {
            return Err(unsupported(self.mir_id, "list mutation non-list"));
        };
        let is_push = name == "list_push";
        let val_op = if is_push { &args[1] } else { &args[2] };
        let (v_ty, v_name) = self.operand_value(val_op)?;

        match element_class(self.types, elem) {
            ElementClass::Managed => {
                let vp = self.coerce(&v_name, v_ty, Val::Ptr, false);
                if is_push {
                    self.emit_call_direct(
                        IMP_LIST_PUSH_ARC,
                        &[(Val::Ptr, list), (Val::Ptr, vp.clone())],
                        None,
                    );
                } else {
                    let (it, iv) = self.operand_value(&args[1])?;
                    let idx = self.coerce(&iv, it, Val::I64, true);
                    self.emit_call_direct(
                        IMP_LIST_SET_ARC,
                        &[(Val::Ptr, list), (Val::I64, idx), (Val::Ptr, vp.clone())],
                        None,
                    );
                }
                self.arc_release(&vp);
                self.empty_moved_local(val_op);
            }
            ElementClass::Scalar => {
                let signed = is_signed_integer_kind(Some(self.types.kind(elem)));
                let vs = self.coerce(&v_name, v_ty, Val::I64, signed);
                if is_push {
                    self.emit_call_direct(IMP_LIST_PUSH, &[(Val::Ptr, list), (Val::I64, vs)], None);
                } else {
                    let (it, iv) = self.operand_value(&args[1])?;
                    let idx = self.coerce(&iv, it, Val::I64, true);
                    self.emit_call_direct(
                        IMP_LIST_SET,
                        &[(Val::Ptr, list), (Val::I64, idx), (Val::I64, vs)],
                        None,
                    );
                }
            }
            ElementClass::Float => {
                let vf = self.coerce(&v_name, v_ty, Val::F64, false);
                if is_push {
                    self.emit_call_direct(
                        IMP_LIST_PUSH_FLOAT,
                        &[(Val::Ptr, list), (Val::F64, vf)],
                        None,
                    );
                } else {
                    let (it, iv) = self.operand_value(&args[1])?;
                    let idx = self.coerce(&iv, it, Val::I64, true);
                    self.emit_call_direct(
                        IMP_LIST_SET_FLOAT,
                        &[(Val::Ptr, list), (Val::I64, idx), (Val::F64, vf)],
                        None,
                    );
                }
            }
            ElementClass::Bool => {
                let vb = self.coerce(&v_name, v_ty, Val::I8, false);
                if is_push {
                    self.emit_call_direct(
                        IMP_LIST_PUSH_BOOL,
                        &[(Val::Ptr, list), (Val::I8, vb)],
                        None,
                    );
                } else {
                    let (it, iv) = self.operand_value(&args[1])?;
                    let idx = self.coerce(&iv, it, Val::I64, true);
                    self.emit_call_direct(
                        IMP_LIST_SET_BOOL,
                        &[(Val::Ptr, list), (Val::I64, idx), (Val::I8, vb)],
                        None,
                    );
                }
            }
        }
        Ok(None)
    }

    fn lower_list_get(&mut self, args: &[Operand]) -> Result<Option<(Val, String)>, CodegenError> {
        let list_op = self.operand_value(&args[0])?;
        let list = self.coerce(&list_op.1, list_op.0, Val::Ptr, false);
        let (it, iv) = self.operand_value(&args[1])?;
        let idx = self.coerce(&iv, it, Val::I64, true);
        let TypeKind::List(elem) = self.types.kind(
            self.program
                .local(match &args[0] {
                    Operand::Copy(l) => *l,
                    _ => return Err(unsupported(self.mir_id, "list_get non-local")),
                })
                .expect("local")
                .ty,
        ) else {
            return Err(unsupported(self.mir_id, "list_get non-list"));
        };
        let out = match element_class(self.types, elem) {
            ElementClass::Managed => {
                let (_, v) = self
                    .emit_call_direct(
                        IMP_LIST_GET_ARC,
                        &[(Val::Ptr, list), (Val::I64, idx)],
                        Some(Val::Ptr),
                    )
                    .unwrap();
                self.arc_retain(&v);
                (Val::Ptr, v)
            }
            ElementClass::Scalar => {
                let (_, v) = self
                    .emit_call_direct(
                        IMP_LIST_GET,
                        &[(Val::Ptr, list), (Val::I64, idx)],
                        Some(Val::I64),
                    )
                    .unwrap();
                let target = val_of_type(self.types, elem, self.mir_id)?;
                let c = self.coerce(
                    &v,
                    Val::I64,
                    target,
                    is_signed_integer_kind(Some(self.types.kind(elem))),
                );
                (target, c)
            }
            ElementClass::Float => self
                .emit_call_direct(
                    IMP_LIST_GET_FLOAT,
                    &[(Val::Ptr, list), (Val::I64, idx)],
                    Some(Val::F64),
                )
                .unwrap(),
            ElementClass::Bool => self
                .emit_call_direct(
                    IMP_LIST_GET_BOOL,
                    &[(Val::Ptr, list), (Val::I64, idx)],
                    Some(Val::I8),
                )
                .unwrap(),
        };
        Ok(Some(out))
    }

    fn lower_slice_init(
        &mut self,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<(Val, String)>, CodegenError> {
        let base = self.operand_value(&args[0])?;
        let start = self.operand_value(&args[1])?;
        let length = self.operand_value(&args[2])?;
        let slot = self.entry_alloca("[40 x i8]", 8);
        let kind = if name == "str_slice" { "0" } else { "1" };
        let bp = self.coerce(&base.1, base.0, Val::Ptr, false);
        let st = self.coerce(&start.1, start.0, Val::I64, true);
        let ln = self.coerce(&length.1, length.0, Val::I64, true);
        Ok(self.emit_call_direct(
            "lpp_slice_init",
            &[
                (Val::Ptr, slot),
                (Val::Ptr, bp),
                (Val::I64, st),
                (Val::I64, ln),
                (Val::I64, kind.to_string()),
            ],
            Some(Val::Ptr),
        ))
    }

    fn lower_simd(
        &mut self,
        builtin: BuiltinId,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<(Val, String)>, CodegenError> {
        let vec_from_lanes = |this: &mut Self, a: &str, b: &str| -> String {
            let t1 = this.fresh();
            let t2 = this.fresh();
            this.line(format!(
                "{t1} = insertelement <2 x i64> zeroinitializer, i64 {a}, i32 0"
            ));
            this.line(format!(
                "{t2} = insertelement <2 x i64> {t1}, i64 {b}, i32 1"
            ));
            t2
        };

        let out = match name {
            "vec_i64x2" => {
                let l0 = self.operand_value(&args[0])?;
                let l1 = self.operand_value(&args[1])?;
                let a = self.coerce(&l0.1, l0.0, Val::I64, true);
                let b = self.coerce(&l1.1, l1.0, Val::I64, true);
                Some((Val::V2I64, vec_from_lanes(self, &a, &b)))
            }
            "vec_i64x2_splat" => {
                let l = self.operand_value(&args[0])?;
                let a = self.coerce(&l.1, l.0, Val::I64, true);
                Some((Val::V2I64, vec_from_lanes(self, &a, &a)))
            }
            "vec_i64x2_not" => {
                let (_, v) = self.operand_value(&args[0])?;
                let r = self.fresh();
                self.line(format!("{r} = xor <2 x i64> {v}, <i64 -1, i64 -1>"));
                Some((Val::V2I64, r))
            }
            "vec_i64x2_add" | "vec_i64x2_sub" | "vec_i64x2_mul" | "vec_i64x2_and"
            | "vec_i64x2_or" | "vec_i64x2_xor" => {
                let (_, l) = self.operand_value(&args[0])?;
                let (_, r) = self.operand_value(&args[1])?;
                let op = match name {
                    "vec_i64x2_add" => "add",
                    "vec_i64x2_sub" => "sub",
                    "vec_i64x2_mul" => "mul",
                    "vec_i64x2_and" => "and",
                    "vec_i64x2_or" => "or",
                    _ => "xor",
                };
                let dst = self.fresh();
                self.line(format!("{dst} = {op} <2 x i64> {l}, {r}"));
                Some((Val::V2I64, dst))
            }
            "vec_i64x2_shr" => {
                let Operand::Constant(Constant::Integer(shift)) = &args[1] else {
                    return Err(unsupported(self.mir_id, "vector shift not constant"));
                };
                let (_, v) = self.operand_value(&args[0])?;
                let dst = self.fresh();
                self.line(format!(
                    "{dst} = ashr <2 x i64> {v}, <i64 {shift}, i64 {shift}>"
                ));
                Some((Val::V2I64, dst))
            }
            "vec_i64x2_shr_var" => {
                let (_, l) = self.operand_value(&args[0])?;
                let (_, r) = self.operand_value(&args[1])?;
                let dst = self.fresh();
                self.line(format!("{dst} = ashr <2 x i64> {l}, {r}"));
                Some((Val::V2I64, dst))
            }
            "vec_i64x2_extract" => {
                let Operand::Constant(Constant::Integer(lane)) = &args[1] else {
                    return Err(unsupported(self.mir_id, "vector lane not constant"));
                };
                let (_, v) = self.operand_value(&args[0])?;
                let dst = self.fresh();
                self.line(format!("{dst} = extractelement <2 x i64> {v}, i32 {lane}"));
                Some((Val::I64, dst))
            }
            "vec_i64x2_sum" => {
                let (_, v) = self.operand_value(&args[0])?;
                let e0 = self.fresh();
                let e1 = self.fresh();
                let s = self.fresh();
                self.line(format!("{e0} = extractelement <2 x i64> {v}, i32 0"));
                self.line(format!("{e1} = extractelement <2 x i64> {v}, i32 1"));
                self.line(format!("{s} = add i64 {e0}, {e1}"));
                Some((Val::I64, s))
            }
            "vec_u8x16_splat" => {
                let (bt, bv) = self.operand_value(&args[0])?;
                let b8 = self.coerce(&bv, bt, Val::I8, false);
                let t0 = self.fresh();
                let t1 = self.fresh();
                let dst = self.fresh();
                self.line(format!(
                    "{t0} = insertelement <16 x i8> poison, i8 {b8}, i32 0"
                ));
                self.line(format!(
                    "{t1} = shufflevector <16 x i8> {t0}, <16 x i8> poison, <16 x i32> zeroinitializer"
                ));
                self.line(format!("{dst} = bitcast <16 x i8> {t1} to <2 x i64>"));
                Some((Val::V2I64, dst))
            }
            "vec_u8x16_eq" => {
                let (_, l) = self.operand_value(&args[0])?;
                let (_, r) = self.operand_value(&args[1])?;
                let l8 = self.fresh();
                let r8 = self.fresh();
                let c1 = self.fresh();
                let c8 = self.fresh();
                let dst = self.fresh();
                self.line(format!("{l8} = bitcast <2 x i64> {l} to <16 x i8>"));
                self.line(format!("{r8} = bitcast <2 x i64> {r} to <16 x i8>"));
                self.line(format!("{c1} = icmp eq <16 x i8> {l8}, {r8}"));
                self.line(format!("{c8} = sext <16 x i1> {c1} to <16 x i8>"));
                self.line(format!("{dst} = bitcast <16 x i8> {c8} to <2 x i64>"));
                Some((Val::V2I64, dst))
            }
            "vec_u8x16_movemask" | "vec_movemask8" => {
                let (_, v) = self.operand_value(&args[0])?;
                let v8 = self.fresh();
                self.line(format!("{v8} = bitcast <2 x i64> {v} to <16 x i8>"));
                let mut mask = "0".to_string();
                for lane in 0..16 {
                    let b = self.fresh();
                    let bit = self.fresh();
                    let bit64 = self.fresh();
                    let shifted = self.fresh();
                    let next_m = self.fresh();
                    self.line(format!("{b} = extractelement <16 x i8> {v8}, i32 {lane}"));
                    self.line(format!("{bit} = lshr i8 {b}, 7"));
                    self.line(format!("{bit64} = zext i8 {bit} to i64"));
                    self.line(format!("{shifted} = shl i64 {bit64}, {lane}"));
                    self.line(format!("{next_m} = or i64 {mask}, {shifted}"));
                    mask = next_m;
                }
                Some((Val::I64, mask))
            }
            "vec_i64_checksum" => {
                let (nt, nv) = self.operand_value(&args[0])?;
                let n = self.coerce(&nv, nt, Val::I64, true);
                self.emit_call_direct(IMP_VEC_CHECKSUM, &[(Val::I64, n)], Some(Val::I64))
            }
            _ => {
                let _ = builtin;
                return Err(unsupported(self.mir_id, "unknown simd builtin"));
            }
        };
        Ok(out)
    }
}
