use super::types::{struct_layout, type_to_cl};
use crate::ast::BinaryOperator;
use crate::mir::ir::*;
use crate::typecheck::{TypeRef, TypeTable};
use cranelift_codegen::entity::EntityRef;
use cranelift_codegen::ir::condcodes::{IntCC, FloatCC};
use cranelift_codegen::ir::types as cl_types;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Value};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, FuncId as CLFuncId, Linkage, Module};
use std::collections::HashMap;

pub struct FunctionLower<'a, M: Module> {
    pub module: &'a mut M,
    pub func_ids: &'a HashMap<FuncId, CLFuncId>,
    pub builtin_ids: &'a HashMap<String, CLFuncId>,
    /// Generated type-specific destructors used by AllocateArcStruct.
    pub drop_ids: &'a HashMap<crate::typecheck::StructTypeId, CLFuncId>,
    pub type_table: &'a TypeTable,
    pub fn_name: String,
    pub next_str_idx: usize,
}

impl<'a, M: Module> FunctionLower<'a, M> {
    pub fn lower_function(&mut self, mir_fn: &MirFunction) -> Result<(), String> {
        let mut sig = self.module.make_signature();
        for param_id in &mir_fn.params {
            let decl = &mir_fn.locals[param_id.0];
            sig.params.push(AbiParam::new(type_to_cl(&decl.ty)));
        }
        if mir_fn.return_type != TypeRef::Void {
            sig.returns.push(AbiParam::new(type_to_cl(&mir_fn.return_type)));
        }

        let func_id = *self
            .func_ids
            .get(&mir_fn.id)
            .ok_or_else(|| format!("Missing Cranelift function id for MIR function '{}'", mir_fn.name))?;
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
                let variable = *local_vars
                    .get(param_id)
                    .ok_or_else(|| format!("Missing Cranelift variable for parameter {:?}", param_id))?;
                builder.def_var(variable, param_vals[index]);
            }

            for (index, block) in mir_fn.blocks.iter().enumerate() {
                let cl_block = *cl_blocks.get(&block.id).ok_or_else(|| {
                    format!("Missing Cranelift block mapping for block {:?} in '{}'", block.id, mir_fn.name)
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
            Operand::Bool(value) => Ok(builder.ins().iconst(cl_types::I8, if *value { 1 } else { 0 })),
            Operand::String(value) => {
                let symbol_name = format!("str_lit_{}_{}", self.fn_name, self.next_str_idx);
                self.next_str_idx += 1;

                let data_id = self
                    .module
                    .declare_data(&symbol_name, Linkage::Local, false, false)
                    .map_err(|e| format!("declare_data '{}': {:?}", symbol_name, e))?;

                let mut data_ctx = DataDescription::new();
                let mut bytes = value.as_bytes().to_vec();
                bytes.push(0);
                data_ctx.define(bytes.into_boxed_slice());
                self.module
                    .define_data(data_id, &data_ctx)
                    .map_err(|e| format!("define_data '{}': {:?}", symbol_name, e))?;

                let local_id = self.module.declare_data_in_func(data_id, &mut builder.func);
                let pointer_type = self.module.target_config().pointer_type();
                Ok(builder.ins().symbol_value(pointer_type, local_id))
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
                let value = self.lower_rvalue_inner(builder, rvalue, local_vars, locals, Some(&locals[dest.0].ty))?;
                let variable = *local_vars
                    .get(dest)
                    .ok_or_else(|| format!("Missing Cranelift variable for destination local {:?}", dest))?;
                builder.def_var(variable, value);
            }
            MirInstr::AssignField { base, field, value } => {
                let base_variable = *local_vars
                    .get(base)
                    .ok_or_else(|| format!("Missing Cranelift variable for base local {:?}", base))?;
                let base_value = builder.use_var(base_variable);
                let base_ty = &locals[base.0].ty;
                let value_value = self.operand_to_value(builder, value, local_vars)?;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_index) = struct_def.fields.iter().position(|(name, _)| name == field) {
                        let (layout, _) = struct_layout(self.type_table, *struct_id);
                        let field_layout = layout[field_index];
                        if builder.func.dfg.value_type(value_value) != field_layout.ty {
                            return Err(format!("Type mismatch storing field '{}' of '{}'", field, struct_def.name));
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
            MirInstr::Retain(local) | MirInstr::Release(local) => {
                // ARC operations are part of MIR semantics, not optional hints.  Emit
                // the runtime call so AOT has the same ownership behavior as the model.
                let symbol = if matches!(instr, MirInstr::Retain(_)) {
                    "lpp_arc_retain"
                } else {
                    "lpp_arc_release"
                };
                let builtin_id = *self.builtin_ids.get(symbol).ok_or_else(|| {
                    format!("ARC runtime symbol '{}' was not declared", symbol)
                })?;
                let func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let value = self.operand_to_value(builder, &Operand::Local(*local), local_vars)?;
                builder.ins().call(func_ref, &[value]);
            }
        }
        Ok(())
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
                    BinaryOperator::Eq | BinaryOperator::NotEq | BinaryOperator::Less |
                    BinaryOperator::Greater | BinaryOperator::LessEq | BinaryOperator::GreaterEq
                );
                let value = match op {
                    BinaryOperator::Add => {
                        if is_float { builder.ins().fadd(left, right) }
                        else { builder.ins().iadd(left, right) }
                    }
                    BinaryOperator::Subtract => {
                        if is_float { builder.ins().fsub(left, right) }
                        else { builder.ins().isub(left, right) }
                    }
                    BinaryOperator::Multiply => {
                        if is_float { builder.ins().fmul(left, right) }
                        else { builder.ins().imul(left, right) }
                    }
                    BinaryOperator::Divide => {
                        if is_float { builder.ins().fdiv(left, right) }
                        else { builder.ins().sdiv(left, right) }
                    }
                    // Cranelift has no fmod instruction. Never silently compile `%`
                    // as subtraction: reject float modulo until it has a runtime lowering.
                    BinaryOperator::Modulo => {
                        if is_float {
                            return Err("float modulo is not implemented by the AOT backend".to_string());
                        }
                        builder.ins().srem(left, right)
                    }
                    BinaryOperator::Eq => {
                        if is_float { builder.ins().fcmp(FloatCC::Equal, left, right) }
                        else { builder.ins().icmp(IntCC::Equal, left, right) }
                    }
                    BinaryOperator::NotEq => {
                        if is_float { builder.ins().fcmp(FloatCC::NotEqual, left, right) }
                        else { builder.ins().icmp(IntCC::NotEqual, left, right) }
                    }
                    BinaryOperator::Less => {
                        if is_float { builder.ins().fcmp(FloatCC::LessThan, left, right) }
                        else { builder.ins().icmp(IntCC::SignedLessThan, left, right) }
                    }
                    BinaryOperator::Greater => {
                        if is_float { builder.ins().fcmp(FloatCC::GreaterThan, left, right) }
                        else { builder.ins().icmp(IntCC::SignedGreaterThan, left, right) }
                    }
                    BinaryOperator::LessEq => {
                        if is_float { builder.ins().fcmp(FloatCC::LessThanOrEqual, left, right) }
                        else { builder.ins().icmp(IntCC::SignedLessThanOrEqual, left, right) }
                    }
                    BinaryOperator::GreaterEq => {
                        if is_float { builder.ins().fcmp(FloatCC::GreaterThanOrEqual, left, right) }
                        else { builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, left, right) }
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
                    format!("Missing direct-call target for MIR function id {:?}", mir_func_id)
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
                let cl_id = *self
                    .builtin_ids
                    .get(symbol)
                    .ok_or_else(|| format!("Builtin '{}' was not declared in the Cranelift module", symbol))?;
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
                    .ok_or_else(|| "Builtin 'lpp_arc_alloc_with_destructor' was not declared".to_string())?;
                let func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let drop_id = *self.drop_ids.get(struct_id).ok_or_else(|| {
                    format!("missing generated ARC destructor for struct {:?}", struct_id)
                })?;
                let drop_ref = self.module.declare_func_in_func(drop_id, builder.func);
                let drop_addr = builder.ins().func_addr(self.module.target_config().pointer_type(), drop_ref);
                let call = builder.ins().call(func_ref, &[size_val, drop_addr]);
                let results = builder.inst_results(call);
                results
                    .first()
                    .copied()
                    .ok_or_else(|| "Allocator call returned no value".to_string())
            }
            Rvalue::AllocateList(element_ty) => {
                // The current runtime stores int64_t elements, not generic boxed values.
                // Refuse unsupported lists in AOT instead of truncating pointers/floats.
                if *element_ty != TypeRef::Int {
                    return Err(format!(
                        "AOT currently supports only List[Int]; got List[{:?}]", element_ty
                    ));
                }
                let builtin_id = *self
                    .builtin_ids
                    .get("lpp_list_new")
                    .ok_or_else(|| "Builtin 'lpp_list_new' was not declared".to_string())?;
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
                    if let Some(field_index) = struct_def.fields.iter().position(|(name, _)| name == field) {
                        let (layout, _) = struct_layout(self.type_table, *struct_id);
                        let field_layout = layout[field_index];
                        Ok(builder.ins().load(
                            field_layout.ty,
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
            Rvalue::MakeClosure(mir_func_id, args) => {
                let size_val = builder.ins().iconst(cl_types::I64, 16);
                let builtin_id = *self
                    .builtin_ids
                    .get("lpp_arc_alloc_with_destructor")
                    .ok_or_else(|| "Builtin 'lpp_arc_alloc_with_destructor' was not declared".to_string())?;
                let alloc_func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let destroy_id = *self
                    .builtin_ids
                    .get("lpp_closure_destroy")
                    .ok_or_else(|| "Builtin 'lpp_closure_destroy' was not declared".to_string())?;
                let destroy_ref = self.module.declare_func_in_func(destroy_id, builder.func);
                let destroy_addr = builder.ins().func_addr(
                    self.module.target_config().pointer_type(),
                    destroy_ref,
                );
                let call = builder.ins().call(alloc_func_ref, &[size_val, destroy_addr]);
                let closure_ptr = builder.inst_results(call)[0];

                let cl_id = *self.func_ids.get(mir_func_id).ok_or_else(|| {
                    format!("Missing direct-call target for MIR function id {:?}", mir_func_id)
                })?;
                let func_ref = self.module.declare_func_in_func(cl_id, builder.func);
                let pointer_type = self.module.target_config().pointer_type();
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
                let closure_ptr = self.operand_to_value(builder, callee, local_vars)?;
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

                let mut sig = self.module.make_signature();
                sig.params.push(AbiParam::new(pointer_type));
                for arg in args {
                    let arg_ty = match arg {
                        Operand::Local(id) | Operand::Borrowed(id) => locals[id.0].ty.clone(),
                        Operand::Int(_) => TypeRef::Int,
                        Operand::Float(_) => TypeRef::Float,
                        Operand::Bool(_) => TypeRef::Bool,
                        Operand::String(_) => TypeRef::Str,
                    };
                    sig.params.push(AbiParam::new(super::types::type_to_cl(&arg_ty)));
                }

                let ret_ty = dest_ty.cloned().unwrap_or(TypeRef::Void);
                if ret_ty != TypeRef::Void {
                    sig.returns.push(AbiParam::new(super::types::type_to_cl(&ret_ty)));
                }

                let sig_ref = builder.import_signature(sig);
                let mut call_args = vec![env_ptr];
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
            Rvalue::AllocateStruct(_) => Err(
                "raw struct allocation reached AOT; use AllocateArcStruct for owned objects".to_string(),
            ),
            Rvalue::AllocateArcStruct(_) => Err(
                "AllocateArcStruct requires a resolved custom struct type".to_string(),
            ),
            Rvalue::FieldAccess(_, _) => Ok(builder.ins().iconst(cl_types::I64, 0)),
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
                builder.ins().brif(cond_bool, then_block, &[], else_block, &[]);
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
                        TypeRef::Int | TypeRef::Str | TypeRef::Custom(_)
                        | TypeRef::Generic(_, _) | TypeRef::Unresolved(_)
                        | TypeRef::Function | TypeRef::Void => {
                            builder.ins().iconst(cl_types::I64, 0)
                        }
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
