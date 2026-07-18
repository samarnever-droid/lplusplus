use super::types::type_to_cl;
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
            Operand::Local(id) => {
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
                        let offset = (field_index * 8) as i32;
                        builder.ins().store(
                            cranelift_codegen::ir::MemFlags::new(),
                            value_value,
                            base_value,
                            offset,
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
            MirInstr::Retain(_) | MirInstr::Release(_) => {}
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
            Rvalue::BinaryOp(op, left, right) => {
                let left = self.operand_to_value(builder, left, local_vars)?;
                let right = self.operand_to_value(builder, right, local_vars)?;
                let is_float = builder.func.dfg.value_type(left) == cl_types::F64;
                Ok(match op {
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
                    BinaryOperator::Modulo => {
                        if is_float { builder.ins().fsub(left, right) }
                        else { builder.ins().srem(left, right) }
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
                })
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
            Rvalue::AllocateStruct(TypeRef::Custom(struct_id)) => {
                let struct_def = &self.type_table.definitions[struct_id.0];
                let size = (struct_def.fields.len() * 8) as i64;
                let size_val = builder.ins().iconst(cl_types::I64, size);
                let builtin_id = *self
                    .builtin_ids
                    .get("lpp_alloc")
                    .ok_or_else(|| "Builtin 'lpp_alloc' was not declared".to_string())?;
                let func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let call = builder.ins().call(func_ref, &[size_val]);
                let results = builder.inst_results(call);
                results
                    .first()
                    .copied()
                    .ok_or_else(|| "Allocator call returned no value".to_string())
            }
            Rvalue::AllocateList(_) => {
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
            Rvalue::FieldAccess(Operand::Local(base_id), field) => {
                let base_value =
                    self.operand_to_value(builder, &Operand::Local(*base_id), local_vars)?;
                let base_ty = &locals[base_id.0].ty;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_index) = struct_def.fields.iter().position(|(name, _)| name == field) {
                        let offset = (field_index * 8) as i32;
                        Ok(builder.ins().load(
                            cl_types::I64,
                            cranelift_codegen::ir::MemFlags::new(),
                            base_value,
                            offset,
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
                    .get("lpp_alloc")
                    .ok_or_else(|| "Builtin 'lpp_alloc' was not declared".to_string())?;
                let alloc_func_ref = self.module.declare_func_in_func(builtin_id, builder.func);
                let call = builder.ins().call(alloc_func_ref, &[size_val]);
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

                let env_val = self.operand_to_value(builder, &args[0], local_vars)?;

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
                        Operand::Local(id) => locals[id.0].ty.clone(),
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
            Rvalue::FieldAccess(_, _)
            | Rvalue::AllocateStruct(_) => Ok(builder.ins().iconst(cl_types::I64, 0)),
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
            Terminator::Return(Some(op)) => {
                let value = self.operand_to_value(builder, op, local_vars)?;
                builder.ins().return_(&[value]);
            }
            Terminator::Return(None) => {
                if *return_type == TypeRef::Void {
                    builder.ins().return_(&[]);
                } else {
                    let zero = builder.ins().iconst(cl_types::I64, 0);
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
