use cranelift_codegen::ir::{AbiParam, InstBuilder, Value};
use cranelift_codegen::ir::types as cl_types;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{FuncId as CLFuncId, Module};
use cranelift_codegen::entity::EntityRef;
use crate::mir::ir::*;
use crate::typecheck::TypeRef;
use crate::ast::BinaryOperator;
use super::types::type_to_cl;
use std::collections::HashMap;

/// Lowers a single MIR function into Cranelift IR and defines it in the module.
pub struct FunctionLower<'a, M: Module> {
    pub module: &'a mut M,
    /// Maps MIR FuncId → Cranelift FuncId (user-defined functions)
    pub func_ids: &'a HashMap<FuncId, CLFuncId>,
    /// Maps runtime builtin symbol name → Cranelift FuncId
    pub builtin_ids: &'a HashMap<String, CLFuncId>,
}

impl<'a, M: Module> FunctionLower<'a, M> {
    /// Translate one MIR function into the Cranelift module.
    pub fn lower_function(&mut self, mir_fn: &MirFunction) -> Result<(), String> {
        // 1. Build signature
        let mut sig = self.module.make_signature();
        for param_id in &mir_fn.params {
            let decl = &mir_fn.locals[param_id.0];
            sig.params.push(AbiParam::new(type_to_cl(&decl.ty)));
        }
        if mir_fn.return_type != TypeRef::Void {
            sig.returns.push(AbiParam::new(type_to_cl(&mir_fn.return_type)));
        }

        let func_id = self.func_ids[&mir_fn.id];
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        ctx.func.name = cranelift_codegen::ir::UserFuncName::user(0, func_id.as_u32());

        let mut fn_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);

            // Declare Variables for each local
            let mut local_vars: HashMap<LocalId, Variable> = HashMap::new();
            for (i, local) in mir_fn.locals.iter().enumerate() {
                let var = Variable::new(i);
                let cl_ty = type_to_cl(&local.ty);
                builder.declare_var(var, cl_ty);
                local_vars.insert(local.id, var);
            }

            // Map MIR BlockId → Cranelift Block
            let mut cl_blocks: HashMap<BlockId, cranelift_codegen::ir::Block> = HashMap::new();
            for block in &mir_fn.blocks {
                let cl_block = builder.create_block();
                cl_blocks.insert(block.id, cl_block);
            }

            // Entry block: wire params
            let entry_block = cl_blocks[&mir_fn.blocks[0].id];
            builder.switch_to_block(entry_block);
            builder.append_block_params_for_function_params(entry_block);
            let param_vals: Vec<Value> = builder.block_params(entry_block).to_vec();
            for (i, param_id) in mir_fn.params.iter().enumerate() {
                builder.def_var(local_vars[param_id], param_vals[i]);
            }

            // Lower blocks
            for (bi, block) in mir_fn.blocks.iter().enumerate() {
                let cl_block = cl_blocks[&block.id];
                if bi != 0 {
                    builder.switch_to_block(cl_block);
                }
                for instr in &block.instrs {
                    Self::lower_instr_inner(
                        &mut builder, instr, &local_vars,
                        self.func_ids, self.builtin_ids, self.module,
                    );
                }
                Self::lower_terminator_inner(
                    &mut builder, &block.terminator,
                    &cl_blocks, &local_vars, &mir_fn.return_type,
                );
            }

            builder.seal_all_blocks();
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("define_function '{}': {:?}", mir_fn.name, e))?;

        Ok(())
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    fn operand_to_value(
        builder: &mut FunctionBuilder,
        op: &Operand,
        local_vars: &HashMap<LocalId, Variable>,
    ) -> Value {
        match op {
            Operand::Local(id)  => builder.use_var(local_vars[id]),
            Operand::Int(i)     => builder.ins().iconst(cl_types::I64, *i),
            Operand::Bool(b)    => builder.ins().iconst(cl_types::I8, if *b { 1 } else { 0 }),
            Operand::String(_)  => builder.ins().iconst(cl_types::I64, 0), // data-section ptr: MVP
        }
    }

    fn lower_instr_inner(
        builder: &mut FunctionBuilder,
        instr: &MirInstr,
        local_vars: &HashMap<LocalId, Variable>,
        func_ids: &HashMap<FuncId, CLFuncId>,
        builtin_ids: &HashMap<String, CLFuncId>,
        module: &mut M,
    ) {
        match instr {
            MirInstr::Assign(dest, rvalue) => {
                let val = Self::lower_rvalue_inner(
                    builder, rvalue, local_vars, func_ids, builtin_ids, module,
                );
                builder.def_var(local_vars[dest], val);
            }
            MirInstr::AssignField { .. } | MirInstr::Retain(_) | MirInstr::Release(_) => {
                // ARC and struct fields: runtime stubs; full impl in a later pass.
            }
        }
    }

    fn lower_rvalue_inner(
        builder: &mut FunctionBuilder,
        rvalue: &Rvalue,
        local_vars: &HashMap<LocalId, Variable>,
        func_ids: &HashMap<FuncId, CLFuncId>,
        builtin_ids: &HashMap<String, CLFuncId>,
        module: &mut M,
    ) -> Value {
        match rvalue {
            Rvalue::Use(op) => Self::operand_to_value(builder, op, local_vars),

            Rvalue::BinaryOp(op, l, r) => {
                let lv = Self::operand_to_value(builder, l, local_vars);
                let rv = Self::operand_to_value(builder, r, local_vars);
                match op {
                    BinaryOperator::Add      => builder.ins().iadd(lv, rv),
                    BinaryOperator::Subtract => builder.ins().isub(lv, rv),
                    BinaryOperator::Multiply => builder.ins().imul(lv, rv),
                    BinaryOperator::Divide   => builder.ins().sdiv(lv, rv),
                    BinaryOperator::Eq =>
                        { let r = builder.ins().icmp(IntCC::Equal, lv, rv); builder.ins().uextend(cl_types::I64, r) }
                    BinaryOperator::NotEq =>
                        { let r = builder.ins().icmp(IntCC::NotEqual, lv, rv); builder.ins().uextend(cl_types::I64, r) }
                    BinaryOperator::Less =>
                        { let r = builder.ins().icmp(IntCC::SignedLessThan, lv, rv); builder.ins().uextend(cl_types::I64, r) }
                    BinaryOperator::Greater =>
                        { let r = builder.ins().icmp(IntCC::SignedGreaterThan, lv, rv); builder.ins().uextend(cl_types::I64, r) }
                    BinaryOperator::LessEq =>
                        { let r = builder.ins().icmp(IntCC::SignedLessThanOrEqual, lv, rv); builder.ins().uextend(cl_types::I64, r) }
                    BinaryOperator::GreaterEq =>
                        { let r = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, lv, rv); builder.ins().uextend(cl_types::I64, r) }
                }
            }

            Rvalue::CallDirect(mir_func_id, args) => {
                if let Some(&cl_id) = func_ids.get(mir_func_id) {
                    let func_ref = module.declare_func_in_func(cl_id, builder.func);
                    let arg_vals: Vec<Value> = args.iter()
                        .map(|a| Self::operand_to_value(builder, a, local_vars))
                        .collect();
                    let call = builder.ins().call(func_ref, &arg_vals);
                    let results = builder.inst_results(call);
                    if results.is_empty() { builder.ins().iconst(cl_types::I64, 0) }
                    else { results[0] }
                } else {
                    builder.ins().iconst(cl_types::I64, 0)
                }
            }

            Rvalue::BuiltinCall(sym, args) => {
                if let Some(&cl_id) = builtin_ids.get(sym) {
                    let func_ref = module.declare_func_in_func(cl_id, builder.func);
                    let arg_vals: Vec<Value> = args.iter()
                        .map(|a| Self::operand_to_value(builder, a, local_vars))
                        .collect();
                    let call = builder.ins().call(func_ref, &arg_vals);
                    let results = builder.inst_results(call);
                    if results.is_empty() { builder.ins().iconst(cl_types::I64, 0) }
                    else { results[0] }
                } else {
                    // Unknown builtin: warn but don't crash
                    builder.ins().iconst(cl_types::I64, 0)
                }
            }

            // Stubs for MVP
            Rvalue::CallIndirect(_, _)
            | Rvalue::MakeClosure(_, _)
            | Rvalue::FieldAccess(_, _)
            | Rvalue::AllocateStruct(_)
            | Rvalue::AllocateList(_) => builder.ins().iconst(cl_types::I64, 0),
        }
    }

    fn lower_terminator_inner(
        builder: &mut FunctionBuilder,
        term: &Terminator,
        cl_blocks: &HashMap<BlockId, cranelift_codegen::ir::Block>,
        local_vars: &HashMap<LocalId, Variable>,
        return_type: &TypeRef,
    ) {
        match term {
            Terminator::Goto(target) => {
                builder.ins().jump(cl_blocks[target], &[]);
            }
            Terminator::If { cond, then_block, else_block } => {
                let cv = Self::operand_to_value(builder, cond, local_vars);
                let cb = builder.ins().icmp_imm(IntCC::NotEqual, cv, 0);
                builder.ins().brif(cb, cl_blocks[then_block], &[], cl_blocks[else_block], &[]);
            }
            Terminator::Return(Some(op)) => {
                let val = Self::operand_to_value(builder, op, local_vars);
                builder.ins().return_(&[val]);
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
                builder.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
            }
        }
    }
}
