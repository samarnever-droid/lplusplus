use crate::ast::*;
use crate::mir::builder::MirBuilder;
use crate::mir::ir::*;
use crate::semantic::{BindingId, ScopeKind, SymbolTable};
use crate::typecheck::{TypeRef, TypeTable};
use std::collections::HashMap;

pub struct MirLowerCtx<'a> {
    pub symbol_table: &'a SymbolTable,
    pub type_table: &'a TypeTable,
    pub functions: HashMap<String, FuncId>,
    pub func_return_types: HashMap<String, TypeRef>,
    pub next_func_id: usize,
}

impl<'a> MirLowerCtx<'a> {
    pub fn new(symbol_table: &'a SymbolTable, type_table: &'a TypeTable) -> Self {
        Self {
            symbol_table,
            type_table,
            functions: HashMap::new(),
            func_return_types: HashMap::new(),
            next_func_id: 0,
        }
    }

    fn get_field_type(&self, base_ty: &TypeRef, field: &str) -> TypeRef {
        if let TypeRef::Custom(struct_id) = base_ty {
            let struct_def = &self.type_table.definitions[struct_id.0];
            if let Some((_, ty)) = struct_def.fields.iter().find(|(name, _)| name == field) {
                return ty.clone();
            }
        }
        TypeRef::Void
    }

    fn resolve_type(&self, ty: &Type) -> TypeRef {
        match ty {
            Type::Int => TypeRef::Int,
            Type::String => TypeRef::Str,
            Type::Void => TypeRef::Void,
            Type::Custom(name) => self
                .type_table
                .structs_by_name
                .get(name)
                .copied()
                .map(TypeRef::Custom)
                .unwrap_or_else(|| TypeRef::Unresolved(name.clone())),
            Type::Generic(name, args) => {
                TypeRef::Generic(name.clone(), args.iter().map(|arg| self.resolve_type(arg)).collect())
            }
        }
    }

    fn expr_type_hint(
        &self,
        expr: &Expr,
        builder: &MirBuilder,
        binding_map: &HashMap<BindingId, LocalId>,
    ) -> TypeRef {
        match expr {
            Expr::IntLiteral(_) => TypeRef::Int,
            Expr::StringLiteral(_) => TypeRef::Str,
            Expr::Identifier(_, cell) => {
                if let Some(ast_id) = cell.get() {
                    if let Some(local_id) = binding_map.get(&BindingId(ast_id)) {
                        return builder.function.locals[local_id.0].ty.clone();
                    }
                }
                TypeRef::Int
            }
            Expr::FieldAccess { base, field } => {
                let base_ty = self.expr_type_hint(base, builder, binding_map);
                self.get_field_type(&base_ty, field)
            }
            Expr::ListLiteral(items) => TypeRef::Generic(
                "List".to_string(),
                vec![items
                    .first()
                    .map(|item| self.expr_type_hint(item, builder, binding_map))
                    .unwrap_or(TypeRef::Int)],
            ),
            Expr::Call { callee, .. } => {
                if let Expr::Identifier(name, _) = &**callee {
                    if let Some(ty) = self.func_return_types.get(name) {
                        return ty.clone();
                    }
                    if let Some(&struct_id) = self.type_table.structs_by_name.get(name) {
                        return TypeRef::Custom(struct_id);
                    }
                    return match name.as_str() {
                        "input" | "read_file" | "json_get_str" | "net_recv" => TypeRef::Str,
                        "parse_int" | "json_parse" | "json_get_int" | "json_get_obj" | "list_get" | "list_len"
                        | "net_connect" | "net_listen" | "net_accept" | "net_send" => TypeRef::Int,
                        "list_new" => TypeRef::Generic("List".to_string(), vec![TypeRef::Int]),
                        "net_close" => TypeRef::Void,
                        _ => TypeRef::Void,
                    };
                }
                TypeRef::Int
            }
            Expr::BinaryOp { .. } => TypeRef::Int,
            Expr::Closure { .. } => TypeRef::Function,
            Expr::Spawn { .. } => TypeRef::Void,
        }
    }

    pub fn lower_program(&mut self, program: &Program) -> Result<MirProgram, String> {
        let mut mir_functions = HashMap::new();

        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                let id = FuncId(self.next_func_id);
                self.next_func_id += 1;
                self.functions.insert(f.name.clone(), id);
                self.func_return_types
                    .insert(f.name.clone(), self.resolve_type(&f.return_type));
            }
        }

        let ast_functions: Vec<_> = program
            .declarations
            .iter()
            .filter_map(|decl| match decl {
                TopLevel::Function(f) => Some(f),
                _ => None,
            })
            .collect();

        for function in ast_functions {
            let mir_fn = self.lower_function(function)?;
            mir_functions.insert(mir_fn.id, mir_fn);
        }

        Ok(MirProgram { functions: mir_functions })
    }

    fn lower_function(&mut self, func: &Function) -> Result<MirFunction, String> {
        let func_id = *self
            .functions
            .get(&func.name)
            .ok_or_else(|| format!("Internal error: missing MIR function id for '{}'", func.name))?;
        let return_type = self.resolve_type(&func.return_type);
        let mut builder = MirBuilder::new(func_id, func.name.clone(), return_type);
        let mut binding_map = HashMap::new();

        for param in &func.params {
            let binding_id = self.symbol_table.scopes.iter().find_map(|scope| {
                if let ScopeKind::Function { name } = &scope.kind {
                    if name == &func.name {
                        return scope.bindings.get(&param.name).copied();
                    }
                }
                None
            });
            let ty = self.resolve_type(&param.ty);
            let local = builder.new_local(ty, false, Some(param.name.clone()), binding_id);
            builder.function.params.push(local);
            if let Some(binding_id) = binding_id {
                binding_map.insert(binding_id, local);
            }
        }

        for stmt in &func.body {
            self.lower_stmt(&mut builder, stmt, &mut binding_map)?;
        }

        if let Ok(current_block) = builder.current_block() {
            if current_block.0 < builder.function.blocks.len() {
                builder.set_terminator(current_block, Terminator::Return(None))?;
            }
        }

        Ok(builder.finish())
    }

    fn lower_stmt(
        &mut self,
        builder: &mut MirBuilder,
        stmt: &Stmt,
        binding_map: &mut HashMap<BindingId, LocalId>,
    ) -> Result<(), String> {
        match stmt {
            Stmt::LetInferred {
                name,
                value,
                binding_id,
                ..
            } => {
                let ast_id = binding_id
                    .get()
                    .ok_or_else(|| format!("Missing binding id while lowering declaration '{}'", name))?;
                let binding_id = BindingId(ast_id);
                let ty = self
                    .symbol_table
                    .bindings
                    .get(ast_id)
                    .and_then(|binding| binding.ty.clone())
                    .ok_or_else(|| format!("Missing inferred type for binding '{}'", name))?;

                let local_id = builder.new_local(ty, true, Some(name.clone()), Some(binding_id));
                binding_map.insert(binding_id, local_id);

                let operand = self.lower_expr(builder, value, binding_map)?;
                builder.push_instr(MirInstr::Assign(local_id, Rvalue::Use(operand)))?;
            }
            Stmt::Assign {
                value, binding_id, ..
            } => {
                let ast_id = binding_id
                    .get()
                    .ok_or_else(|| "Missing binding id while lowering assignment".to_string())?;
                let binding_id = BindingId(ast_id);
                let local_id = *binding_map
                    .get(&binding_id)
                    .ok_or_else(|| format!("Missing MIR local for binding {}", ast_id))?;

                let operand = self.lower_expr(builder, value, binding_map)?;
                builder.push_instr(MirInstr::Assign(local_id, Rvalue::Use(operand)))?;
            }
            Stmt::AssignField { base, field, value } => {
                let base_op = self.lower_expr(builder, base, binding_map)?;
                let value_op = self.lower_expr(builder, value, binding_map)?;
                if let Operand::Local(base_id) = base_op {
                    builder.push_instr(MirInstr::AssignField {
                        base: base_id,
                        field: field.clone(),
                        value: value_op,
                    })?;
                } else {
                    return Err("Field assignment base is not a local variable".to_string());
                }
            }
            Stmt::Expr(expr) => {
                self.lower_expr(builder, expr, binding_map)?;
            }
            Stmt::Return(expr) => {
                let op = match expr {
                    Some(expr) => Some(self.lower_expr(builder, expr, binding_map)?),
                    None => None,
                };
                builder.terminate_current_block(Terminator::Return(op))?;
                let next = builder.new_block();
                builder.switch_to_block(next);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                let cond_op = self.lower_expr(builder, condition, binding_map)?;
                let then_block_id = builder.new_block();
                let else_block_id = builder.new_block();
                let merge_block_id = builder.new_block();

                builder.terminate_current_block(Terminator::If {
                    cond: cond_op,
                    then_block: then_block_id,
                    else_block: if else_block.is_some() {
                        else_block_id
                    } else {
                        merge_block_id
                    },
                })?;

                builder.switch_to_block(then_block_id);
                for stmt in then_block {
                    self.lower_stmt(builder, stmt, binding_map)?;
                }
                if builder.current_block().is_ok() {
                    builder.terminate_current_block(Terminator::Goto(merge_block_id))?;
                }

                if let Some(else_block) = else_block {
                    builder.switch_to_block(else_block_id);
                    for stmt in else_block {
                        self.lower_stmt(builder, stmt, binding_map)?;
                    }
                    if builder.current_block().is_ok() {
                        builder.terminate_current_block(Terminator::Goto(merge_block_id))?;
                    }
                }

                builder.switch_to_block(merge_block_id);
            }
            Stmt::While { condition, body } => {
                let cond_block_id = builder.new_block();
                let body_block_id = builder.new_block();
                let end_block_id = builder.new_block();

                builder.terminate_current_block(Terminator::Goto(cond_block_id))?;

                builder.switch_to_block(cond_block_id);
                let cond_op = self.lower_expr(builder, condition, binding_map)?;
                builder.terminate_current_block(Terminator::If {
                    cond: cond_op,
                    then_block: body_block_id,
                    else_block: end_block_id,
                })?;

                builder.switch_to_block(body_block_id);
                for stmt in body {
                    self.lower_stmt(builder, stmt, binding_map)?;
                }
                if builder.current_block().is_ok() {
                    builder.terminate_current_block(Terminator::Goto(cond_block_id))?;
                }

                builder.switch_to_block(end_block_id);
            }
        }
        Ok(())
    }

    fn lower_expr(
        &mut self,
        builder: &mut MirBuilder,
        expr: &Expr,
        binding_map: &mut HashMap<BindingId, LocalId>,
    ) -> Result<Operand, String> {
        match expr {
            Expr::IntLiteral(value) => Ok(Operand::Int(*value)),
            Expr::StringLiteral(value) => Ok(Operand::String(value.clone())),
            Expr::Identifier(name, cell) => {
                let ast_id = cell
                    .get()
                    .ok_or_else(|| format!("Missing binding id for identifier '{}'", name))?;
                let local_id = *binding_map.get(&BindingId(ast_id)).ok_or_else(|| {
                    format!(
                        "Identifier '{}' (binding {}) was not mapped into MIR locals for '{}'",
                        name, ast_id, builder.function.name
                    )
                })?;
                Ok(Operand::Local(local_id))
            }
            Expr::BinaryOp { left, op, right } => {
                let left = self.lower_expr(builder, left, binding_map)?;
                let right = self.lower_expr(builder, right, binding_map)?;
                let temp = builder.new_local(TypeRef::Int, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::BinaryOp(op.clone(), left, right),
                ))?;
                Ok(Operand::Local(temp))
            }
            Expr::Call { callee, args } => {
                let mut lowered_args = Vec::new();
                for arg in args {
                    lowered_args.push(self.lower_expr(builder, arg, binding_map)?);
                }

                let mut return_type = TypeRef::Void;
                if let Expr::Identifier(name, _) = &**callee {
                    if let Some(ty) = self.func_return_types.get(name) {
                        return_type = ty.clone();
                    } else if let Some(&struct_id) = self.type_table.structs_by_name.get(name) {
                        return_type = TypeRef::Custom(struct_id);
                    } else {
                        return_type = match name.as_str() {
                            "input" | "read_file" | "json_get_str" => TypeRef::Str,
                            "parse_int"
                            | "json_parse"
                            | "json_get_int"
                            | "json_get_obj"
                            | "list_get"
                            | "list_len"
                            | "net_connect"
                            | "net_listen"
                            | "net_accept"
                            | "net_send" => TypeRef::Int,
                            "net_recv" => TypeRef::Str,
                            "list_new" => TypeRef::Generic("List".to_string(), vec![TypeRef::Int]),
                            _ => TypeRef::Void,
                        };
                    }
                }

                let temp = builder.new_local(return_type, false, None, None);

                if let Expr::Identifier(name, _) = &**callee {
                    if let Some(&func_id) = self.functions.get(name) {
                        builder.push_instr(MirInstr::Assign(
                            temp,
                            Rvalue::CallDirect(func_id, lowered_args),
                        ))?;
                        return Ok(Operand::Local(temp));
                    }

                    if let Some(&struct_id) = self.type_table.structs_by_name.get(name) {
                        builder.push_instr(MirInstr::Assign(
                            temp,
                            Rvalue::AllocateStruct(TypeRef::Custom(struct_id)),
                        ))?;
                        return Ok(Operand::Local(temp));
                    }

                    let builtin_symbol = match name.as_str() {
                        "print" => {
                            let is_string = match lowered_args.first() {
                                Some(Operand::String(_)) => true,
                                Some(Operand::Local(local_id)) => {
                                    builder.function.locals[local_id.0].ty == TypeRef::Str
                                }
                                _ => false,
                            };
                            Some(if is_string {
                                "lpp_print_str"
                            } else {
                                "lpp_print_int"
                            })
                        }
                        "print_str" => Some("lpp_print_str"),
                        "input" => Some("lpp_input"),
                        "read_file" => Some("lpp_read_file"),
                        "write_file" => Some("lpp_write_file"),
                        "parse_int" => Some("lpp_parse_int"),
                        "json_parse" => Some("lpp_json_parse"),
                        "json_get_int" => Some("lpp_json_get_int"),
                        "json_get_str" => Some("lpp_json_get_str"),
                        "json_get_obj" => Some("lpp_json_get_obj"),
                        "json_free" => Some("lpp_json_free"),
                        "list_new" => Some("lpp_list_new"),
                        "list_push" => Some("lpp_list_push"),
                        "list_get" => Some("lpp_list_get"),
                        "list_len" => Some("lpp_list_len"),
                        "list_free" => Some("lpp_list_free"),
                        "net_connect" => Some("lpp_net_connect"),
                        "net_listen" => Some("lpp_net_listen"),
                        "net_accept" => Some("lpp_net_accept"),
                        "net_send" => Some("lpp_net_send"),
                        "net_recv" => Some("lpp_net_recv"),
                        "net_close" => Some("lpp_net_close"),
                        _ => None,
                    };

                    if let Some(symbol) = builtin_symbol {
                        builder.push_instr(MirInstr::Assign(
                            temp,
                            Rvalue::BuiltinCall(symbol.to_string(), lowered_args),
                        ))?;
                        return Ok(Operand::Local(temp));
                    }
                }

                let callee = self.lower_expr(builder, callee, binding_map)?;
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::CallIndirect(callee, lowered_args),
                ))?;
                Ok(Operand::Local(temp))
            }
            Expr::FieldAccess { base, field } => {
                let base_op = self.lower_expr(builder, base, binding_map)?;
                let base_ty = match &base_op {
                    Operand::Local(local_id) => builder.function.locals[local_id.0].ty.clone(),
                    _ => TypeRef::Void,
                };
                let temp = builder.new_local(self.get_field_type(&base_ty, field), false, None, None);
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::FieldAccess(base_op, field.clone()),
                ))?;
                Ok(Operand::Local(temp))
            }
            Expr::ListLiteral(items) => {
                let elem_ty = items
                    .first()
                    .map(|item| self.expr_type_hint(item, builder, binding_map))
                    .unwrap_or(TypeRef::Int);
                let temp = builder.new_local(
                    TypeRef::Generic("List".to_string(), vec![elem_ty.clone()]),
                    false,
                    None,
                    None,
                );
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::AllocateList(elem_ty),
                ))?;
                for item in items {
                    let item_op = self.lower_expr(builder, item, binding_map)?;
                    let discard_local = builder.new_local(TypeRef::Void, false, None, None);
                    builder.push_instr(MirInstr::Assign(
                        discard_local,
                        Rvalue::BuiltinCall(
                            "lpp_list_push".to_string(),
                            vec![Operand::Local(temp), item_op],
                        ),
                    ))?;
                }
                Ok(Operand::Local(temp))
            }
            Expr::Spawn { closure } => self.lower_expr(builder, closure, binding_map),
            Expr::Closure { .. } => Ok(Operand::Int(0)),
        }
    }
}
