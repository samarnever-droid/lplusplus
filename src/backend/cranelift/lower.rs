use super::types::{abi_to_cl, type_to_cl};
use crate::layout::{struct_layout, tuple_layout, tuple_runtime_metadata};
use crate::ast::BinaryOperator;
use crate::mir::ir::*;
use crate::type_facts::ListElementClass;
use crate::types::{TypeRef, TypeTable};
use cranelift_codegen::entity::EntityRef;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types as cl_types;
use cranelift_codegen::ir::{
    AbiParam, InstBuilder, MemFlags, StackSlotData, StackSlotKind, Value,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, FuncId as CLFuncId, Linkage, Module};
use std::collections::HashMap;

pub struct FunctionLower<'a, M: Module> {
    pub module: &'a mut M,
    pub func_ids: &'a HashMap<FuncId, CLFuncId>,
    pub builtin_ids: &'a mut HashMap<String, CLFuncId>,
    /// Generated type-specific destructors used by AllocateArcStruct.
    pub drop_ids: &'a HashMap<crate::types::StructTypeId, CLFuncId>,
    /// One `(ptr env) -> i64` trampoline per async MIR function.
    pub task_thunk_ids: &'a HashMap<FuncId, CLFuncId>,
    pub type_table: &'a TypeTable,
    pub fn_name: String,
    pub next_str_idx: usize,
    /// Emit non-atomic ARC when the whole program is proven single-threaded.
    pub arc_non_atomic: bool,
    /// Locals the escape solver classified `Shared`, i.e. reachable from a
    /// second thread. Only these need atomic refcounts. `None` means the
    /// information is unavailable, which is treated as "assume shared".
    pub shared_locals: Option<&'a std::collections::HashSet<LocalId>>,
}

impl<'a, M: Module> FunctionLower<'a, M> {
    /// Build Cranelift IR for `mir_fn` without compiling it.
    ///
    /// Split out from `lower_function` so the backend can construct IR serially
    /// (it mutates the module: callee references and string-literal data
    /// objects are interned on demand) and then compile the bodies in
    /// parallel, which is the expensive half.
    pub fn build_function_ir(
        &mut self,
        mir_fn: &MirFunction,
    ) -> Result<(CLFuncId, cranelift_codegen::Context), String> {
        let mut sig = self.module.make_signature();
        for param_id in &mir_fn.params {
            let decl = &mir_fn.locals[param_id.0];
            sig.params.push(AbiParam::new(type_to_cl(&decl.ty)));
        }
        if mir_fn.return_type != TypeRef::Void {
            sig.returns
                .push(AbiParam::new(type_to_cl(&mir_fn.return_type)));
        }

        let func_id = *self.func_ids.get(&mir_fn.id).ok_or_else(|| {
            format!(
                "Missing Cranelift function id for MIR function '{}'",
                mir_fn.name
            )
        })?;
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        ctx.func.name = cranelift_codegen::ir::UserFuncName::user(0, func_id.as_u32());

        let mut fn_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);

            let mut local_vars: HashMap<LocalId, Variable> = HashMap::new();
            for (index, local) in mir_fn.locals.iter().enumerate() {
                let variable = Variable::new(index);
                builder.declare_var(variable, type_to_cl(&local.ty));
                local_vars.insert(local.id, variable);
            }

            let mut cl_blocks = HashMap::new();
            for block in &mir_fn.blocks {
                cl_blocks.insert(block.id, builder.create_block());
            }

            let entry_block_id = mir_fn
                .blocks
                .first()
                .map(|block| block.id)
                .ok_or_else(|| format!("MIR function '{}' has no blocks", mir_fn.name))?;
            let entry_block = *cl_blocks
                .get(&entry_block_id)
                .ok_or_else(|| format!("Missing Cranelift entry block for '{}'", mir_fn.name))?;
            builder.switch_to_block(entry_block);
            builder.append_block_params_for_function_params(entry_block);
            let param_vals: Vec<Value> = builder.block_params(entry_block).to_vec();
            for (index, param_id) in mir_fn.params.iter().enumerate() {
                let variable = *local_vars.get(param_id).ok_or_else(|| {
                    format!("Missing Cranelift variable for parameter {:?}", param_id)
                })?;
                builder.def_var(variable, param_vals[index]);
            }
            let cl_i64_zero = builder.ins().iconst(cl_types::I64, 0);
            let cl_i8_zero = builder.ins().iconst(cl_types::I8, 0);
            let cl_f64_zero = builder.ins().f64const(0.0);
            let cl_i64x2_zero = builder.ins().splat(cl_types::I64X2, cl_i64_zero);
            for local in &mir_fn.locals {
                if !mir_fn.params.contains(&local.id) && local.ty != TypeRef::Void {
                    let variable = *local_vars.get(&local.id).unwrap();
                    let cl_ty = type_to_cl(&local.ty);
                    let zero_val = if cl_ty == cl_types::F64 {
                        cl_f64_zero
                    } else if cl_ty == cl_types::I8 {
                        cl_i8_zero
                    } else if cl_ty == cl_types::I64X2 {
                        cl_i64x2_zero
                    } else {
                        cl_i64_zero
                    };
                    builder.def_var(variable, zero_val);
                }
            }

            for (index, block) in mir_fn.blocks.iter().enumerate() {
                let cl_block = *cl_blocks.get(&block.id).ok_or_else(|| {
                    format!(
                        "Missing Cranelift block mapping for block {:?} in '{}'",
                        block.id, mir_fn.name
                    )
                })?;
                if index != 0 {
                    builder.switch_to_block(cl_block);
                }
                for instr in &block.instrs {
                    self.lower_instr_inner(&mut builder, instr, &local_vars, &mir_fn.locals)?;
                }
                self.lower_terminator_inner(
                    &mut builder,
                    &block.terminator,
                    &cl_blocks,
                    &local_vars,
                    &mir_fn.return_type,
                )?;
            }

            builder.seal_all_blocks();
            builder.finalize();
        }

        Ok((func_id, ctx))
    }

    /// Build IR and immediately define the function (serial path).
    pub fn lower_function(&mut self, mir_fn: &MirFunction) -> Result<(), String> {
        let (func_id, mut ctx) = self.build_function_ir(mir_fn)?;
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("define_function '{}': {:?}", mir_fn.name, e))?;
        Ok(())
    }

    fn operand_to_value(
        &mut self,
        builder: &mut FunctionBuilder,
        op: &Operand,
        local_vars: &HashMap<LocalId, Variable>,
    ) -> Result<Value, String> {
        match op {
            Operand::Local(id) | Operand::Borrowed(id) => {
                let variable = *local_vars
                    .get(id)
                    .ok_or_else(|| format!("Missing Cranelift variable for local {:?}", id))?;
                Ok(builder.use_var(variable))
            }
            Operand::Int(value) => Ok(builder.ins().iconst(cl_types::I64, *value)),
            Operand::Float(value) => Ok(builder.ins().f64const(*value)),
            Operand::Bool(value) => Ok(builder
                .ins()
                .iconst(cl_types::I8, if *value { 1 } else { 0 })),
            Operand::String(value) => {
                let symbol_name = format!("str_lit_{}_{}", self.fn_name, self.next_str_idx);
                self.next_str_idx += 1;

                let data_id = self
                    .module
                    .declare_data(&symbol_name, Linkage::Export, false, false)
                    .map_err(|e| format!("declare_data '{}': {:?}", symbol_name, e))?;

                // A string literal is emitted with a real 24-byte ARC header in
                // front of its bytes, and the pointer handed to generated code
                // points *past* that header -- exactly like a heap string from
                // `lpp_arc_alloc`.
                //
                // This is what lets a `Str` local be owned at all. Without a
                // header, `release` on a literal reads 24 bytes in front of
                // .rodata and decrements whatever it finds; with one, release
                // reads a well-formed header, sees the immortal sentinel and
                // returns. The alternative -- never owning `Str` -- is what
                // caused the unbounded string leak this replaces.
                //
                // The layout satisfies BOTH runtimes at once, which is required
                // because the same object file links against either:
                //
                //   offset 0..4   LPP_ARC_MAGIC   host: `magic`    | free: `refcount`
                //   offset 4..8   LPP_ARC_MAGIC   host: `refcount` | free: `generation`
                //   offset 8..24  zero            destructor = NULL, map_size = 0
                //
                // The same constant sits in both of the first two words, so it
                // reads as valid magic to the host runtime and as the immortal
                // refcount sentinel to whichever runtime inspects it.
                const LPP_ARC_MAGIC: u32 = 0x4152_4331;
                let mut bytes = Vec::with_capacity(24 + value.len() + 1);
                bytes.extend_from_slice(&LPP_ARC_MAGIC.to_le_bytes());
                bytes.extend_from_slice(&LPP_ARC_MAGIC.to_le_bytes());
                bytes.extend_from_slice(&[0u8; 16]);
                bytes.extend_from_slice(value.as_bytes());
                bytes.push(0);

                let mut data_ctx = DataDescription::new();
                data_ctx.define(bytes.into_boxed_slice());
                // The header must be 8-byte aligned for the runtime's pointer
                // sanity check (`(addr & 7) != 0` is rejected); 16 keeps the
                // payload itself aligned too.
                data_ctx.set_align(16);
                self.module
                    .define_data(data_id, &data_ctx)
                    .map_err(|e| format!("define_data '{}': {:?}", symbol_name, e))?;

                let local_id = self.module.declare_data_in_func(data_id, &mut builder.func);
                let pointer_type = self.module.target_config().pointer_type();
                let base = builder.ins().symbol_value(pointer_type, local_id);
                // Hand out a pointer to the payload, past the header.
                Ok(builder.ins().iadd_imm(base, 24))
            }
        }
    }

    fn lower_instr_inner(
        &mut self,
        builder: &mut FunctionBuilder,
        instr: &MirInstr,
        local_vars: &HashMap<LocalId, Variable>,
        locals: &[LocalDecl],
    ) -> Result<(), String> {
        match instr {
            MirInstr::Assign(dest, rvalue) => {
                let value = self.lower_rvalue_inner(
                    builder,
                    rvalue,
                    local_vars,
                    locals,
                    Some(&locals[dest.0].ty),
                )?;
                let variable = *local_vars.get(dest).ok_or_else(|| {
                    format!(
                        "Missing Cranelift variable for destination local {:?}",
                        dest
                    )
                })?;
                builder.def_var(variable, value);
            }
            MirInstr::AssignField { base, field, value } => {
                let base_variable = *local_vars.get(base).ok_or_else(|| {
                    format!("Missing Cranelift variable for base local {:?}", base)
                })?;
                let base_value = builder.use_var(base_variable);
                let base_ty = &locals[base.0].ty;
                let value_value = self.operand_to_value(builder, value, local_vars)?;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_index) =
                        struct_def.fields.iter().position(|(name, _)| name == field)
                    {
                        let (layout, _) = struct_layout(self.type_table, *struct_id);
                        let field_layout = layout[field_index];
                        if builder.func.dfg.value_type(value_value) != abi_to_cl(field_layout.abi) {
                            return Err(format!(
                                "Type mismatch storing field '{}' of '{}'",
                                field, struct_def.name
                            ));
                        }
                        builder.ins().store(
                            cranelift_codegen::ir::MemFlags::new(),
                            value_value,
                            base_value,
                            field_layout.offset as i32,
                        );
                    } else {
                        return Err(format!(
                            "Field '{}' not found while lowering struct '{}'",
                            field, struct_def.name
                        ));
                    }
                } else {
                    return Err(format!(
                        "Cannot assign field '{}' on non-struct MIR local {:?}",
                        field, base
                    ));
                }
            }
            MirInstr::Retain(local) => {
                // A stack payload has no ARC header and must never reach this
                // path. `pass_arc` only emits Retain for ARC-managed locals.
                if locals[local.0].ownership.is_copy() {
                    return Err(format!("attempted to retain stack local {:?}", local));
                }
                let is_arena = matches!(
                    locals[local.0].ty,
                    TypeRef::Custom(id)
                        if self
                            .type_table
                            .definitions
                            .get(id.0)
                            .map(|definition| definition.is_self_referential)
                            .unwrap_or(false)
                );
                let local_is_shared = self
                    .shared_locals
                    .map(|s| s.contains(local))
                    .unwrap_or(true);
                let use_non_atomic = self.arc_non_atomic || !local_is_shared;
                let symbol = if is_arena {
                    "lpp_arena_retain"
                } else if use_non_atomic {
                    "lpp_arc_retain_local"
                } else {
                    "lpp_arc_retain"
                };
                let builtin_id = *self
                    .builtin_ids
                    .get(symbol)
                    .ok_or_else(|| format!("ARC runtime symbol '{}' was not declared", symbol))?;
                let func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let value = self.operand_to_value(builder, &Operand::Local(*local), local_vars)?;
                builder.ins().call(func_ref, &[value]);
            }
            MirInstr::Release(local) => {
                // `pass_arc` also uses Release as the lifetime-end opcode for a
                // promoted custom struct. In that case the payload is a stack
                // slot, so call the generated destructor directly; asking the
                // runtime to inspect bytes before the slot would be a header
                // violation. The destructor preserves the cycle-breaker's weak
                // field skips and releases each owned child exactly once.
                if locals[local.0].ownership.is_copy() {
                    let value = self.operand_to_value(builder, &Operand::Local(*local), local_vars)?;
                    match &locals[local.0].ty {
                        TypeRef::Custom(struct_id) => {
                            let drop_id = *self.drop_ids.get(struct_id).ok_or_else(|| {
                                format!("missing stack destructor for struct {:?}", struct_id)
                            })?;
                            let drop_ref = self.module.declare_func_in_func(drop_id, builder.func);
                            builder.ins().call(drop_ref, &[value]);
                            return Ok(());
                        }
                        TypeRef::Function => {
                            let destroy_id = *self
                                .builtin_ids
                                .get("lpp_closure_destroy")
                                .ok_or_else(|| "closure destructor was not declared".to_string())?;
                            let destroy_ref =
                                self.module.declare_func_in_func(destroy_id, builder.func);
                            builder.ins().call(destroy_ref, &[value]);
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                let is_arena = matches!(
                    locals[local.0].ty,
                    TypeRef::Custom(id)
                        if self
                            .type_table
                            .definitions
                            .get(id.0)
                            .map(|definition| definition.is_self_referential)
                            .unwrap_or(false)
                );
                let local_is_shared = self
                    .shared_locals
                    .map(|s| s.contains(local))
                    .unwrap_or(true);
                let use_non_atomic = self.arc_non_atomic || !local_is_shared;
                let symbol = if is_arena {
                    "lpp_arena_release_node"
                } else if use_non_atomic {
                    "lpp_arc_release_local"
                } else {
                    "lpp_arc_release"
                };
                let builtin_id = *self
                    .builtin_ids
                    .get(symbol)
                    .ok_or_else(|| format!("ARC runtime symbol '{}' was not declared", symbol))?;
                let func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let value = self.operand_to_value(builder, &Operand::Local(*local), local_vars)?;
                builder.ins().call(func_ref, &[value]);
            }
        }
        Ok(())
    }

    fn vector_from_lanes(
        &mut self,
        builder: &mut FunctionBuilder,
        lanes: [Value; 2],
    ) -> Value {
        let value = builder.ins().splat(cl_types::I64X2, lanes[0]);
        builder.ins().insertlane(value, lanes[1], 1u8)
    }

    fn lower_vector_builtin(
        &mut self,
        builder: &mut FunctionBuilder,
        symbol: &str,
        args: &[Operand],
        local_vars: &HashMap<LocalId, Variable>,
    ) -> Result<Option<Value>, String> {
        let scalar = |builder: &mut FunctionBuilder, op: &Operand, this: &mut Self| {
            this.operand_to_value(builder, op, local_vars)
        };
        let vector = |builder: &mut FunctionBuilder, op: &Operand, this: &mut Self| {
            this.operand_to_value(builder, op, local_vars)
        };
        let value = match symbol {
            "lpp_vec_i64x2" => {
                if args.len() != 2 {
                    return Err("vec_i64x2 requires exactly two lanes".to_string());
                }
                let lanes = [
                    scalar(builder, &args[0], self)?,
                    scalar(builder, &args[1], self)?,
                ];
                Some(self.vector_from_lanes(builder, lanes))
            }
            "lpp_vec_i64x2_splat" => {
                let lane = scalar(builder, args.first().ok_or_else(|| "vector splat needs one argument".to_string())?, self)?;
                Some(self.vector_from_lanes(builder, [lane, lane]))
            }
            "lpp_vec_i64x2_add" | "lpp_vec_i64x2_sub" | "lpp_vec_i64x2_mul"
            | "lpp_vec_i64x2_xor" | "lpp_vec_i64x2_shr" | "lpp_vec_i64x2_shr_var" => {
                let left = vector(builder, args.first().ok_or_else(|| "vector operation is missing its left operand".to_string())?, self)?;
                let right = args.get(1).ok_or_else(|| "vector operation is missing its right operand".to_string())?;
                if symbol == "lpp_vec_i64x2_shr" {
                    let shift = match right {
                        Operand::Int(value) => *value,
                        _ => return Err("vector shift amount must be a constant integer".to_string()),
                    };
                    let mut lanes = [builder.ins().iconst(cl_types::I64, 0); 2];
                    for lane in 0..2u8 {
                        let item = builder.ins().extractlane(left, lane);
                        lanes[lane as usize] = builder.ins().sshr_imm(item, shift);
                    }
                    Some(self.vector_from_lanes(builder, lanes))
                } else if symbol == "lpp_vec_i64x2_shr_var" {
                    let right = vector(builder, right, self)?;
                    let left0 = builder.ins().extractlane(left, 0u8);
                    let left1 = builder.ins().extractlane(left, 1u8);
                    let right0 = builder.ins().extractlane(right, 0u8);
                    let right1 = builder.ins().extractlane(right, 1u8);
                    let lanes = [
                        builder.ins().sshr(left0, right0),
                        builder.ins().sshr(left1, right1),
                    ];
                    Some(self.vector_from_lanes(builder, lanes))
                } else {
                    let right = vector(builder, right, self)?;
                    Some(match symbol {
                        "lpp_vec_i64x2_add" => builder.ins().iadd(left, right),
                        "lpp_vec_i64x2_sub" => builder.ins().isub(left, right),
                        "lpp_vec_i64x2_mul" => builder.ins().imul(left, right),
                        _ => builder.ins().bxor(left, right),
                    })
                }
            }
            "lpp_vec_i64x2_extract" => {
                let value = vector(builder, args.first().ok_or_else(|| "vector extract is missing its vector".to_string())?, self)?;
                let lane = match args.get(1) {
                    Some(Operand::Int(index)) if (0..2).contains(index) => *index as u8,
                    _ => return Err("vector extract lane must be a constant integer 0..3".to_string()),
                };
                Some(builder.ins().extractlane(value, lane))
            }
            "lpp_vec_i64x2_sum" => {
                let value = vector(builder, args.first().ok_or_else(|| "vector sum is missing its vector".to_string())?, self)?;
                let mut result = builder.ins().extractlane(value, 0u8);
                for lane in 1..2u8 {
                    let item = builder.ins().extractlane(value, lane);
                    result = builder.ins().iadd(result, item);
                }
                Some(result)
            }
            _ => None,
        };
        Ok(value)
    }

    fn lower_rvalue_inner(
        &mut self,
        builder: &mut FunctionBuilder,
        rvalue: &Rvalue,
        local_vars: &HashMap<LocalId, Variable>,
        locals: &[LocalDecl],
        dest_ty: Option<&TypeRef>,
    ) -> Result<Value, String> {
        match rvalue {
            Rvalue::AllocateTuple(types, values) => {
                let (layout, total_size) = tuple_layout(types);
                let (managed_mask, packed_offsets) = tuple_runtime_metadata(types);
                let allocator = *self.builtin_ids.get("lpp_tuple_alloc")
                    .ok_or_else(|| "Builtin 'lpp_tuple_alloc' was not declared".to_string())?;
                let allocator_ref = self.module.declare_func_in_func(allocator, builder.func);
                let size = builder.ins().iconst(cl_types::I64, total_size as i64);
                let mask = builder.ins().iconst(cl_types::I64, managed_mask as i64);
                let offsets = builder.ins().iconst(cl_types::I64, packed_offsets as i64);
                let call = builder.ins().call(allocator_ref, &[size, mask, offsets]);
                let tuple = builder.inst_results(call)[0];
                for ((value, field), ty) in values.iter().zip(layout.iter()).zip(types.iter()) {
                    let stored = self.operand_to_value(builder, value, local_vars)?;
                    if builder.func.dfg.value_type(stored) != type_to_cl(ty) {
                        return Err(format!("tuple field type mismatch for {:?}", ty));
                    }
                    builder.ins().store(MemFlags::new(), stored, tuple, field.offset as i32);
                }
                Ok(tuple)
            }
            Rvalue::TupleField(base, index) => {
                let base_id = match base {
                    Operand::Local(id) | Operand::Borrowed(id) => *id,
                    _ => return Err("tuple field base must be a local".to_string()),
                };
                let types = match &locals[base_id.0].ty {
                    TypeRef::Tuple(types) => types,
                    other => return Err(format!("tuple field base has type {:?}", other)),
                };
                let (layout, _) = tuple_layout(types);
                let field = layout.get(*index)
                    .ok_or_else(|| format!("tuple field {} out of range", index))?;
                let tuple = self.operand_to_value(builder, base, local_vars)?;
                Ok(builder.ins().load(abi_to_cl(field.abi), MemFlags::new(), tuple, field.offset as i32))
            }
            Rvalue::MakeSlice { base, start, length, kind } => {
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot, 40, 3,
                ));
                let pointer_type = self.module.target_config().pointer_type();
                let view = builder.ins().stack_addr(pointer_type, slot, 0);
                let base = self.operand_to_value(builder, base, local_vars)?;
                let start = self.operand_to_value(builder, start, local_vars)?;
                let length = self.operand_to_value(builder, length, local_vars)?;
                let kind = builder.ins().iconst(cl_types::I64, *kind as i64);
                let init = *self.builtin_ids.get("lpp_slice_init")
                    .ok_or_else(|| "Builtin 'lpp_slice_init' was not declared".to_string())?;
                let init_ref = self.module.declare_func_in_func(init, builder.func);
                let call = builder.ins().call(init_ref, &[view, base, start, length, kind]);
                Ok(builder.inst_results(call)[0])
            }
            Rvalue::SliceLen(view) => {
                let view = self.operand_to_value(builder, view, local_vars)?;
                let id = *self.builtin_ids.get("lpp_slice_len")
                    .ok_or_else(|| "Builtin 'lpp_slice_len' was not declared".to_string())?;
                let function = self.module.declare_func_in_func(id, builder.func);
                let call = builder.ins().call(function, &[view]);
                Ok(builder.inst_results(call)[0])
            }
            Rvalue::SliceGet(view, index) => {
                let view_id = match view {
                    Operand::Local(id) | Operand::Borrowed(id) => *id,
                    _ => return Err("slice_get view must be a local".to_string()),
                };
                let result_ty = dest_ty.cloned().unwrap_or(TypeRef::Int);
                let symbol = match (&locals[view_id.0].ty, &result_ty) {
                    (TypeRef::StrSlice, _) => "lpp_str_slice_get",
                    (_, TypeRef::Bool) => "lpp_slice_get_bool",
                    (_, TypeRef::Float) => "lpp_slice_get_float",
                    _ => "lpp_slice_get",
                };
                let view = self.operand_to_value(builder, view, local_vars)?;
                let index = self.operand_to_value(builder, index, local_vars)?;
                let id = if let Some(id) = self.builtin_ids.get(symbol).copied() {
                    id
                } else {
                    let mut signature = self.module.make_signature();
                    signature.params.push(AbiParam::new(cl_types::I64));
                    signature.params.push(AbiParam::new(cl_types::I64));
                    signature.returns.push(AbiParam::new(type_to_cl(&result_ty)));
                    let id = self.module.declare_function(symbol, Linkage::Import, &signature)
                        .map_err(|error| format!("declare slice builtin '{}': {:?}", symbol, error))?;
                    self.builtin_ids.insert(symbol.to_string(), id);
                    id
                };
                let function = self.module.declare_func_in_func(id, builder.func);
                let call = builder.ins().call(function, &[view, index]);
                Ok(builder.inst_results(call)[0])
            }
            Rvalue::SliceToStr(view) => {
                let view = self.operand_to_value(builder, view, local_vars)?;
                let id = *self.builtin_ids.get("lpp_str_slice_to_str")
                    .ok_or_else(|| "Builtin 'lpp_str_slice_to_str' was not declared".to_string())?;
                let function = self.module.declare_func_in_func(id, builder.func);
                let call = builder.ins().call(function, &[view]);
                Ok(builder.inst_results(call)[0])
            }
            Rvalue::MakeTask(function_id, argument_types, arguments, result_type) => {
                let (layout, total_size) = tuple_layout(argument_types);
                let (managed_mask, packed_offsets) = tuple_runtime_metadata(argument_types);
                let allocator = *self.builtin_ids.get("lpp_tuple_alloc")
                    .ok_or_else(|| "Builtin 'lpp_tuple_alloc' was not declared".to_string())?;
                let allocator_ref = self.module.declare_func_in_func(allocator, builder.func);
                let size = builder.ins().iconst(cl_types::I64, total_size as i64);
                let mask = builder.ins().iconst(cl_types::I64, managed_mask as i64);
                let offsets = builder.ins().iconst(cl_types::I64, packed_offsets as i64);
                let allocation = builder.ins().call(allocator_ref, &[size, mask, offsets]);
                let environment = builder.inst_results(allocation)[0];
                for ((argument, field), ty) in
                    arguments.iter().zip(layout.iter()).zip(argument_types.iter())
                {
                    let value = self.operand_to_value(builder, argument, local_vars)?;
                    if builder.func.dfg.value_type(value) != type_to_cl(ty) {
                        return Err(format!("task argument type mismatch for {:?}", ty));
                    }
                    builder.ins().store(MemFlags::new(), value, environment, field.offset as i32);
                }
                let thunk_id = *self.task_thunk_ids.get(function_id)
                    .ok_or_else(|| format!("missing task thunk for fn_{}", function_id.0))?;
                let thunk_ref = self.module.declare_func_in_func(thunk_id, builder.func);
                let thunk = builder.ins().func_addr(self.module.target_config().pointer_type(), thunk_ref);
                let managed = builder.ins().iconst(cl_types::I64, result_type.is_managed() as i64);
                let new_id = *self.builtin_ids.get("lpp_task_new")
                    .ok_or_else(|| "Builtin 'lpp_task_new' was not declared".to_string())?;
                let new_ref = self.module.declare_func_in_func(new_id, builder.func);
                let call = builder.ins().call(new_ref, &[thunk, environment, managed]);
                Ok(builder.inst_results(call)[0])
            }
            Rvalue::Await(task) => {
                let task = self.operand_to_value(builder, task, local_vars)?;
                let id = *self.builtin_ids.get("lpp_task_await")
                    .ok_or_else(|| "Builtin 'lpp_task_await' was not declared".to_string())?;
                let function = self.module.declare_func_in_func(id, builder.func);
                let call = builder.ins().call(function, &[task]);
                let raw = builder.inst_results(call)[0];
                match dest_ty.cloned().unwrap_or(TypeRef::Int) {
                    TypeRef::Bool => Ok(builder.ins().ireduce(cl_types::I8, raw)),
                    TypeRef::Float => {
                        let slot = builder.create_sized_stack_slot(StackSlotData::new(
                            StackSlotKind::ExplicitSlot, 8, 3,
                        ));
                        let pointer_type = self.module.target_config().pointer_type();
                        let address = builder.ins().stack_addr(pointer_type, slot, 0);
                        builder.ins().store(MemFlags::trusted(), raw, address, 0);
                        Ok(builder.ins().load(cl_types::F64, MemFlags::trusted(), address, 0))
                    }
                    _ => Ok(raw),
                }
            }
            Rvalue::Use(op) => self.operand_to_value(builder, op, local_vars),
            Rvalue::Move(local) => {
                self.operand_to_value(builder, &Operand::Local(*local), local_vars)
            }
            Rvalue::BinaryOp(op, left, right) => {
                let left = self.operand_to_value(builder, left, local_vars)?;
                let right = self.operand_to_value(builder, right, local_vars)?;
                let is_float = builder.func.dfg.value_type(left) == cl_types::F64;
                let is_comparison = matches!(
                    op,
                    BinaryOperator::Eq
                        | BinaryOperator::NotEq
                        | BinaryOperator::Less
                        | BinaryOperator::Greater
                        | BinaryOperator::LessEq
                        | BinaryOperator::GreaterEq
                );
                let value = match op {
                    BinaryOperator::Add => {
                        if is_float {
                            builder.ins().fadd(left, right)
                        } else {
                            builder.ins().iadd(left, right)
                        }
                    }
                    BinaryOperator::Subtract => {
                        if is_float {
                            builder.ins().fsub(left, right)
                        } else {
                            builder.ins().isub(left, right)
                        }
                    }
                    BinaryOperator::Multiply => {
                        if is_float {
                            builder.ins().fmul(left, right)
                        } else {
                            builder.ins().imul(left, right)
                        }
                    }
                    BinaryOperator::Divide => {
                        if is_float {
                            builder.ins().fdiv(left, right)
                        } else {
                            builder
                                .ins()
                                .trapz(right, cranelift_codegen::ir::TrapCode::unwrap_user(1));
                            builder.ins().sdiv(left, right)
                        }
                    }
                    BinaryOperator::Modulo => {
                        if is_float {
                            let fmod_id = *self.builtin_ids.get("fmod").ok_or_else(|| {
                                "Builtin 'fmod' was not declared in Cranelift module".to_string()
                            })?;
                            let func_ref = self.module.declare_func_in_func(fmod_id, builder.func);
                            let call = builder.ins().call(func_ref, &[left, right]);
                            let results = builder.inst_results(call);
                            results[0]
                        } else {
                            builder
                                .ins()
                                .trapz(right, cranelift_codegen::ir::TrapCode::unwrap_user(1));
                            builder.ins().srem(left, right)
                        }
                    }
                    BinaryOperator::Eq => {
                        if is_float {
                            builder.ins().fcmp(FloatCC::Equal, left, right)
                        } else {
                            builder.ins().icmp(IntCC::Equal, left, right)
                        }
                    }
                    BinaryOperator::NotEq => {
                        if is_float {
                            builder.ins().fcmp(FloatCC::NotEqual, left, right)
                        } else {
                            builder.ins().icmp(IntCC::NotEqual, left, right)
                        }
                    }
                    BinaryOperator::Less => {
                        if is_float {
                            builder.ins().fcmp(FloatCC::LessThan, left, right)
                        } else {
                            builder.ins().icmp(IntCC::SignedLessThan, left, right)
                        }
                    }
                    BinaryOperator::Greater => {
                        if is_float {
                            builder.ins().fcmp(FloatCC::GreaterThan, left, right)
                        } else {
                            builder.ins().icmp(IntCC::SignedGreaterThan, left, right)
                        }
                    }
                    BinaryOperator::LessEq => {
                        if is_float {
                            builder.ins().fcmp(FloatCC::LessThanOrEqual, left, right)
                        } else {
                            builder
                                .ins()
                                .icmp(IntCC::SignedLessThanOrEqual, left, right)
                        }
                    }
                    BinaryOperator::GreaterEq => {
                        if is_float {
                            builder.ins().fcmp(FloatCC::GreaterThanOrEqual, left, right)
                        } else {
                            builder
                                .ins()
                                .icmp(IntCC::SignedGreaterThanOrEqual, left, right)
                        }
                    }
                    BinaryOperator::And => {
                        builder.ins().band(left, right)
                    }
                    BinaryOperator::Or => {
                        builder.ins().bor(left, right)
                    }
                    BinaryOperator::BitAnd => {
                        builder.ins().band(left, right)
                    }
                    BinaryOperator::BitOr => {
                        builder.ins().bor(left, right)
                    }
                    BinaryOperator::BitXor => {
                        builder.ins().bxor(left, right)
                    }
                    BinaryOperator::Shl => {
                        let zero = builder.ins().iconst(cl_types::I64, 0);
                        let in_bounds = builder.ins().icmp_imm(IntCC::UnsignedLessThan, right, 64);
                        let shifted = builder.ins().ishl(left, right);
                        builder.ins().select(in_bounds, shifted, zero)
                    }
                    BinaryOperator::Shr => {
                        let zero = builder.ins().iconst(cl_types::I64, 0);
                        let max_shift = builder.ins().iconst(cl_types::I64, 63);
                        let not_negative = builder.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, right, 0);
                        let lt_64 = builder.ins().icmp_imm(IntCC::SignedLessThan, right, 64);
                        let clamped_shift = builder.ins().select(lt_64, right, max_shift);
                        let shifted = builder.ins().sshr(left, clamped_shift);
                        builder.ins().select(not_negative, shifted, zero)
                    }
                };
                // Cranelift 0.113 represents integer/float comparisons as I8,
                // which is also L++'s stable Bool ABI. Keep the value unchanged;
                // extending it as if it were b1 creates invalid CLIF.
                let _ = is_comparison;
                Ok(value)
            }
            Rvalue::CallDirect(mir_func_id, args) => {
                let cl_id = *self.func_ids.get(mir_func_id).ok_or_else(|| {
                    format!(
                        "Missing direct-call target for MIR function id {:?}",
                        mir_func_id
                    )
                })?;
                let func_ref = self.module.declare_func_in_func(cl_id, builder.func);
                let arg_values: Vec<Value> = args
                    .iter()
                    .map(|arg| self.operand_to_value(builder, arg, local_vars))
                    .collect::<Result<_, _>>()?;
                let call = builder.ins().call(func_ref, &arg_values);
                let results = builder.inst_results(call);
                Ok(if results.is_empty() {
                    builder.ins().iconst(cl_types::I64, 0)
                } else {
                    results[0]
                })
            }
            Rvalue::BuiltinCall(symbol, args) => {
                if let Some(value) = self.lower_vector_builtin(builder, symbol, args, local_vars)? {
                    return Ok(value);
                }
                // Look up known builtins first; auto-declare unknown symbols as FFI imports
                let cl_id = if let Some(&id) = self.builtin_ids.get(symbol) {
                    id
                } else {
                    // Auto-declare as FFI import: all params and return are i64 (C ABI)
                    let mut sig = self.module.make_signature();
                    for _ in args {
                        sig.params.push(AbiParam::new(cl_types::I64));
                    }
                    // Assume i64 return (covers pointers, ints, handles)
                    let ret_cl = dest_ty
                        .map(|t| super::types::type_to_cl(t))
                        .unwrap_or(cl_types::I64);
                    if dest_ty.map_or(true, |t| *t != TypeRef::Void) {
                        sig.returns.push(AbiParam::new(ret_cl));
                    }
                    let id = self.module
                        .declare_function(symbol, Linkage::Import, &sig)
                        .map_err(|e| format!("declare FFI '{}': {:?}", symbol, e))?;
                    self.builtin_ids.insert(symbol.clone(), id);
                    id
                };
                let func_ref = self.module.declare_func_in_func(cl_id, builder.func);
                let arg_values: Vec<Value> = args
                    .iter()
                    .map(|arg| self.operand_to_value(builder, arg, local_vars))
                    .collect::<Result<_, _>>()?;
                let call = builder.ins().call(func_ref, &arg_values);
                let results = builder.inst_results(call);
                Ok(if results.is_empty() {
                    builder.ins().iconst(cl_types::I64, 0)
                } else {
                    results[0]
                })
            }
            Rvalue::AllocateArcStruct(TypeRef::Custom(struct_id)) => {
                let (_, layout_size) = struct_layout(self.type_table, *struct_id);
                let size_val = builder.ins().iconst(cl_types::I64, layout_size as i64);
                let builtin_id = *self
                    .builtin_ids
                    .get("lpp_arc_alloc_with_destructor")
                    .ok_or_else(|| {
                        "Builtin 'lpp_arc_alloc_with_destructor' was not declared".to_string()
                    })?;
                let func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let drop_id = *self.drop_ids.get(struct_id).ok_or_else(|| {
                    format!(
                        "missing generated ARC destructor for struct {:?}",
                        struct_id
                    )
                })?;
                let drop_ref = self.module.declare_func_in_func(drop_id, builder.func);
                let drop_addr = builder
                    .ins()
                    .func_addr(self.module.target_config().pointer_type(), drop_ref);
                let call = builder.ins().call(func_ref, &[size_val, drop_addr]);
                let results = builder.inst_results(call);
                results
                    .first()
                    .copied()
                    .ok_or_else(|| "Allocator call returned no value".to_string())
            }
            Rvalue::AllocateArenaStruct(TypeRef::Custom(struct_id), arena) => {
                let (_, layout_size) = struct_layout(self.type_table, *struct_id);
                let size_val = builder.ins().iconst(cl_types::I64, layout_size as i64);
                let arena_id = *self
                    .builtin_ids
                    .get("lpp_arena_alloc")
                    .ok_or_else(|| "Builtin 'lpp_arena_alloc' was not declared".to_string())?;
                let arena_ref = self.module.declare_func_in_func(arena_id, builder.func);
                let drop_id = *self.drop_ids.get(struct_id).ok_or_else(|| {
                    format!("missing generated arena destructor for struct {:?}", struct_id)
                })?;
                let drop_ref = self.module.declare_func_in_func(drop_id, builder.func);
                let drop_addr = builder
                    .ins()
                    .func_addr(self.module.target_config().pointer_type(), drop_ref);
                let arena_value = self.operand_to_value(builder, arena, local_vars)?;
                let call = builder
                    .ins()
                    .call(arena_ref, &[size_val, arena_value, drop_addr]);
                builder
                    .inst_results(call)
                    .first()
                    .copied()
                    .ok_or_else(|| "Arena allocator call returned no value".to_string())
            }
            Rvalue::AllocateArenaStruct(other, _) => Err(format!(
                "arena allocation requires a resolved custom struct type, got {:?}",
                other
            )),
            Rvalue::AllocateStackStruct(TypeRef::Custom(struct_id)) => {
                // A frame-local struct: same payload layout as the heap form,
                // so every field load/store below is unchanged. What is gone is
                // the header, the allocator call and the refcount traffic.
                //
                // Zero-initialised to match `lpp_arc_alloc_with_destructor`,
                // which uses calloc -- reading a field before writing it must
                // behave identically either way.
                let (_, layout_size) = struct_layout(self.type_table, *struct_id);
                let slot_size = layout_size.max(1) as u32;
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    slot_size,
                    3, // 2^3 = 8-byte alignment, matching the heap payload
                ));
                let pointer_type = self.module.target_config().pointer_type();
                let addr = builder.ins().stack_addr(pointer_type, slot, 0);
                let zero = builder.ins().iconst(cl_types::I64, 0);
                let mut offset = 0i32;
                while (offset as u32) < slot_size {
                    builder
                        .ins()
                        .store(MemFlags::trusted(), zero, addr, offset);
                    offset += 8;
                }
                Ok(addr)
            }
            Rvalue::AllocateStackStruct(other) => Err(format!(
                "stack struct allocation requires a custom struct type, got {:?}",
                other
            )),
            Rvalue::AllocateList(element_ty) => {
                let allocator = match element_ty.list_element_class() {
                    ListElementClass::Scalar
                    | ListElementClass::Bool
                    | ListElementClass::Float => "lpp_list_new",
                    ListElementClass::Arc => "lpp_list_new_arc",
                    ListElementClass::Unsupported => {
                        return Err(format!(
                            "AOT does not support List[{:?}] safely",
                            element_ty
                        ));
                    }
                };
                let builtin_id = *self
                    .builtin_ids
                    .get(allocator)
                    .ok_or_else(|| format!("Builtin '{}' was not declared", allocator))?;
                let func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let call = builder.ins().call(func_ref, &[]);
                let results = builder.inst_results(call);
                results
                    .first()
                    .copied()
                    .ok_or_else(|| "List allocator call returned no value".to_string())
            }
            Rvalue::FieldAccess(Operand::Local(base_id), field)
            | Rvalue::FieldAccess(Operand::Borrowed(base_id), field) => {
                let base_value =
                    self.operand_to_value(builder, &Operand::Local(*base_id), local_vars)?;
                let base_ty = &locals[base_id.0].ty;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_index) =
                        struct_def.fields.iter().position(|(name, _)| name == field)
                    {
                        let (layout, _) = struct_layout(self.type_table, *struct_id);
                        let field_layout = layout[field_index];
                        Ok(builder.ins().load(
                            abi_to_cl(field_layout.abi),
                            cranelift_codegen::ir::MemFlags::new(),
                            base_value,
                            field_layout.offset as i32,
                        ))
                    } else {
                        Err(format!(
                            "Field '{}' not found while lowering struct '{}'",
                            field, struct_def.name
                        ))
                    }
                } else {
                    Err(format!(
                        "Cannot read field '{}' on non-struct MIR local {:?}",
                        field, base_id
                    ))
                }
            }
            rv @ (Rvalue::MakeClosure(mir_func_id, args)
            | Rvalue::MakeStackClosure(mir_func_id, args)) => {
                let stack_closure = matches!(rv, Rvalue::MakeStackClosure(_, _));
                let pointer_type = self.module.target_config().pointer_type();
                let closure_ptr = if stack_closure {
                    // A frame-local closure capsule is exactly two words:
                    // [code pointer, environment pointer]. The environment is
                    // still ARC-owned and is released by lpp_closure_destroy.
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        16,
                        3,
                    ));
                    let addr = builder.ins().stack_addr(pointer_type, slot, 0);
                    let zero = builder.ins().iconst(cl_types::I64, 0);
                    builder
                        .ins()
                        .store(MemFlags::trusted(), zero, addr, 0);
                    builder
                        .ins()
                        .store(MemFlags::trusted(), zero, addr, 8);
                    addr
                } else {
                    let size_val = builder.ins().iconst(cl_types::I64, 16);
                    let builtin_id = *self
                        .builtin_ids
                        .get("lpp_arc_alloc_with_destructor")
                        .ok_or_else(|| {
                            "Builtin 'lpp_arc_alloc_with_destructor' was not declared".to_string()
                        })?;
                    let alloc_func_ref =
                        self.module.declare_func_in_func(builtin_id, builder.func);
                    let destroy_id = *self.builtin_ids.get("lpp_closure_destroy").ok_or_else(|| {
                        "Builtin 'lpp_closure_destroy' was not declared".to_string()
                    })?;
                    let destroy_ref = self.module.declare_func_in_func(destroy_id, builder.func);
                    let destroy_addr = builder.ins().func_addr(pointer_type, destroy_ref);
                    let call = builder
                        .ins()
                        .call(alloc_func_ref, &[size_val, destroy_addr]);
                    builder.inst_results(call)[0]
                };

                let cl_id = *self.func_ids.get(mir_func_id).ok_or_else(|| {
                    format!(
                        "Missing direct-call target for MIR function id {:?}",
                        mir_func_id
                    )
                })?;
                let func_ref = self.module.declare_func_in_func(cl_id, builder.func);
                let func_addr = builder.ins().func_addr(pointer_type, func_ref);
                builder.ins().store(
                    cranelift_codegen::ir::MemFlags::new(),
                    func_addr,
                    closure_ptr,
                    0,
                );

                let env_operand = args.first().ok_or_else(|| {
                    "internal error: closure construction is missing its environment".to_string()
                })?;
                let env_val = self.operand_to_value(builder, env_operand, local_vars)?;
                builder.ins().store(
                    cranelift_codegen::ir::MemFlags::new(),
                    env_val,
                    closure_ptr,
                    8,
                );

                Ok(closure_ptr)
            }
            Rvalue::CallIndirect(callee, args) => {
                let callee_val = self.operand_to_value(builder, callee, local_vars)?;
                let pointer_type = self.module.target_config().pointer_type();

                // Check if this is a direct function pointer (trait dispatch)
                // or a closure struct pointer. Trait dispatch uses plain function pointers,
                // closures use a struct with (func_ptr, env_ptr).
                let is_direct_fptr = match callee {
                    Operand::Local(id) | Operand::Borrowed(id) => {
                        // Only Int locals are direct function pointers (from FuncRef/trait vtable).
                        // Function-typed locals are closures (struct with [fptr, env_ptr]).
                        matches!(locals[id.0].ty, TypeRef::Int)
                    }
                    _ => false,
                };

                let (func_ptr, env_ptr_opt) = if is_direct_fptr {
                    // Direct function pointer (from FuncRef / trait vtable)
                    (callee_val, None)
                } else {
                    // Closure struct: load func_ptr and env_ptr
                    let fp = builder.ins().load(
                        pointer_type,
                        cranelift_codegen::ir::MemFlags::new(),
                        callee_val, 0,
                    );
                    let ep = builder.ins().load(
                        pointer_type,
                        cranelift_codegen::ir::MemFlags::new(),
                        callee_val, 8,
                    );
                    (fp, Some(ep))
                };

                let mut sig = self.module.make_signature();
                if env_ptr_opt.is_some() {
                    sig.params.push(AbiParam::new(pointer_type)); // env_ptr for closures
                }
                for arg in args {
                    let arg_ty = match arg {
                        Operand::Local(id) | Operand::Borrowed(id) => locals[id.0].ty.clone(),
                        Operand::Int(_) => TypeRef::Int,
                        Operand::Float(_) => TypeRef::Float,
                        Operand::Bool(_) => TypeRef::Bool,
                        Operand::String(_) => TypeRef::Str,
                    };
                    sig.params
                        .push(AbiParam::new(super::types::type_to_cl(&arg_ty)));
                }

                let ret_ty = dest_ty.cloned().unwrap_or(TypeRef::Void);
                if ret_ty != TypeRef::Void {
                    sig.returns
                        .push(AbiParam::new(super::types::type_to_cl(&ret_ty)));
                }

                let sig_ref = builder.import_signature(sig);
                let mut call_args = Vec::new();
                if let Some(ep) = env_ptr_opt {
                    call_args.push(ep);
                }
                for arg in args {
                    call_args.push(self.operand_to_value(builder, arg, local_vars)?);
                }

                let call = builder.ins().call_indirect(sig_ref, func_ptr, &call_args);
                let results = builder.inst_results(call);
                Ok(if results.is_empty() {
                    builder.ins().iconst(cl_types::I64, 0)
                } else {
                    results[0]
                })
            }
            Rvalue::SpawnThread(closure_op) => {
                let closure_ptr = self.operand_to_value(builder, closure_op, local_vars)?;
                let pointer_type = self.module.target_config().pointer_type();

                let func_ptr = builder.ins().load(
                    pointer_type,
                    cranelift_codegen::ir::MemFlags::new(),
                    closure_ptr,
                    0,
                );

                let env_ptr = builder.ins().load(
                    pointer_type,
                    cranelift_codegen::ir::MemFlags::new(),
                    closure_ptr,
                    8,
                );

                let builtin_id = *self.builtin_ids.get("lpp_thread_spawn").ok_or_else(|| {
                    "Builtin 'lpp_thread_spawn' was not declared in Cranelift module".to_string()
                })?;

                let func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                builder.ins().call(func_ref, &[func_ptr, env_ptr]);

                Ok(builder.ins().iconst(pointer_type, 0))
            }
            Rvalue::AllocateStruct(_) => Err(
                "raw struct allocation reached AOT; use AllocateArcStruct for owned objects"
                    .to_string(),
            ),
            Rvalue::AllocateArcStruct(_) => {
                Err("AllocateArcStruct requires a resolved custom struct type".to_string())
            }
            Rvalue::FieldAccess(_, _) => Ok(builder.ins().iconst(cl_types::I64, 0)),
            Rvalue::FuncRef(mir_func_id) => {
                let func_id = *self.func_ids.get(mir_func_id).ok_or_else(|| {
                    format!("FuncRef: unknown MIR function id fn_{}", mir_func_id.0)
                })?;
                let func_ref = self.module.declare_func_in_func(func_id, builder.func);
                let pointer_type = self.module.target_config().pointer_type();
                Ok(builder.ins().func_addr(pointer_type, func_ref))
            }
        }
    }

    fn lower_terminator_inner(
        &mut self,
        builder: &mut FunctionBuilder,
        terminator: &Terminator,
        cl_blocks: &HashMap<BlockId, cranelift_codegen::ir::Block>,
        local_vars: &HashMap<LocalId, Variable>,
        return_type: &TypeRef,
    ) -> Result<(), String> {
        match terminator {
            Terminator::Goto(target) => {
                let block = *cl_blocks
                    .get(target)
                    .ok_or_else(|| format!("Missing jump target block {:?}", target))?;
                builder.ins().jump(block, &[]);
            }
            Terminator::If {
                cond,
                then_block,
                else_block,
            } => {
                let cond_value = self.operand_to_value(builder, cond, local_vars)?;
                let cond_bool = builder.ins().icmp_imm(IntCC::NotEqual, cond_value, 0);
                let then_block = *cl_blocks
                    .get(then_block)
                    .ok_or_else(|| format!("Missing then-block mapping for {:?}", then_block))?;
                let else_block = *cl_blocks
                    .get(else_block)
                    .ok_or_else(|| format!("Missing else-block mapping for {:?}", else_block))?;
                builder
                    .ins()
                    .brif(cond_bool, then_block, &[], else_block, &[]);
            }
            Terminator::IfCmp {
                op,
                left,
                right,
                then_block,
                else_block,
            } => {
                let left = self.operand_to_value(builder, left, local_vars)?;
                let right = self.operand_to_value(builder, right, local_vars)?;
                let is_float = builder.func.dfg.value_type(left) == cl_types::F64;

                let comparison = if is_float {
                    let float_cc = match op {
                        BinaryOperator::Eq => FloatCC::Equal,
                        BinaryOperator::NotEq => FloatCC::NotEqual,
                        BinaryOperator::Less => FloatCC::LessThan,
                        BinaryOperator::Greater => FloatCC::GreaterThan,
                        BinaryOperator::LessEq => FloatCC::LessThanOrEqual,
                        BinaryOperator::GreaterEq => FloatCC::GreaterThanOrEqual,
                        _ => {
                            return Err(
                                "non-comparison operator reached fused branch lowering".to_string()
                            );
                        }
                    };
                    builder.ins().fcmp(float_cc, left, right)
                } else {
                    let int_cc = match op {
                        BinaryOperator::Eq => IntCC::Equal,
                        BinaryOperator::NotEq => IntCC::NotEqual,
                        BinaryOperator::Less => IntCC::SignedLessThan,
                        BinaryOperator::Greater => IntCC::SignedGreaterThan,
                        BinaryOperator::LessEq => IntCC::SignedLessThanOrEqual,
                        BinaryOperator::GreaterEq => IntCC::SignedGreaterThanOrEqual,
                        _ => {
                            return Err(
                                "non-comparison operator reached fused branch lowering".to_string()
                            );
                        }
                    };
                    builder.ins().icmp(int_cc, left, right)
                };

                let then_block = *cl_blocks
                    .get(then_block)
                    .ok_or_else(|| "missing fused then block".to_string())?;
                let else_block = *cl_blocks
                    .get(else_block)
                    .ok_or_else(|| "missing fused else block".to_string())?;
                builder
                    .ins()
                    .brif(comparison, then_block, &[], else_block, &[]);
            }
            Terminator::Return(Some(op)) | Terminator::ReturnOwned(op) => {
                // ReturnOwned transfers an ARC reference in MIR; its machine ABI is
                // the same return instruction as an ordinary return.
                let value = self.operand_to_value(builder, op, local_vars)?;
                builder.ins().return_(&[value]);
            }
            Terminator::Return(None) => {
                if *return_type == TypeRef::Void {
                    builder.ins().return_(&[]);
                } else {
                    // Keep an implicit/default return ABI-correct. In particular, an
                    // `f64` function cannot return an I64 zero and a Bool is I8.
                    let zero = match return_type {
                        TypeRef::Float => builder.ins().f64const(0.0),
                        TypeRef::Bool => builder.ins().iconst(cl_types::I8, 0),
                        TypeRef::Int
                        | TypeRef::Char
                        | TypeRef::Str
                        | TypeRef::Custom(_)
                        | TypeRef::Generic(_, _)
                        | TypeRef::Unresolved(_)
                        | TypeRef::Function
                        | TypeRef::TypeParam(_)
                        | TypeRef::Tuple(_)
                        | TypeRef::StrSlice
                        | TypeRef::Slice(_)
                        | TypeRef::Task(_)
                        | TypeRef::Void => builder.ins().iconst(cl_types::I64, 0),
                        TypeRef::VectorI64x2 => {
                            let zero = builder.ins().iconst(cl_types::I64, 0);
                            builder.ins().splat(cl_types::I64X2, zero)
                        },
                    };
                    builder.ins().return_(&[zero]);
                }
            }
            Terminator::Unreachable => {
                builder
                    .ins()
                    .trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
            }
        }
        Ok(())
    }
}
