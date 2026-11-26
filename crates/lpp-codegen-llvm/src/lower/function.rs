use std::collections::HashMap;
use std::fmt::Write as _;

use lpp_codegen_api::layout::tuple_layout;
use lpp_codegen_api::{CodegenError, NameResolver};
use lpp_mir::{
    BasicBlockId, BinaryOperator, Constant, InstructionKind, MirFunction, MirFunctionId,
    MirFunctionKind, MirLocalId, MirLocalKind, MirProgram, MirStringId, Operand, Rvalue,
    Terminator, UnaryOperator,
};
use lpp_types::{PrimitiveType, TypeInterner, TypeKind};

use super::managed::emit_unbox_from_i64;
use super::plan::{PlaceStep, Plan, ValueClass, operand_kind, resolve_place};
use super::{
    ElementClass, IMP_ARC_ALLOC, IMP_ARC_MARK_COLLECTIBLE, IMP_ARC_RELEASE, IMP_ARC_RETAIN,
    IMP_BOOL_TO_STR, IMP_CLOSURE_DESTROY, IMP_CLOSURE_TRACE, IMP_EMPTY_STR, IMP_FLOAT_TO_STR,
    IMP_INT_TO_STR, IMP_LIST_GET, IMP_LIST_GET_ARC, IMP_LIST_GET_BOOL, IMP_LIST_GET_FLOAT,
    IMP_LIST_LEN, IMP_LIST_NEW, IMP_LIST_NEW_ARC, IMP_LIST_PUSH, IMP_LIST_PUSH_ARC,
    IMP_LIST_PUSH_BOOL, IMP_LIST_PUSH_FLOAT, IMP_LIST_SET, IMP_LIST_SET_ARC, IMP_LIST_SET_BOOL,
    IMP_LIST_SET_FLOAT, IMP_LIST_TRACE, IMP_PANIC_DIV_ZERO, IMP_PGO_HIT, IMP_STR_CONCAT,
    IMP_STR_EQ, IMP_TASK_AWAIT, IMP_TASK_DESTROY, IMP_TASK_NEW, IMP_TASK_POLL, IMP_TASK_TRACE,
    IMP_TUPLE_ALLOC, IMP_TUPLE_TRACE, Val, element_class, is_managed, is_signed_integer_kind,
    is_void_type, llvm_sym, unsupported, user_parameters, val_of_type,
};

pub(crate) struct FnCtx<'a> {
    pub program: &'a MirProgram,
    pub types: &'a TypeInterner,
    pub plan: &'a Plan,
    pub mir_id: MirFunctionId,
    pub function: &'a MirFunction,
    pub string_ids: &'a HashMap<MirStringId, usize>,
    pub local_slots: HashMap<MirLocalId, (String, Val)>,
    pub capture_slots: HashMap<MirLocalId, usize>,
    pub env_slot: Option<String>,
    pub cur_block_label: String,
    prologue: String,
    body: String,
    seq: usize,
    lbl_seq: usize,
    terminated: bool,
    cur_dbg: Option<u32>,
}

impl<'a> FnCtx<'a> {
    pub fn fresh(&mut self) -> String {
        let s = format!("%v{}", self.seq);
        self.seq += 1;
        s
    }

    pub fn fresh_label(&mut self, prefix: &str) -> String {
        let s = format!("{prefix}_{}", self.lbl_seq);
        self.lbl_seq += 1;
        s
    }

    pub fn line(&mut self, s: impl AsRef<str>) {
        if let Some(dbg) = self.cur_dbg {
            let _ = writeln!(self.body, "  {}, !dbg !{dbg}", s.as_ref());
        } else {
            let _ = writeln!(self.body, "  {}", s.as_ref());
        }
    }

    pub fn start_label(&mut self, label: &str) {
        self.cur_block_label = label.to_string();
        let _ = writeln!(self.body, "{label}:");
        self.terminated = false;
    }

    pub fn entry_alloca(&mut self, ty_str: &str, align: u32) -> String {
        let slot = self.fresh();
        let _ = writeln!(self.prologue, "  {slot} = alloca {ty_str}, align {align}");
        slot
    }

    pub fn coerce(&mut self, val: &str, from: Val, to: Val, signed: bool) -> String {
        if from == to {
            return val.to_string();
        }
        let r = self.fresh();
        match (from, to) {
            (Val::Ptr, Val::I64) => self.line(format!("{r} = ptrtoint ptr {val} to i64")),
            (Val::I64, Val::Ptr) => self.line(format!("{r} = inttoptr i64 {val} to ptr")),
            (Val::Ptr, t) if t.is_int() => {
                let wide = self.fresh();
                self.line(format!("{wide} = ptrtoint ptr {val} to i64"));
                self.line(format!("{r} = trunc i64 {wide} to {}", t.llvm()));
            }
            (s, Val::Ptr) if s.is_int() => {
                let op = if signed { "sext" } else { "zext" };
                let wide = self.fresh();
                self.line(format!("{wide} = {op} {} {val} to i64", s.llvm()));
                self.line(format!("{r} = inttoptr i64 {wide} to ptr"));
            }
            (s, d) if s.is_int() && d.is_int() => {
                if s.bit_width() > d.bit_width() {
                    self.line(format!("{r} = trunc {} {val} to {}", s.llvm(), d.llvm()));
                } else {
                    let op = if signed { "sext" } else { "zext" };
                    self.line(format!("{r} = {op} {} {val} to {}", s.llvm(), d.llvm()));
                }
            }
            (Val::I64, Val::F64) => self.line(format!("{r} = bitcast i64 {val} to double")),
            (Val::F64, Val::I64) => self.line(format!("{r} = bitcast double {val} to i64")),
            _ => return val.to_string(),
        }
        r
    }

    pub fn emit_call_direct(
        &mut self,
        symbol: &str,
        args: &[(Val, String)],
        result: Option<Val>,
    ) -> Option<(Val, String)> {
        let args_str: Vec<String> = args
            .iter()
            .map(|(v, n)| format!("{} {n}", v.llvm()))
            .collect();
        let sym = llvm_sym(symbol);
        match result {
            Some(rv) => {
                let res = self.fresh();
                self.line(format!(
                    "{res} = call {} @{sym}({})",
                    rv.llvm(),
                    args_str.join(", ")
                ));
                Some((rv, res))
            }
            None => {
                self.line(format!("call void @{sym}({})", args_str.join(", ")));
                None
            }
        }
    }

    pub fn arc_retain(&mut self, ptr: &str) {
        self.line(format!("call void @{IMP_ARC_RETAIN}(ptr {ptr})"));
    }

    pub fn arc_release(&mut self, ptr: &str) {
        self.line(format!("call void @{IMP_ARC_RELEASE}(ptr {ptr})"));
    }

    pub fn mark_collectible(&mut self, ptr: &str, trace_symbol: &str) {
        let sym = llvm_sym(trace_symbol);
        self.line(format!(
            "call void @{IMP_ARC_MARK_COLLECTIBLE}(ptr {ptr}, ptr @{sym})"
        ));
    }

    pub fn empty_moved_local(&mut self, op: &Operand) {
        let Operand::Copy(local) = op else {
            return;
        };
        let ty = self.program.local(*local).expect("local").ty;
        if !is_managed(self.types, ty) {
            return;
        }
        if let Some((slot, _)) = self.local_slots.get(local) {
            let slot = slot.clone();
            self.line(format!("store ptr null, ptr {slot}, align 8"));
        }
    }

    pub fn operand_value(&mut self, op: &Operand) -> Result<(Val, String), CodegenError> {
        match op {
            Operand::Copy(local) => {
                let ty = self.program.local(*local).expect("local").ty;
                if is_void_type(self.types, ty) {
                    return Ok((Val::Void, String::new()));
                }
                let vty = val_of_type(self.types, ty, self.mir_id)?;
                let (slot, slot_ty) = self
                    .local_slots
                    .get(local)
                    .cloned()
                    .ok_or_else(|| unsupported(self.mir_id, "read void local"))?;
                let r = self.fresh();
                self.line(format!(
                    "{r} = load {}, ptr {slot}, align {}",
                    slot_ty.llvm(),
                    slot_ty.align()
                ));
                Ok((vty, r))
            }
            Operand::Constant(c) => Ok(match c {
                Constant::Integer(v) => (Val::I64, v.to_string()),
                Constant::FloatBits(bits) => (Val::F64, format!("0x{:016x}", bits)),
                Constant::Bool(b) => (Val::I8, if *b { "1" } else { "0" }.to_string()),
                Constant::Character { character, .. } => {
                    (Val::I32, (*character as i32).to_string())
                }
                Constant::String { string, .. } => {
                    let id = self.string_ids[string];
                    let text = self.program.string(*string).expect("string");
                    let len = 16 + text.len() + 1;
                    (
                        Val::Ptr,
                        format!(
                            "getelementptr inbounds ([{len} x i8], ptr @.str.{id}, i64 0, i64 16)"
                        ),
                    )
                }
            }),
            Operand::Function(_) => Err(unsupported(self.mir_id, "bare function operand")),
        }
    }

    pub fn owned_value(&mut self, op: &Operand) -> Result<(Val, String), CodegenError> {
        let (vty, val) = self.operand_value(op)?;
        if let Operand::Copy(local) = op {
            let ty = self.program.local(*local).expect("local").ty;
            if is_managed(self.types, ty) {
                let p = self.coerce(&val, vty, Val::Ptr, false);
                self.arc_retain(&p);
                return Ok((Val::Ptr, p));
            }
        }
        Ok((vty, val))
    }
}

pub(crate) struct EmitFunctionCtx<'a> {
    pub(crate) program: &'a MirProgram,
    pub(crate) types: &'a TypeInterner,
    pub(crate) plan: &'a Plan,
    pub(crate) string_ids: &'a HashMap<MirStringId, usize>,
    pub(crate) pgo_fingerprint: Option<u64>,
    pub(crate) names: &'a dyn NameResolver,
}

pub(crate) fn emit_function(
    out: &mut String,
    ctx: &EmitFunctionCtx<'_>,
    mir_id: MirFunctionId,
    function: &MirFunction,
    dbg: &mut super::debug::DebugMetadata<'_>,
) -> Result<(), CodegenError> {
    let EmitFunctionCtx {
        program,
        types,
        plan,
        string_ids,
        pgo_fingerprint,
        names,
    } = *ctx;
    let is_closure = matches!(function.kind, MirFunctionKind::Closure);
    let capture_locals: Vec<MirLocalId> = program
        .function_parameters(function)
        .iter()
        .copied()
        .filter(|l| matches!(program.local(*l).expect("l").kind, MirLocalKind::Capture))
        .collect();
    let mut capture_slots = HashMap::new();
    for (i, &local) in capture_locals.iter().enumerate() {
        capture_slots.insert(local, i);
    }

    let mut params_str = Vec::new();
    if is_closure {
        params_str.push("ptr %__cenv".to_string());
    }
    let mut user_param_locals = Vec::new();
    for &param in program.function_parameters(function) {
        if is_closure && capture_locals.contains(&param) {
            continue;
        }
        let ty = program.local(param).expect("local").ty;
        let v = val_of_type(types, ty, mir_id)?;
        let idx = user_param_locals.len();
        params_str.push(format!("{} %p{idx}", v.llvm()));
        user_param_locals.push((param, v));
    }

    let ret_val = if is_void_type(types, function.return_type) {
        Val::Void
    } else {
        val_of_type(types, function.return_type, mir_id)?
    };
    let export_name = &plan.export_names[&mir_id];
    let name = llvm_sym(export_name);
    let source_name = function
        .name
        .and_then(|s| names.resolve(s.raw()))
        .unwrap_or(export_name.as_str());
    let dbg_pair = dbg.register_subprogram(source_name, export_name, function.origin.raw());
    let dbg_fn_attr = dbg_pair
        .map(|(sp, _)| format!(" !dbg !{sp}"))
        .unwrap_or_default();
    let attrs = program.function_attrs(mir_id);
    let sec_attr = attrs
        .section
        .as_deref()
        .map(|s| format!(" section \"{s}\""))
        .unwrap_or_default();

    let _ = writeln!(
        out,
        "define {} @{name}({}){sec_attr}{dbg_fn_attr} {{",
        ret_val.llvm(),
        params_str.join(", ")
    );

    let mut ctx = FnCtx {
        program,
        types,
        plan,
        mir_id,
        function,
        string_ids,
        local_slots: HashMap::new(),
        capture_slots,
        env_slot: None,
        cur_block_label: "prologue".to_string(),
        prologue: String::new(),
        body: String::new(),
        seq: 0,
        lbl_seq: 0,
        terminated: false,
        cur_dbg: dbg_pair.map(|(_, d)| d),
    };

    // Allocate every non-Void local in prologue
    let _ = writeln!(ctx.prologue, "prologue:");
    if is_closure {
        let env_alloca = ctx.fresh();
        let _ = writeln!(ctx.prologue, "  {env_alloca} = alloca ptr, align 8");
        let _ = writeln!(ctx.body, "  store ptr %__cenv, ptr {env_alloca}, align 8");
        ctx.env_slot = Some(env_alloca);
    }

    for &local in program.function_locals(function) {
        let desc = program.local(local).expect("local");
        if is_void_type(types, desc.ty) {
            continue;
        }
        if let Some(&stack_bytes) = plan.stack_locals.get(&(mir_id, local)) {
            let buf = format!("%loc_{}_buf", local.raw());
            let slot = format!("%loc_{}", local.raw());
            let _ = writeln!(
                ctx.prologue,
                "  {buf} = alloca [{stack_bytes} x i8], align 8"
            );
            let _ = writeln!(ctx.prologue, "  {slot} = alloca ptr, align 8");
            let _ = writeln!(ctx.body, "  store ptr {buf}, ptr {slot}, align 8");
            ctx.local_slots.insert(local, (slot, Val::Ptr));
            continue;
        }
        let v = val_of_type(types, desc.ty, mir_id)?;
        let slot = format!("%loc_{}", local.raw());
        let _ = writeln!(
            ctx.prologue,
            "  {slot} = alloca {}, align {}",
            v.llvm(),
            v.align()
        );
        ctx.local_slots.insert(local, (slot.clone(), v));
        if !matches!(desc.kind, MirLocalKind::Parameter) {
            let zero = v.zero_const();
            let _ = writeln!(
                ctx.body,
                "  store {} {zero}, ptr {slot}, align {}",
                v.llvm(),
                v.align()
            );
        }
    }

    // Store captures from %__cenv
    for (i, &cap_local) in capture_locals.iter().enumerate() {
        if let Some((slot, v)) = ctx.local_slots.get(&cap_local).cloned() {
            let gep = ctx.fresh();
            let raw = ctx.fresh();
            let _ = writeln!(
                ctx.body,
                "  {gep} = getelementptr i8, ptr %__cenv, i64 {}",
                8 * i
            );
            let _ = writeln!(ctx.body, "  {raw} = load i64, ptr {gep}, align 8");
            let loaded = ctx.coerce(&raw, Val::I64, v, true);
            let _ = writeln!(
                ctx.body,
                "  store {} {loaded}, ptr {slot}, align {}",
                v.llvm(),
                v.align()
            );
        }
    }

    // Store user parameters
    for (idx, (param, v)) in user_param_locals.iter().enumerate() {
        let (slot, _) = &ctx.local_slots[param];
        let _ = writeln!(
            ctx.body,
            "  store {} %p{idx}, ptr {slot}, align {}",
            v.llvm(),
            v.align()
        );
    }

    if pgo_fingerprint.is_some() {
        ctx.emit_call_direct(IMP_PGO_HIT, &[(Val::I64, mir_id.raw().to_string())], None);
    }

    let entry_lbl = format!("bb{}", function.entry.raw());
    let _ = writeln!(ctx.body, "  br label %{entry_lbl}");

    for &block_id in program.function_blocks(function) {
        let block = program.block(block_id).expect("block");
        ctx.start_label(&format!("bb{}", block_id.raw()));
        for &inst_id in program.block_instructions(block) {
            let instr = program.instruction(inst_id).expect("inst");
            if let Some((sp_id, _)) = dbg_pair {
                ctx.cur_dbg = dbg.location_for_origin(sp_id, instr.origin.raw());
            }
            match &instr.kind {
                InstructionKind::Store { place, value } => ctx.lower_store(*place, value)?,
                InstructionKind::Assign { target, value } => ctx.lower_assign(*target, value)?,
            }
        }
        if let Some((sp_id, _)) = dbg_pair {
            ctx.cur_dbg = dbg.location_for_origin(sp_id, block.origin.raw());
        }
        ctx.lower_terminator(&block.terminator)?;
    }

    let _ = writeln!(out, "{}{}}}", ctx.prologue, ctx.body);
    Ok(())
}

impl<'a> FnCtx<'a> {
    fn place_chain_base(
        &mut self,
        res: &super::plan::PlaceResolution,
    ) -> Result<String, CodegenError> {
        let (slot, _) = self.local_slots[&res.root].clone();
        let mut base = self.fresh();
        self.line(format!("{base} = load ptr, ptr {slot}, align 8"));
        for step in res.steps.iter().take(res.steps.len().saturating_sub(1)) {
            base = match step {
                PlaceStep::Field { offset, .. } => {
                    let gep = self.fresh();
                    let next = self.fresh();
                    self.line(format!(
                        "{gep} = getelementptr i8, ptr {base}, i64 {offset}"
                    ));
                    self.line(format!("{next} = load ptr, ptr {gep}, align 8"));
                    next
                }
                PlaceStep::ListIndex { index, .. } => {
                    let (it, iv) = self.operand_value(index)?;
                    let idx = self.coerce(&iv, it, Val::I64, true);
                    self.emit_call_direct(
                        IMP_LIST_GET_ARC,
                        &[(Val::Ptr, base), (Val::I64, idx)],
                        Some(Val::Ptr),
                    )
                    .unwrap()
                    .1
                }
            };
        }
        Ok(base)
    }

    fn lower_store(
        &mut self,
        place: lpp_mir::MirPlaceId,
        value: &Operand,
    ) -> Result<(), CodegenError> {
        let res = resolve_place(
            self.program,
            self.types,
            place,
            &self.plan.aggregates,
            &self.plan.layouts,
            self.mir_id,
            true,
        )?;
        if res.steps.is_empty() {
            if is_void_type(self.types, res.ty) {
                return Ok(());
            }
            let is_cap = self.capture_slots.contains_key(&res.root);
            let (new_vty, new_val) = self.owned_value(value)?;
            let (slot, slot_ty) = self.local_slots[&res.root].clone();
            let signed = is_signed_integer_kind(Some(self.types.kind(res.ty)));
            let stored = self.coerce(&new_val, new_vty, slot_ty, signed);
            if is_managed(self.types, res.ty) && !is_cap {
                let old = self.fresh();
                self.line(format!("{old} = load ptr, ptr {slot}, align 8"));
                self.arc_release(&old);
            }
            self.line(format!(
                "store {} {stored}, ptr {slot}, align {}",
                slot_ty.llvm(),
                slot_ty.align()
            ));
            if let Some(&cap_idx) = self.capture_slots.get(&res.root) {
                let env_slot = self.env_slot.clone().expect("closure env");
                let env = self.fresh();
                self.line(format!("{env} = load ptr, ptr {env_slot}, align 8"));
                let gep = self.fresh();
                self.line(format!(
                    "{gep} = getelementptr i8, ptr {env}, i64 {}",
                    8 * cap_idx
                ));
                if is_managed(self.types, res.ty) {
                    let old = self.fresh();
                    self.line(format!("{old} = load ptr, ptr {gep}, align 8"));
                    self.arc_release(&old);
                }
                let raw_i64 = self.coerce(&stored, slot_ty, Val::I64, signed);
                self.line(format!("store i64 {raw_i64}, ptr {gep}, align 8"));
            }
            return Ok(());
        }

        match res.steps.last().expect("non-empty") {
            PlaceStep::Field {
                offset,
                ty,
                managed,
            } => {
                let base = self.place_chain_base(&res)?;
                let (new_vty, new_val) = self.owned_value(value)?;
                let field_ty = val_of_type(self.types, *ty, self.mir_id)?;
                let gep = self.fresh();
                self.line(format!(
                    "{gep} = getelementptr i8, ptr {base}, i64 {offset}"
                ));
                if *managed {
                    let old = self.fresh();
                    self.line(format!("{old} = load ptr, ptr {gep}, align 8"));
                    self.arc_release(&old);
                }
                let signed = is_signed_integer_kind(Some(self.types.kind(*ty)));
                let stored = self.coerce(&new_val, new_vty, field_ty, signed);
                self.line(format!(
                    "store {} {stored}, ptr {gep}, align 1",
                    field_ty.llvm()
                ));
            }
            PlaceStep::ListIndex { element, index } => {
                let base = self.place_chain_base(&res)?;
                let (it, iv) = self.operand_value(index)?;
                let idx = self.coerce(&iv, it, Val::I64, true);
                let (vt, vn) = self.operand_value(value)?;
                match element_class(self.types, *element) {
                    ElementClass::Managed => {
                        let vp = self.coerce(&vn, vt, Val::Ptr, false);
                        self.emit_call_direct(
                            IMP_LIST_SET_ARC,
                            &[(Val::Ptr, base), (Val::I64, idx), (Val::Ptr, vp)],
                            None,
                        );
                    }
                    ElementClass::Scalar => {
                        let vs = self.coerce(
                            &vn,
                            vt,
                            Val::I64,
                            is_signed_integer_kind(Some(self.types.kind(*element))),
                        );
                        self.emit_call_direct(
                            IMP_LIST_SET,
                            &[(Val::Ptr, base), (Val::I64, idx), (Val::I64, vs)],
                            None,
                        );
                    }
                    ElementClass::Float => {
                        let vf = self.coerce(&vn, vt, Val::F64, false);
                        self.emit_call_direct(
                            IMP_LIST_SET_FLOAT,
                            &[(Val::Ptr, base), (Val::I64, idx), (Val::F64, vf)],
                            None,
                        );
                    }
                    ElementClass::Bool => {
                        let vb = self.coerce(&vn, vt, Val::I8, false);
                        self.emit_call_direct(
                            IMP_LIST_SET_BOOL,
                            &[(Val::Ptr, base), (Val::I64, idx), (Val::I8, vb)],
                            None,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn lower_assign(&mut self, target: MirLocalId, value: &Rvalue) -> Result<(), CodegenError> {
        let target_ty = self.program.local(target).expect("local").ty;
        let is_void = is_void_type(self.types, target_ty);

        let produced: Option<(Val, String)> = match value {
            Rvalue::Use(op) => {
                if let Operand::Copy(src) = op
                    && let Some(&stack_bytes) = self.plan.stack_locals.get(&(self.mir_id, *src))
                    && self.plan.stack_locals.contains_key(&(self.mir_id, target))
                {
                    let src_slot = self.local_slots[src].0.clone();
                    let dst_slot = self.local_slots[&target].0.clone();
                    let src_ptr = self.fresh();
                    let dst_ptr = self.fresh();
                    self.line(format!("{src_ptr} = load ptr, ptr {src_slot}, align 8"));
                    self.line(format!("{dst_ptr} = load ptr, ptr {dst_slot}, align 8"));
                    self.copy_stack_bytes(&dst_ptr, &src_ptr, stack_bytes);
                    None
                } else if let Operand::Function(fn_id) = op {
                    Some((Val::Ptr, self.materialize_function_value(*fn_id)?))
                } else {
                    Some(self.owned_value(op)?)
                }
            }
            Rvalue::Unary { operator, operand } => {
                let (ty, val) = self.operand_value(operand)?;
                let r = match (operator, ty) {
                    (UnaryOperator::Negate, t) if t.is_int() => {
                        let r = self.fresh();
                        self.line(format!("{r} = sub {} 0, {val}", t.llvm()));
                        r
                    }
                    (UnaryOperator::Negate, Val::F64) => {
                        let r = self.fresh();
                        self.line(format!("{r} = fneg double {val}"));
                        r
                    }
                    (UnaryOperator::Plus, t) if t.is_int() || t == Val::F64 => val,
                    (UnaryOperator::BitNot, t) if t.is_int() => {
                        let r = self.fresh();
                        self.line(format!("{r} = xor {} {val}, -1", t.llvm()));
                        r
                    }
                    (UnaryOperator::Not, Val::I8) => {
                        let r = self.fresh();
                        self.line(format!("{r} = xor i8 {val}, 1"));
                        r
                    }
                    _ => return Err(unsupported(self.mir_id, "invalid unary operands")),
                };
                Some((ty, r))
            }
            Rvalue::Binary {
                left,
                operator,
                right,
            } => Some(self.lower_binary(*operator, left, right)?),
            Rvalue::ExternalCall {
                function: external_id,
                arguments,
            } => {
                let ext = self.program.external_function(*external_id).expect("ext");
                let TypeKind::Function { parameters, result } = self.types.kind(ext.ty) else {
                    return Err(unsupported(self.mir_id, "ext non-fn"));
                };
                let param_tys = self.types.list(parameters).to_vec();
                let ops = self.program.operands(*arguments);
                let mut call_args = Vec::with_capacity(ops.len());
                for (op, pt) in ops.iter().zip(param_tys.iter()) {
                    let (vt, vn) = self.operand_value(op)?;
                    let expected = val_of_type(self.types, *pt, self.mir_id)?;
                    let signed = is_signed_integer_kind(Some(self.types.kind(*pt)));
                    call_args.push((expected, self.coerce(&vn, vt, expected, signed)));
                }
                let ret_v = if is_void_type(self.types, result) {
                    None
                } else {
                    Some(val_of_type(self.types, result, self.mir_id)?)
                };
                let res = self.emit_call_direct(&ext.name, &call_args, ret_v);
                if matches!(
                    self.types.kind(result),
                    TypeKind::Primitive(PrimitiveType::String)
                ) {
                    let (_, foreign) = res.expect("string ptr");
                    let (_, empty) = self
                        .emit_call_direct(IMP_EMPTY_STR, &[], Some(Val::Ptr))
                        .unwrap();
                    self.emit_call_direct(
                        IMP_STR_CONCAT,
                        &[(Val::Ptr, foreign), (Val::Ptr, empty)],
                        Some(Val::Ptr),
                    )
                } else {
                    res
                }
            }
            Rvalue::Call { callee, arguments } => {
                self.lower_call(callee, self.program.operands(*arguments))?
            }
            Rvalue::Load(place) => {
                let res = resolve_place(
                    self.program,
                    self.types,
                    *place,
                    &self.plan.aggregates,
                    &self.plan.layouts,
                    self.mir_id,
                    false,
                )?;
                if res.steps.is_empty() {
                    Some(self.operand_value(&Operand::Copy(res.root))?)
                } else {
                    let base = self.place_chain_base(&res)?;
                    match res.steps.last().expect("step") {
                        PlaceStep::Field {
                            offset,
                            ty,
                            managed,
                        } => {
                            let fty = val_of_type(self.types, *ty, self.mir_id)?;
                            let gep = self.fresh();
                            let loaded = self.fresh();
                            self.line(format!(
                                "{gep} = getelementptr i8, ptr {base}, i64 {offset}"
                            ));
                            self.line(format!(
                                "{loaded} = load {}, ptr {gep}, align 1",
                                fty.llvm()
                            ));
                            if *managed {
                                let p = self.coerce(&loaded, fty, Val::Ptr, false);
                                self.arc_retain(&p);
                                Some((Val::Ptr, p))
                            } else {
                                Some((fty, loaded))
                            }
                        }
                        PlaceStep::ListIndex { element, index } => {
                            let (it, iv) = self.operand_value(index)?;
                            let idx = self.coerce(&iv, it, Val::I64, true);
                            match element_class(self.types, *element) {
                                ElementClass::Managed => {
                                    let (_, v) = self
                                        .emit_call_direct(
                                            IMP_LIST_GET_ARC,
                                            &[(Val::Ptr, base), (Val::I64, idx)],
                                            Some(Val::Ptr),
                                        )
                                        .unwrap();
                                    self.arc_retain(&v);
                                    Some((Val::Ptr, v))
                                }
                                ElementClass::Scalar => {
                                    let (_, v) = self
                                        .emit_call_direct(
                                            IMP_LIST_GET,
                                            &[(Val::Ptr, base), (Val::I64, idx)],
                                            Some(Val::I64),
                                        )
                                        .unwrap();
                                    let target_e = val_of_type(self.types, *element, self.mir_id)?;
                                    let c = self.coerce(
                                        &v,
                                        Val::I64,
                                        target_e,
                                        is_signed_integer_kind(Some(self.types.kind(*element))),
                                    );
                                    Some((target_e, c))
                                }
                                ElementClass::Float => self.emit_call_direct(
                                    IMP_LIST_GET_FLOAT,
                                    &[(Val::Ptr, base), (Val::I64, idx)],
                                    Some(Val::F64),
                                ),
                                ElementClass::Bool => self.emit_call_direct(
                                    IMP_LIST_GET_BOOL,
                                    &[(Val::Ptr, base), (Val::I64, idx)],
                                    Some(Val::I8),
                                ),
                            }
                        }
                    }
                }
            }
            Rvalue::ListLen(op) => {
                let (t, n) = self.operand_value(op)?;
                let p = self.coerce(&n, t, Val::Ptr, false);
                self.emit_call_direct(IMP_LIST_LEN, &[(Val::Ptr, p)], Some(Val::I64))
            }
            Rvalue::List(operands) => {
                let TypeKind::List(elem) = self.types.kind(target_ty) else {
                    return Err(unsupported(self.mir_id, "list non-list"));
                };
                let new_sym = if is_managed(self.types, elem) {
                    IMP_LIST_NEW_ARC
                } else {
                    IMP_LIST_NEW
                };
                let (_, list) = self.emit_call_direct(new_sym, &[], Some(Val::Ptr)).unwrap();
                if self.plan.shared_types.contains(&target_ty) {
                    self.mark_collectible(&list, IMP_LIST_TRACE);
                }
                for op in self.program.operands(*operands) {
                    let (vt, vn) = self.operand_value(op)?;
                    match element_class(self.types, elem) {
                        ElementClass::Managed => {
                            let vp = self.coerce(&vn, vt, Val::Ptr, false);
                            self.emit_call_direct(
                                IMP_LIST_PUSH_ARC,
                                &[(Val::Ptr, list.clone()), (Val::Ptr, vp.clone())],
                                None,
                            );
                            self.arc_release(&vp);
                            self.empty_moved_local(op);
                        }
                        ElementClass::Scalar => {
                            let vs = self.coerce(
                                &vn,
                                vt,
                                Val::I64,
                                is_signed_integer_kind(Some(self.types.kind(elem))),
                            );
                            self.emit_call_direct(
                                IMP_LIST_PUSH,
                                &[(Val::Ptr, list.clone()), (Val::I64, vs)],
                                None,
                            );
                        }
                        ElementClass::Float => {
                            let vf = self.coerce(&vn, vt, Val::F64, false);
                            self.emit_call_direct(
                                IMP_LIST_PUSH_FLOAT,
                                &[(Val::Ptr, list.clone()), (Val::F64, vf)],
                                None,
                            );
                        }
                        ElementClass::Bool => {
                            let vb = self.coerce(&vn, vt, Val::I8, false);
                            self.emit_call_direct(
                                IMP_LIST_PUSH_BOOL,
                                &[(Val::Ptr, list.clone()), (Val::I8, vb)],
                                None,
                            );
                        }
                    }
                }
                Some((Val::Ptr, list))
            }
            Rvalue::ConstructStruct { aggregate, fields } => {
                let agg_desc = self.program.aggregate(*aggregate).expect("agg");
                let layout = self.plan.layouts[aggregate].clone();
                let is_stack = self.plan.stack_locals.contains_key(&(self.mir_id, target));
                let ptr = if is_stack {
                    let slot = self.local_slots[&target].0.clone();
                    let p = self.fresh();
                    self.line(format!("{p} = load ptr, ptr {slot}, align 8"));
                    p
                } else {
                    let size = layout.total_size.max(8) as usize;
                    let is_pod_owned = !self.plan.shared_types.contains(&agg_desc.ty)
                        && lpp_codegen_api::is_pod_aggregate(&layout, self.types);
                    let ptr = if is_pod_owned {
                        self.arc_alloc_no_dtor(size)
                    } else {
                        let dtor = format!("lpp_drop_s{}", aggregate.raw());
                        self.arc_alloc(size, &dtor)
                    };
                    if self.plan.shared_types.contains(&agg_desc.ty) {
                        let trace = format!("lpp_trace_s{}", aggregate.raw());
                        self.mark_collectible(&ptr, &trace);
                    }
                    ptr
                };
                let values = self.program.operands(*fields);
                for (op, slot) in values.iter().zip(layout.struct_fields.iter()) {
                    let (vt, vn) = self.owned_value(op)?;
                    let fty = val_of_type(self.types, slot.ty, self.mir_id)?;
                    let signed = is_signed_integer_kind(Some(self.types.kind(slot.ty)));
                    let stored = self.coerce(&vn, vt, fty, signed);
                    let gep = self.fresh();
                    self.line(format!(
                        "{gep} = getelementptr i8, ptr {ptr}, i64 {}",
                        slot.offset
                    ));
                    self.line(format!("store {} {stored}, ptr {gep}, align 1", fty.llvm()));
                }
                if is_stack {
                    None
                } else {
                    Some((Val::Ptr, ptr))
                }
            }
            Rvalue::ConstructVariant {
                aggregate,
                variant,
                fields,
            } => {
                let agg_desc = self.program.aggregate(*aggregate).expect("agg");
                let layout = self.plan.layouts[aggregate].clone();
                let ord = self.program.variant(*variant).expect("var").ordinal;
                let size = layout.total_size.max(8) as usize;
                let is_pod_owned = !self.plan.shared_types.contains(&agg_desc.ty)
                    && lpp_codegen_api::is_pod_aggregate(&layout, self.types);
                let ptr = if is_pod_owned {
                    self.arc_alloc_no_dtor(size)
                } else {
                    let dtor = format!("lpp_drop_e{}", aggregate.raw());
                    self.arc_alloc(size, &dtor)
                };
                if self.plan.shared_types.contains(&agg_desc.ty) {
                    let trace = format!("lpp_trace_e{}", aggregate.raw());
                    self.mark_collectible(&ptr, &trace);
                }
                self.line(format!("store i64 {ord}, ptr {ptr}, align 8"));
                let slots = &layout.variant_fields[ord as usize];
                let values = self.program.operands(*fields);
                for (op, slot) in values.iter().zip(slots.iter()) {
                    let (vt, vn) = self.owned_value(op)?;
                    let fty = val_of_type(self.types, slot.ty, self.mir_id)?;
                    let signed = is_signed_integer_kind(Some(self.types.kind(slot.ty)));
                    let stored = self.coerce(&vn, vt, fty, signed);
                    let gep = self.fresh();
                    self.line(format!(
                        "{gep} = getelementptr i8, ptr {ptr}, i64 {}",
                        slot.offset
                    ));
                    self.line(format!("store {} {stored}, ptr {gep}, align 1", fty.llvm()));
                }
                Some((Val::Ptr, ptr))
            }
            Rvalue::Tuple(elements) => {
                let (slots, total_size) = tuple_layout(self.types, target_ty)
                    .ok_or_else(|| unsupported(self.mir_id, "tuple non-tuple"))?;
                let is_stack = self.plan.stack_locals.contains_key(&(self.mir_id, target));
                let ptr = if is_stack {
                    let slot = self.local_slots[&target].0.clone();
                    let p = self.fresh();
                    self.line(format!("{p} = load ptr, ptr {slot}, align 8"));
                    p
                } else {
                    let size = total_size.max(8) as usize;
                    let is_pod_owned = !self.plan.shared_types.contains(&target_ty)
                        && lpp_codegen_api::is_pod_tuple(self.types, target_ty);
                    let ptr = if is_pod_owned {
                        self.arc_alloc_no_dtor(size)
                    } else {
                        let dtor = format!("lpp_drop_t{}", target_ty.raw());
                        self.arc_alloc(size, &dtor)
                    };
                    if self.plan.shared_types.contains(&target_ty) {
                        let trace = format!("lpp_trace_t{}", target_ty.raw());
                        self.mark_collectible(&ptr, &trace);
                    }
                    ptr
                };
                let values = self.program.operands(*elements);
                for (op, (offset, ety)) in values.iter().zip(slots.iter()) {
                    let (vt, vn) = self.owned_value(op)?;
                    let fty = val_of_type(self.types, *ety, self.mir_id)?;
                    let signed = is_signed_integer_kind(Some(self.types.kind(*ety)));
                    let stored = self.coerce(&vn, vt, fty, signed);
                    let gep = self.fresh();
                    self.line(format!("{gep} = getelementptr i8, ptr {ptr}, i64 {offset}"));
                    self.line(format!("store {} {stored}, ptr {gep}, align 1", fty.llvm()));
                }
                if is_stack {
                    None
                } else {
                    Some((Val::Ptr, ptr))
                }
            }
            Rvalue::MakeClosure {
                function: closure_id,
                captures,
            } => Some((Val::Ptr, self.lower_make_closure(*closure_id, *captures)?)),
            Rvalue::Await(op) => {
                let Operand::Copy(local) = op else {
                    return Err(unsupported(self.mir_id, "await non-local"));
                };
                let (vt, vn) = self.operand_value(op)?;
                let tp = self.coerce(&vn, vt, Val::Ptr, false);
                let (_, boxed) = self
                    .emit_call_direct(IMP_TASK_AWAIT, &[(Val::Ptr, tp)], Some(Val::I64))
                    .unwrap();
                let TypeKind::Task(inner) =
                    self.types.kind(self.program.local(*local).expect("l").ty)
                else {
                    return Err(unsupported(self.mir_id, "await non-task"));
                };
                emit_unbox_from_i64(&mut self.body, &boxed, inner, self.types, &mut || {
                    let r = format!("%v{}", self.seq);
                    self.seq += 1;
                    r
                })
            }
            Rvalue::Spawn(op) => {
                let Operand::Copy(local) = op else {
                    return Err(unsupported(self.mir_id, "spawn non-local"));
                };
                let (ct, cn) = self.operand_value(op)?;
                let cap = self.coerce(&cn, ct, Val::Ptr, false);
                let code = self.fresh();
                self.line(format!("{code} = load ptr, ptr {cap}, align 8"));
                let gep_env = self.fresh();
                let env = self.fresh();
                self.line(format!("{gep_env} = getelementptr i8, ptr {cap}, i64 8"));
                self.line(format!("{env} = load ptr, ptr {gep_env}, align 8"));
                self.arc_retain(&env);
                let TypeKind::Function { result, .. } =
                    self.types.kind(self.program.local(*local).expect("l").ty)
                else {
                    return Err(unsupported(self.mir_id, "spawn non-fn"));
                };
                let managed = Self::task_managed_flag(self.types, result);
                let (_, task) = self
                    .emit_call_direct(
                        IMP_TASK_NEW,
                        &[
                            (Val::Ptr, code),
                            (Val::Ptr, env),
                            (Val::I64, managed.to_string()),
                        ],
                        Some(Val::Ptr),
                    )
                    .unwrap();
                self.emit_call_direct(IMP_TASK_POLL, &[(Val::Ptr, task.clone())], Some(Val::I64));
                self.emit_call_direct(IMP_TASK_DESTROY, &[(Val::Ptr, task)], None);
                None
            }
            Rvalue::Builtin { builtin, arguments } => {
                self.lower_builtin(*builtin, self.program.operands(*arguments), target_ty)?
            }
            Rvalue::Cast { value, target } => Some(self.lower_cast(value, *target)?),
            Rvalue::Intrinsic {
                intrinsic,
                type_argument,
                arguments,
            } => self.lower_kernel_intrinsic(
                *intrinsic,
                *type_argument,
                self.program.operands(*arguments),
                target_ty,
            )?,
        };

        if let Some((pv, pn)) = produced
            && !is_void
            && pv != Val::Void
        {
            let (slot, slot_ty) = self.local_slots[&target].clone();
            if is_managed(self.types, target_ty) {
                let old = self.fresh();
                self.line(format!("{old} = load ptr, ptr {slot}, align 8"));
                self.arc_release(&old);
            }
            let signed = is_signed_integer_kind(Some(self.types.kind(target_ty)));
            let final_name = self.coerce(&pn, pv, slot_ty, signed);
            self.line(format!(
                "store {} {final_name}, ptr {slot}, align {}",
                slot_ty.llvm(),
                slot_ty.align()
            ));
        }
        Ok(())
    }

    fn arc_alloc(&mut self, size: usize, dtor_symbol: &str) -> String {
        let sym = llvm_sym(dtor_symbol);
        let r = self.fresh();
        self.line(format!(
            "{r} = call ptr @{IMP_ARC_ALLOC}(i64 {size}, ptr @{sym})"
        ));
        r
    }

    fn arc_alloc_no_dtor(&mut self, size: usize) -> String {
        let r = self.fresh();
        self.line(format!(
            "{r} = call ptr @{IMP_ARC_ALLOC}(i64 {size}, ptr null)"
        ));
        r
    }

    /// Copies `bytes` between stack-allocated POD buffers without ARC retain/release.
    fn copy_stack_bytes(&mut self, dst: &str, src: &str, bytes: u32) {
        let mut offset = 0u32;
        while offset + 8 <= bytes {
            let sgep = self.fresh();
            let dgep = self.fresh();
            let val = self.fresh();
            self.line(format!(
                "{sgep} = getelementptr i8, ptr {src}, i64 {offset}"
            ));
            self.line(format!("{val} = load i64, ptr {sgep}, align 8"));
            self.line(format!(
                "{dgep} = getelementptr i8, ptr {dst}, i64 {offset}"
            ));
            self.line(format!("store i64 {val}, ptr {dgep}, align 8"));
            offset += 8;
        }
        while offset < bytes {
            let sgep = self.fresh();
            let dgep = self.fresh();
            let val = self.fresh();
            self.line(format!(
                "{sgep} = getelementptr i8, ptr {src}, i64 {offset}"
            ));
            self.line(format!("{val} = load i8, ptr {sgep}, align 1"));
            self.line(format!(
                "{dgep} = getelementptr i8, ptr {dst}, i64 {offset}"
            ));
            self.line(format!("store i8 {val}, ptr {dgep}, align 1"));
            offset += 1;
        }
    }

    fn task_managed_flag(types: &TypeInterner, return_type: lpp_types::TypeId) -> i64 {
        let inner = match types.kind(return_type) {
            TypeKind::Task(i) => i,
            _ => return_type,
        };
        if is_managed(types, inner) { 1 } else { 0 }
    }

    fn task_env_for_args(
        &mut self,
        args_ops: &[Operand],
        collectible: bool,
    ) -> Result<String, CodegenError> {
        let mut mask = 0i64;
        let mut offsets = 0i64;
        for (i, op) in args_ops.iter().enumerate() {
            if let Operand::Copy(l) = op
                && is_managed(self.types, self.program.local(*l).expect("l").ty)
            {
                mask |= 1i64 << i;
            }
            offsets |= ((16 + 8 * i) as i64) << (16 * i as i64);
        }
        let size = 16 + 8 * args_ops.len() as i64;
        let (_, env) = self
            .emit_call_direct(
                IMP_TUPLE_ALLOC,
                &[
                    (Val::I64, size.to_string()),
                    (Val::I64, mask.to_string()),
                    (Val::I64, offsets.to_string()),
                ],
                Some(Val::Ptr),
            )
            .unwrap();
        for (i, op) in args_ops.iter().enumerate() {
            let (vt, vn) = self.owned_value(op)?;
            let raw = self.coerce(&vn, vt, Val::I64, true);
            let gep = self.fresh();
            self.line(format!(
                "{gep} = getelementptr i8, ptr {env}, i64 {}",
                16 + 8 * i
            ));
            self.line(format!("store i64 {raw}, ptr {gep}, align 8"));
        }
        if collectible {
            self.mark_collectible(&env, IMP_TUPLE_TRACE);
        }
        Ok(env)
    }

    fn materialize_function_value(&mut self, fn_id: MirFunctionId) -> Result<String, CodegenError> {
        let f = self.program.function(fn_id).expect("fn");
        let capsule = self.arc_alloc(16, IMP_CLOSURE_DESTROY);
        let (code_ptr, env_ptr) = match f.kind {
            MirFunctionKind::Async => {
                let thunk = format!("@__lpp_task_thunk{}", fn_id.raw());
                let (_, env) = self
                    .emit_call_direct(
                        IMP_TUPLE_ALLOC,
                        &[
                            (Val::I64, "16".to_string()),
                            (Val::I64, "0".to_string()),
                            (Val::I64, "0".to_string()),
                        ],
                        Some(Val::Ptr),
                    )
                    .unwrap();
                (thunk, env)
            }
            MirFunctionKind::Function => {
                let sym = format!("@{}", llvm_sym(&self.plan.export_names[&fn_id]));
                (sym, "null".to_string())
            }
            MirFunctionKind::Closure => {
                return Err(unsupported(self.mir_id, "closure as function operand"));
            }
        };
        self.line(format!("store ptr {code_ptr}, ptr {capsule}, align 8"));
        let gep = self.fresh();
        self.line(format!("{gep} = getelementptr i8, ptr {capsule}, i64 8"));
        self.line(format!("store ptr {env_ptr}, ptr {gep}, align 8"));
        if self.plan.shared_types.contains(&f.ty) {
            self.mark_collectible(&capsule, IMP_CLOSURE_TRACE);
        }
        Ok(capsule)
    }

    fn lower_make_closure(
        &mut self,
        closure_id: MirFunctionId,
        captures: lpp_mir::ListRange<Operand>,
    ) -> Result<String, CodegenError> {
        let closure_fn = self.program.function(closure_id).expect("fn");
        let cap_args = self.program.operands(captures);
        let zero_params = user_parameters(self.program, closure_fn).is_empty();
        let collectible = self.plan.shared_types.contains(&closure_fn.ty);

        let raw_env = if cap_args.is_empty() {
            if zero_params {
                "null".to_string()
            } else {
                self.arc_alloc_no_dtor(8)
            }
        } else {
            let dtor = format!("lpp_drop_c{}", closure_id.raw());
            let env = self.arc_alloc(8 * cap_args.len(), &dtor);
            if collectible {
                let trace = format!("lpp_trace_c{}", closure_id.raw());
                self.mark_collectible(&env, &trace);
            }
            for (i, op) in cap_args.iter().enumerate() {
                let (vt, vn) = self.owned_value(op)?;
                let raw = self.coerce(&vn, vt, Val::I64, true);
                let gep = self.fresh();
                self.line(format!(
                    "{gep} = getelementptr i8, ptr {env}, i64 {}",
                    8 * i
                ));
                self.line(format!("store i64 {raw}, ptr {gep}, align 8"));
            }
            env
        };

        let capsule = self.arc_alloc(16, IMP_CLOSURE_DESTROY);
        if collectible {
            self.mark_collectible(&capsule, IMP_CLOSURE_TRACE);
        }
        let code_sym = if zero_params {
            format!("@__lpp_closure_thunk{}", closure_id.raw())
        } else {
            format!("@{}", llvm_sym(&self.plan.export_names[&closure_id]))
        };
        self.line(format!("store ptr {code_sym}, ptr {capsule}, align 8"));

        let stored_env = if zero_params {
            let (_, wrapper) = self
                .emit_call_direct(
                    IMP_TUPLE_ALLOC,
                    &[
                        (Val::I64, "24".to_string()),
                        (Val::I64, "1".to_string()),
                        (Val::I64, "16".to_string()),
                    ],
                    Some(Val::Ptr),
                )
                .unwrap();
            if collectible {
                self.mark_collectible(&wrapper, IMP_TUPLE_TRACE);
            }
            let wgep = self.fresh();
            self.line(format!("{wgep} = getelementptr i8, ptr {wrapper}, i64 16"));
            self.line(format!("store ptr {raw_env}, ptr {wgep}, align 8"));
            wrapper
        } else {
            raw_env
        };
        let gep = self.fresh();
        self.line(format!("{gep} = getelementptr i8, ptr {capsule}, i64 8"));
        self.line(format!("store ptr {stored_env}, ptr {gep}, align 8"));
        Ok(capsule)
    }

    fn lower_call(
        &mut self,
        callee: &Operand,
        args_ops: &[Operand],
    ) -> Result<Option<(Val, String)>, CodegenError> {
        match callee {
            Operand::Function(callee_id) => {
                let callee_fn = self.program.function(*callee_id).expect("fn");
                match callee_fn.kind {
                    MirFunctionKind::Closure => Err(unsupported(self.mir_id, "closure operand")),
                    MirFunctionKind::Async => {
                        let collectible = self.plan.shared_types.contains(&callee_fn.return_type);
                        let env = self.task_env_for_args(args_ops, collectible)?;
                        let thunk = format!("@__lpp_task_thunk{}", callee_id.raw());
                        let managed = Self::task_managed_flag(self.types, callee_fn.return_type);
                        let code_slot = self.fresh();
                        self.line(format!("{code_slot} = bitcast ptr {thunk} to ptr"));
                        let (_, task) = self
                            .emit_call_direct(
                                IMP_TASK_NEW,
                                &[
                                    (Val::Ptr, code_slot),
                                    (Val::Ptr, env),
                                    (Val::I64, managed.to_string()),
                                ],
                                Some(Val::Ptr),
                            )
                            .unwrap();
                        if collectible {
                            self.mark_collectible(&task, IMP_TASK_TRACE);
                        }
                        Ok(Some((Val::Ptr, task)))
                    }
                    MirFunctionKind::Function => {
                        let params = user_parameters(self.program, callee_fn);
                        let mut call_args = Vec::with_capacity(args_ops.len());
                        for (op, &p_local) in args_ops.iter().zip(params.iter()) {
                            let pt = self.program.local(p_local).expect("l").ty;
                            let expected = val_of_type(self.types, pt, self.mir_id)?;
                            let (vt, vn) = self.owned_value(op)?;
                            let signed = is_signed_integer_kind(Some(self.types.kind(pt)));
                            call_args.push((expected, self.coerce(&vn, vt, expected, signed)));
                        }
                        let ret_v = if is_void_type(self.types, callee_fn.return_type) {
                            None
                        } else {
                            Some(val_of_type(self.types, callee_fn.return_type, self.mir_id)?)
                        };
                        let sym = self.plan.export_names[callee_id].clone();
                        Ok(self.emit_call_direct(&sym, &call_args, ret_v))
                    }
                }
            }
            Operand::Copy(local) => {
                let class = self
                    .plan
                    .value_classes
                    .get(&(self.mir_id, *local))
                    .copied()
                    .ok_or_else(|| unsupported(self.mir_id, "unknown fn class"))?;
                let callee_ty = self.program.local(*local).expect("l").ty;
                let TypeKind::Function { parameters, result } = self.types.kind(callee_ty) else {
                    return Err(unsupported(self.mir_id, "non-function callee"));
                };
                let param_tys = self.types.list(parameters).to_vec();
                let (_, cap) = self.operand_value(&Operand::Copy(*local))?;
                let code = self.fresh();
                self.line(format!("{code} = load ptr, ptr {cap}, align 8"));

                match class {
                    ValueClass::Closure => {
                        let gep = self.fresh();
                        let env = self.fresh();
                        self.line(format!("{gep} = getelementptr i8, ptr {cap}, i64 8"));
                        self.line(format!("{env} = load ptr, ptr {gep}, align 8"));
                        let mut user_args = Vec::with_capacity(args_ops.len());
                        for (op, &pt) in args_ops.iter().zip(param_tys.iter()) {
                            let expected = val_of_type(self.types, pt, self.mir_id)?;
                            let (vt, vn) = self.owned_value(op)?;
                            let signed = is_signed_integer_kind(Some(self.types.kind(pt)));
                            let c = self.coerce(&vn, vt, expected, signed);
                            user_args.push(format!("{} {c}", expected.llvm()));
                        }
                        let has_env = self.fresh();
                        let cls_lbl = self.fresh_label("cls_call");
                        let fn_lbl = self.fresh_label("fn_call");
                        let merge_lbl = self.fresh_label("cap_merge");
                        self.line(format!("{has_env} = icmp ne ptr {env}, null"));
                        self.line(format!(
                            "br i1 {has_env}, label %{cls_lbl}, label %{fn_lbl}"
                        ));

                        self.line(format!("{cls_lbl}:"));
                        let cls_res = if param_tys.is_empty() {
                            let boxed = self.fresh();
                            self.line(format!("{boxed} = call i64 {code}(ptr {env})"));
                            emit_unbox_from_i64(
                                &mut self.body,
                                &boxed,
                                result,
                                self.types,
                                &mut || {
                                    let r = format!("%v{}", self.seq);
                                    self.seq += 1;
                                    r
                                },
                            )
                        } else {
                            let mut call_args = vec![format!("ptr {env}")];
                            call_args.extend(user_args.iter().cloned());
                            if is_void_type(self.types, result) {
                                self.line(format!("call void {code}({})", call_args.join(", ")));
                                None
                            } else {
                                let rv = val_of_type(self.types, result, self.mir_id)?;
                                let res = self.fresh();
                                self.line(format!(
                                    "{res} = call {} {code}({})",
                                    rv.llvm(),
                                    call_args.join(", ")
                                ));
                                Some((rv, res))
                            }
                        };
                        self.line(format!("br label %{merge_lbl}"));

                        self.line(format!("{fn_lbl}:"));
                        let fn_res = if is_void_type(self.types, result) {
                            self.line(format!("call void {code}({})", user_args.join(", ")));
                            None
                        } else {
                            let rv = val_of_type(self.types, result, self.mir_id)?;
                            let res = self.fresh();
                            self.line(format!(
                                "{res} = call {} {code}({})",
                                rv.llvm(),
                                user_args.join(", ")
                            ));
                            Some((rv, res))
                        };
                        self.line(format!("br label %{merge_lbl}"));

                        self.line(format!("{merge_lbl}:"));
                        match (cls_res, fn_res) {
                            (Some((rv, cv)), Some((_, fv))) => {
                                let phi = self.fresh();
                                self.line(format!(
                                    "{phi} = phi {} [ {cv}, %{cls_lbl} ], [ {fv}, %{fn_lbl} ]",
                                    rv.llvm()
                                ));
                                Ok(Some((rv, phi)))
                            }
                            _ => Ok(None),
                        }
                    }
                    ValueClass::FunctionValue => {
                        if matches!(self.types.kind(result), TypeKind::Task(_)) {
                            let collectible = self.plan.shared_types.contains(&result);
                            let env = self.task_env_for_args(args_ops, collectible)?;
                            let managed = Self::task_managed_flag(self.types, result);
                            let (_, task) = self
                                .emit_call_direct(
                                    IMP_TASK_NEW,
                                    &[
                                        (Val::Ptr, code),
                                        (Val::Ptr, env),
                                        (Val::I64, managed.to_string()),
                                    ],
                                    Some(Val::Ptr),
                                )
                                .unwrap();
                            if collectible {
                                self.mark_collectible(&task, IMP_TASK_TRACE);
                            }
                            Ok(Some((Val::Ptr, task)))
                        } else {
                            let mut call_args = Vec::with_capacity(args_ops.len());
                            for (op, &pt) in args_ops.iter().zip(param_tys.iter()) {
                                let expected = val_of_type(self.types, pt, self.mir_id)?;
                                let (vt, vn) = self.owned_value(op)?;
                                let signed = is_signed_integer_kind(Some(self.types.kind(pt)));
                                let c = self.coerce(&vn, vt, expected, signed);
                                call_args.push(format!("{} {c}", expected.llvm()));
                            }
                            if is_void_type(self.types, result) {
                                self.line(format!("call void {code}({})", call_args.join(", ")));
                                Ok(None)
                            } else {
                                let rv = val_of_type(self.types, result, self.mir_id)?;
                                let res = self.fresh();
                                self.line(format!(
                                    "{res} = call {} {code}({})",
                                    rv.llvm(),
                                    call_args.join(", ")
                                ));
                                Ok(Some((rv, res)))
                            }
                        }
                    }
                }
            }
            _ => Err(unsupported(self.mir_id, "non-function callee")),
        }
    }

    fn lower_binary(
        &mut self,
        operator: BinaryOperator,
        left: &Operand,
        right: &Operand,
    ) -> Result<(Val, String), CodegenError> {
        let left_kind = operand_kind(self.program, self.types, left);
        let right_kind = operand_kind(self.program, self.types, right);
        let left_is_str = left_kind == Some(TypeKind::Primitive(PrimitiveType::String));

        if operator == BinaryOperator::Add && left_is_str {
            let (lt, ln) = self.operand_value(left)?;
            let lp = self.coerce(&ln, lt, Val::Ptr, false);
            let (rt, rn) = self.operand_value(right)?;
            let rstr = match right_kind {
                Some(TypeKind::Primitive(PrimitiveType::String)) => {
                    self.coerce(&rn, rt, Val::Ptr, false)
                }
                Some(TypeKind::Primitive(PrimitiveType::Float)) => {
                    let rf = self.coerce(&rn, rt, Val::F64, false);
                    self.emit_call_direct(IMP_FLOAT_TO_STR, &[(Val::F64, rf)], Some(Val::Ptr))
                        .unwrap()
                        .1
                }
                Some(TypeKind::Primitive(PrimitiveType::Bool)) => {
                    let rb = self.coerce(&rn, rt, Val::I8, false);
                    self.emit_call_direct(IMP_BOOL_TO_STR, &[(Val::I8, rb)], Some(Val::Ptr))
                        .unwrap()
                        .1
                }
                Some(TypeKind::Primitive(PrimitiveType::Char)) => {
                    let rc = self.coerce(&rn, rt, Val::I64, false);
                    self.emit_call_direct("lpp_chr", &[(Val::I64, rc)], Some(Val::Ptr))
                        .unwrap()
                        .1
                }
                Some(TypeKind::Primitive(
                    PrimitiveType::U8
                    | PrimitiveType::U16
                    | PrimitiveType::U32
                    | PrimitiveType::U64
                    | PrimitiveType::USize,
                )) => {
                    let ru = self.coerce(&rn, rt, Val::I64, false);
                    self.emit_call_direct("lpp_u64_to_str", &[(Val::I64, ru)], Some(Val::Ptr))
                        .unwrap()
                        .1
                }
                _ => {
                    let ri = self.coerce(&rn, rt, Val::I64, true);
                    self.emit_call_direct(IMP_INT_TO_STR, &[(Val::I64, ri)], Some(Val::Ptr))
                        .unwrap()
                        .1
                }
            };
            return Ok(self
                .emit_call_direct(
                    IMP_STR_CONCAT,
                    &[(Val::Ptr, lp), (Val::Ptr, rstr)],
                    Some(Val::Ptr),
                )
                .unwrap());
        }

        let (lt, ln) = self.operand_value(left)?;
        let (rt, rn) = self.operand_value(right)?;

        if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual) && left_is_str {
            let lp = self.coerce(&ln, lt, Val::Ptr, false);
            let rp = self.coerce(&rn, rt, Val::Ptr, false);
            let (_, eq64) = self
                .emit_call_direct(
                    IMP_STR_EQ,
                    &[(Val::Ptr, lp), (Val::Ptr, rp)],
                    Some(Val::I64),
                )
                .unwrap();
            let trunc = self.fresh();
            self.line(format!("{trunc} = trunc i64 {eq64} to i8"));
            if operator == BinaryOperator::NotEqual {
                let inv = self.fresh();
                self.line(format!("{inv} = xor i8 {trunc}, 1"));
                return Ok((Val::I8, inv));
            }
            return Ok((Val::I8, trunc));
        }

        let (ty, l, r, signed) = if lt == Val::Ptr && rt == Val::Ptr {
            (Val::Ptr, ln, rn, false)
        } else if lt == Val::Ptr || rt == Val::Ptr {
            let li = self.coerce(&ln, lt, Val::I64, is_signed_integer_kind(left_kind));
            let ri = self.coerce(&rn, rt, Val::I64, is_signed_integer_kind(right_kind));
            (Val::I64, li, ri, false)
        } else if lt == Val::F64 || rt == Val::F64 {
            (Val::F64, ln, rn, false)
        } else {
            let target = if lt.bit_width() >= rt.bit_width() {
                lt
            } else {
                rt
            };
            let ls = is_signed_integer_kind(left_kind);
            let rs = is_signed_integer_kind(right_kind);
            let l = self.coerce(&ln, lt, target, ls);
            let r = self.coerce(&rn, rt, target, rs);
            (target, l, r, ls && rs)
        };

        if ty == Val::F64 {
            let out = match operator {
                BinaryOperator::Add => {
                    let s = self.fresh();
                    self.line(format!("{s} = fadd double {l}, {r}"));
                    (Val::F64, s)
                }
                BinaryOperator::Subtract => {
                    let s = self.fresh();
                    self.line(format!("{s} = fsub double {l}, {r}"));
                    (Val::F64, s)
                }
                BinaryOperator::Multiply => {
                    let s = self.fresh();
                    self.line(format!("{s} = fmul double {l}, {r}"));
                    (Val::F64, s)
                }
                BinaryOperator::Divide => {
                    let s = self.fresh();
                    self.line(format!("{s} = fdiv double {l}, {r}"));
                    (Val::F64, s)
                }
                BinaryOperator::Modulo => {
                    let s = self.fresh();
                    self.line(format!("{s} = frem double {l}, {r}"));
                    (Val::F64, s)
                }
                BinaryOperator::Power => self
                    .emit_call_direct("lpp_pow", &[(Val::F64, l), (Val::F64, r)], Some(Val::F64))
                    .unwrap(),
                BinaryOperator::Equal
                | BinaryOperator::NotEqual
                | BinaryOperator::Less
                | BinaryOperator::Greater
                | BinaryOperator::LessEqual
                | BinaryOperator::GreaterEqual => {
                    let cc = match operator {
                        BinaryOperator::Equal => "oeq",
                        BinaryOperator::NotEqual => "une",
                        BinaryOperator::Less => "olt",
                        BinaryOperator::Greater => "ogt",
                        BinaryOperator::LessEqual => "ole",
                        _ => "oge",
                    };
                    let cmp = self.fresh();
                    let b = self.fresh();
                    self.line(format!("{cmp} = fcmp {cc} double {l}, {r}"));
                    self.line(format!("{b} = zext i1 {cmp} to i8"));
                    (Val::I8, b)
                }
                _ => return Err(unsupported(self.mir_id, "invalid float binary op")),
            };
            return Ok(out);
        }

        if matches!(operator, BinaryOperator::Divide | BinaryOperator::Modulo) {
            return Ok(self.emit_int_divmod(
                ty,
                operator == BinaryOperator::Divide,
                signed,
                &l,
                &r,
            ));
        }

        let ty_str = ty.llvm();
        let out = match operator {
            BinaryOperator::Add => {
                let s = self.fresh();
                self.line(format!("{s} = add {ty_str} {l}, {r}"));
                (ty, s)
            }
            BinaryOperator::Subtract => {
                let s = self.fresh();
                self.line(format!("{s} = sub {ty_str} {l}, {r}"));
                (ty, s)
            }
            BinaryOperator::Multiply => {
                let s = self.fresh();
                self.line(format!("{s} = mul {ty_str} {l}, {r}"));
                (ty, s)
            }
            BinaryOperator::Power => {
                let l64 = self.coerce(&l, ty, Val::I64, signed);
                let r64 = self.coerce(&r, ty, Val::I64, signed);
                let (_, raw) = self
                    .emit_call_direct(
                        "lpp_int_pow",
                        &[(Val::I64, l64), (Val::I64, r64)],
                        Some(Val::I64),
                    )
                    .unwrap();
                let res = self.coerce(&raw, Val::I64, ty, signed);
                (ty, res)
            }
            BinaryOperator::BitAnd | BinaryOperator::LogicalAnd => {
                let s = self.fresh();
                self.line(format!("{s} = and {ty_str} {l}, {r}"));
                (ty, s)
            }
            BinaryOperator::BitOr | BinaryOperator::LogicalOr => {
                let s = self.fresh();
                self.line(format!("{s} = or {ty_str} {l}, {r}"));
                (ty, s)
            }
            BinaryOperator::BitXor => {
                let s = self.fresh();
                self.line(format!("{s} = xor {ty_str} {l}, {r}"));
                (ty, s)
            }
            BinaryOperator::ShiftLeft => {
                let s = self.fresh();
                self.line(format!("{s} = shl {ty_str} {l}, {r}"));
                (ty, s)
            }
            BinaryOperator::ShiftRight => {
                let s = self.fresh();
                let op = if signed { "ashr" } else { "lshr" };
                self.line(format!("{s} = {op} {ty_str} {l}, {r}"));
                (ty, s)
            }
            BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::Less
            | BinaryOperator::Greater
            | BinaryOperator::LessEqual
            | BinaryOperator::GreaterEqual => {
                let cc = match (operator, signed) {
                    (BinaryOperator::Equal, _) => "eq",
                    (BinaryOperator::NotEqual, _) => "ne",
                    (BinaryOperator::Less, true) => "slt",
                    (BinaryOperator::Less, false) => "ult",
                    (BinaryOperator::Greater, true) => "sgt",
                    (BinaryOperator::Greater, false) => "ugt",
                    (BinaryOperator::LessEqual, true) => "sle",
                    (BinaryOperator::LessEqual, false) => "ule",
                    (BinaryOperator::GreaterEqual, true) => "sge",
                    (BinaryOperator::GreaterEqual, false) => "uge",
                    _ => "eq",
                };
                let cmp = self.fresh();
                let b = self.fresh();
                self.line(format!("{cmp} = icmp {cc} {ty_str} {l}, {r}"));
                self.line(format!("{b} = zext i1 {cmp} to i8"));
                (Val::I8, b)
            }
            BinaryOperator::Divide | BinaryOperator::Modulo => unreachable!(),
        };
        Ok(out)
    }

    fn emit_int_divmod(
        &mut self,
        ty: Val,
        divide: bool,
        signed: bool,
        l: &str,
        r: &str,
    ) -> (Val, String) {
        let ty_str = ty.llvm();
        let dz = self.fresh();
        let panic_lbl = self.fresh_label("div_zero");
        let ok_lbl = self.fresh_label("div_ok");
        let merge_lbl = self.fresh_label("div_done");
        self.line(format!("{dz} = icmp ne {ty_str} {r}, 0"));
        self.line(format!("br i1 {dz}, label %{ok_lbl}, label %{panic_lbl}"));

        self.start_label(&panic_lbl);
        self.line(format!("call void @{IMP_PANIC_DIV_ZERO}()"));
        self.line("unreachable");

        self.start_label(&ok_lbl);
        let q = self.fresh();
        if signed {
            let min_val: i64 = match ty {
                Val::I8 => i64::from(i8::MIN),
                Val::I16 => i64::from(i16::MIN),
                Val::I32 => i64::from(i32::MIN),
                _ => i64::MIN,
            };
            let is_min = self.fresh();
            let is_neg1 = self.fresh();
            let both = self.fresh();
            let safe_r = self.fresh();
            self.line(format!("{is_min} = icmp eq {ty_str} {l}, {min_val}"));
            self.line(format!("{is_neg1} = icmp eq {ty_str} {r}, -1"));
            self.line(format!("{both} = and i1 {is_min}, {is_neg1}"));
            self.line(format!(
                "{safe_r} = select i1 {both}, {ty_str} 1, {ty_str} {r}"
            ));
            let op = if divide { "sdiv" } else { "srem" };
            self.line(format!("{q} = {op} {ty_str} {l}, {safe_r}"));
        } else {
            let op = if divide { "udiv" } else { "urem" };
            self.line(format!("{q} = {op} {ty_str} {l}, {r}"));
        }
        self.line(format!("br label %{merge_lbl}"));
        self.start_label(&merge_lbl);
        (ty, q)
    }

    fn lower_terminator(&mut self, terminator: &Terminator) -> Result<(), CodegenError> {
        match terminator {
            Terminator::Goto(target) => {
                self.line(format!("br label %bb{}", target.raw()));
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let (ct, cn) = self.operand_value(condition)?;
                let cmp = self.fresh();
                if ct == Val::Ptr {
                    self.line(format!("{cmp} = icmp ne ptr {cn}, null"));
                } else {
                    self.line(format!("{cmp} = icmp ne {} {cn}, 0", ct.llvm()));
                }
                self.line(format!(
                    "br i1 {cmp}, label %bb{}, label %bb{}",
                    then_block.raw(),
                    else_block.raw()
                ));
            }
            Terminator::SwitchEnum {
                subject, targets, ..
            } => {
                let (st, sn) = self.operand_value(subject)?;
                let sp = self.coerce(&sn, st, Val::Ptr, false);
                let tag = self.fresh();
                self.line(format!("{tag} = load i64, ptr {sp}, align 8"));
                let blocks: Vec<BasicBlockId> = self.program.switch_targets(*targets).to_vec();
                let mut cases = String::new();
                for (ord, b) in blocks.iter().enumerate().skip(1) {
                    let _ = write!(cases, " i64 {ord}, label %bb{}", b.raw());
                }
                self.line(format!(
                    "switch i64 {tag}, label %bb{} [{cases} ]",
                    blocks[0].raw()
                ));
            }
            Terminator::Return(operand) => {
                let ret_val = if let Some(op) = operand
                    && !is_void_type(self.types, self.function.return_type)
                {
                    let (vt, vn) = self.operand_value(op)?;
                    let expected = val_of_type(self.types, self.function.return_type, self.mir_id)?;
                    let signed =
                        is_signed_integer_kind(Some(self.types.kind(self.function.return_type)));
                    let coerced = self.coerce(&vn, vt, expected, signed);
                    if is_managed(self.types, self.function.return_type) {
                        self.arc_retain(&coerced);
                    }
                    Some((expected, coerced))
                } else {
                    None
                };

                for &local in self.program.function_locals(self.function) {
                    let desc = self.program.local(local).expect("l");
                    if matches!(desc.kind, MirLocalKind::Capture) {
                        continue;
                    }
                    if self.plan.stack_locals.contains_key(&(self.mir_id, local)) {
                        continue;
                    }
                    if is_managed(self.types, desc.ty)
                        && let Some((slot, _)) = self.local_slots.get(&local).cloned()
                    {
                        let old = self.fresh();
                        self.line(format!("{old} = load ptr, ptr {slot}, align 8"));
                        self.arc_release(&old);
                    }
                }

                match ret_val {
                    Some((vt, vn)) => self.line(format!("ret {} {vn}", vt.llvm())),
                    None => self.line("ret void"),
                }
            }
            Terminator::Unreachable => self.line("unreachable"),
        }
        self.terminated = true;
        Ok(())
    }
}
