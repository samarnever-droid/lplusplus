use crate::ast::*;
use crate::mir::builder::MirBuilder;
use crate::mir::ir::*;
use crate::semantic::{BindingId, ScopeKind, SymbolTable};
use crate::typecheck::{TypeRef, TypeTable};
use std::collections::HashMap;

pub struct MirLowerCtx<'a> {
    pub symbol_table: &'a SymbolTable,
    pub type_table: &'a mut TypeTable,
    pub functions: HashMap<String, FuncId>,
    pub func_return_types: HashMap<String, TypeRef>,
    pub next_func_id: usize,

    // Closure compilation context
    pub lifted_functions: HashMap<FuncId, MirFunction>,
    pub closure_scope_idx: usize,
    pub current_env_ptr: Option<LocalId>,
    pub current_captures: Vec<BindingId>,
}

impl<'a> MirLowerCtx<'a> {
    pub fn new(symbol_table: &'a SymbolTable, type_table: &'a mut TypeTable) -> Self {
        Self {
            symbol_table,
            type_table,
            functions: HashMap::new(),
            func_return_types: HashMap::new(),
            next_func_id: 0,
            lifted_functions: HashMap::new(),
            closure_scope_idx: 0,
            current_env_ptr: None,
            current_captures: Vec::new(),
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
            Type::Float => TypeRef::Float,
            Type::String => TypeRef::Str,
            Type::Bool => TypeRef::Bool,
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
            Expr::FloatLiteral(_) => TypeRef::Float,
            Expr::StringLiteral(_) => TypeRef::Str,
            Expr::BoolLiteral(_) => TypeRef::Bool,
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
                    if let Some(builtin) = crate::builtins::get_builtins().iter().find(|b| b.name == name) {
                        // list_new is special-cased because of generic parameter type inference
                        if name != "list_new" {
                            return builtin.return_type.clone();
                        }
                    }
                    return match name.as_str() {
                        "input" | "read_file" | "json_get_str" | "net_recv" => TypeRef::Str,
                        "parse_int" | "json_parse" | "json_get_int" | "json_get_obj" | "list_get" | "list_len"
                        | "net_connect" | "net_listen" | "net_accept" | "net_send" => TypeRef::Int,
                        "list_new" => TypeRef::Generic("List".to_string(), vec![TypeRef::Int]),
                        "print" | "print_str" | "json_free" | "list_push" | "list_free" | "net_close" => TypeRef::Void,
                        _ => TypeRef::Int,
                    };
                }
                TypeRef::Int
            }
            Expr::BinaryOp { left, .. } => {
                let left_ty = self.expr_type_hint(left, builder, binding_map);
                left_ty
            }
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

        for (id, func) in self.lifted_functions.drain() {
            mir_functions.insert(id, func);
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
            // Function arguments are owned by the caller unless an eventual
            // explicit `owned` parameter mode says otherwise.
            builder.set_local_ownership(local, Ownership::Borrowed);
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

    /// Select an explicit ownership operation for an assignment. A direct
    /// `Local` read of an owned temporary is a move; identifiers of owned
    /// variables lower to `Borrowed` and therefore stay usable after assignment.
    fn assignment_rvalue(builder: &MirBuilder, destination: LocalId, operand: Operand) -> Rvalue {
        if let Operand::Local(source) = operand {
            if builder.function.locals[destination.0].ownership == Ownership::Owned
                && builder.function.locals[source.0].ownership == Ownership::Owned
            {
                return Rvalue::Move(source);
            }
            return Rvalue::Use(Operand::Local(source));
        }
        Rvalue::Use(operand)
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
                let rvalue = Self::assignment_rvalue(builder, local_id, operand);
                builder.push_instr(MirInstr::Assign(local_id, rvalue))?;
            }
            Stmt::Assign {
                value, binding_id, ..
            } => {
                let ast_id = binding_id
                    .get()
                    .ok_or_else(|| "Missing binding id while lowering assignment".to_string())?;
                let binding_id = BindingId(ast_id);
                let operand = self.lower_expr(builder, value, binding_map)?;
                if let Some(local_id) = binding_map.get(&binding_id) {
                    let rvalue = Self::assignment_rvalue(builder, *local_id, operand);
                    builder.push_instr(MirInstr::Assign(*local_id, rvalue))?;
                } else if let Some(env_ptr) = self.current_env_ptr {
                    if let Some(idx) = self.current_captures.iter().position(|&cid| cid == binding_id) {
                        builder.push_instr(MirInstr::AssignField {
                            base: env_ptr,
                            field: format!("cap_{}", idx),
                            value: operand,
                        })?;
                    } else {
                        return Err(format!("Missing MIR local or capture for binding {}", ast_id));
                    }
                } else {
                    return Err(format!("Missing MIR local for binding {}", ast_id));
                }
            }
            Stmt::AssignField { base, field, value } => {
                let base_op = self.lower_expr(builder, base, binding_map)?;
                let value_op = self.lower_expr(builder, value, binding_map)?;
                if let Operand::Local(base_id) | Operand::Borrowed(base_id) = base_op {
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
                // Function ownership contract: custom structs and closure
                // capsules are returned *owned*. Returning an owned local moves
                // its reference. Returning a borrowed parameter/field first
                // retains it, thereby creating the caller's return reference.
                let managed_return = match &op {
                    Some(Operand::Local(local)) | Some(Operand::Borrowed(local)) => {
                        matches!(
                            &builder.function.locals[local.0].ty,
                            TypeRef::Custom(_) | TypeRef::Function | TypeRef::Generic(_, _)
                        )
                        .then_some(*local)
                    }
                    _ => None,
                };
                let terminator = if let Some(local) = managed_return {
                    if builder.function.locals[local.0].ownership == Ownership::Borrowed {
                        builder.push_instr(MirInstr::Retain(local))?;
                    }
                    Terminator::ReturnOwned(Operand::Local(local))
                } else {
                    Terminator::Return(op)
                };
                builder.terminate_current_block(terminator)?;
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
            Stmt::Block(stmts) => {
                for stmt in stmts {
                    self.lower_stmt(builder, stmt, binding_map)?;
                }
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
            Expr::FloatLiteral(value) => Ok(Operand::Float(*value)),
            Expr::StringLiteral(value) => Ok(Operand::String(value.clone())),
            Expr::BoolLiteral(value) => Ok(Operand::Bool(*value)),
            Expr::Identifier(name, cell) => {
                let ast_id = cell
                    .get()
                    .ok_or_else(|| format!("Missing binding id for identifier '{}'", name))?;
                let binding_id = BindingId(ast_id);
                if let Some(local_id) = binding_map.get(&binding_id) {
                    let local = &builder.function.locals[local_id.0];
                    if local.ownership == Ownership::Owned {
                        // Identifier reads borrow owned objects. A later ownership
                        // operation decides whether to retain or move the value.
                        Ok(Operand::Borrowed(*local_id))
                    } else {
                        Ok(Operand::Local(*local_id))
                    }
                } else if let Some(env_ptr) = self.current_env_ptr {
                    if let Some(idx) = self.current_captures.iter().position(|&cid| cid == binding_id) {
                        let cap_ty = self.symbol_table.bindings[binding_id.0].ty.clone().unwrap_or(TypeRef::Int);
                        let temp = builder.new_local(cap_ty.clone(), false, Some(format!("cap_val_{}", name)), None);
                        // A captured custom value is borrowed from the closure
                        // environment; the environment owns the ARC edge.
                        if matches!(cap_ty, TypeRef::Custom(_) | TypeRef::Generic(_, _)) {
                            builder.set_local_ownership(temp, Ownership::Borrowed);
                        }
                        builder.push_instr(MirInstr::Assign(
                            temp,
                            Rvalue::FieldAccess(Operand::Local(env_ptr), format!("cap_{}", idx)),
                        ))?;
                        if builder.function.locals[temp.0].ownership == Ownership::Borrowed {
                            Ok(Operand::Borrowed(temp))
                        } else {
                            Ok(Operand::Local(temp))
                        }
                    } else {
                        Err(format!(
                            "Identifier '{}' (binding {}) was not mapped in locals or captures of '{}'",
                            name, ast_id, builder.function.name
                        ))
                    }
                } else {
                    Err(format!(
                        "Identifier '{}' (binding {}) was not mapped into MIR locals for '{}'",
                        name, ast_id, builder.function.name
                    ))
                }
            }
            Expr::BinaryOp { left, op, right } => {
                let left_ty = self.expr_type_hint(left, builder, binding_map);
                let left = self.lower_expr(builder, left, binding_map)?;
                let right = self.lower_expr(builder, right, binding_map)?;
                let res_ty = match op {
                    BinaryOperator::Eq | BinaryOperator::NotEq |
                    BinaryOperator::Less | BinaryOperator::LessEq |
                    BinaryOperator::Greater | BinaryOperator::GreaterEq => TypeRef::Bool,
                    _ => left_ty,
                };
                let temp = builder.new_local(res_ty, false, None, None);
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
                    } else if let Some(builtin) = crate::builtins::get_builtins().iter().find(|b| b.name == name) {
                        if name == "list_get" {
                            let list_ty = args.first()
                                .map(|arg| self.expr_type_hint(arg, builder, binding_map))
                                .unwrap_or(TypeRef::Int);
                            if let TypeRef::Generic(_, params) = list_ty {
                                if let Some(element_ty) = params.first() {
                                    return_type = element_ty.clone();
                                }
                            }
                        } else if name != "list_new" {
                            return_type = builtin.return_type.clone();
                        } else {
                            // list_new is special-cased because of generic list type inference
                            return_type = TypeRef::Generic("List".to_string(), vec![TypeRef::Int]);
                        }
                    } else {
                        return_type = match name.as_str() {
                            "input" | "read_file" | "json_get_str" | "net_recv" => TypeRef::Str,
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
                            "list_new" => TypeRef::Generic("List".to_string(), vec![TypeRef::Int]),
                            "print" | "print_str" | "json_free" | "list_push" | "list_free" | "net_close" => TypeRef::Void,
                            _ => TypeRef::Int,
                        };
                    }
                } else {
                    return_type = TypeRef::Int;
                }

                let list_get_borrows_element = matches!(
                    &**callee,
                    Expr::Identifier(name, _) if name == "list_get"
                ) && matches!(&return_type, TypeRef::Custom(_));
                let temp = builder.new_local(return_type, false, None, None);
                if list_get_borrows_element {
                    // List[ARC] owns the element edge; get returns only a
                    // borrow. Assignment/return will retain explicitly.
                    builder.set_local_ownership(temp, Ownership::Borrowed);
                }

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
                            Rvalue::AllocateArcStruct(TypeRef::Custom(struct_id)),
                        ))?;
                        return Ok(Operand::Local(temp));
                    }

                    let builtin_symbol = if (name == "list_push" || name == "list_get")
                        && matches!(
                            args.first().map(|arg| self.expr_type_hint(arg, builder, binding_map)),
                            Some(TypeRef::Generic(_, ref params)) if matches!(params.first(), Some(TypeRef::Custom(_)))
                        )
                    {
                        Some(if name == "list_push" {
                            "lpp_list_push_arc".to_string()
                        } else {
                            "lpp_list_get_arc".to_string()
                        })
                    } else if name == "print" {
                        let (is_string, is_float) = match lowered_args.first() {
                            Some(Operand::String(_)) => (true, false),
                            Some(Operand::Float(_)) => (false, true),
                            Some(Operand::Local(local_id)) | Some(Operand::Borrowed(local_id)) => {
                                let ty = &builder.function.locals[local_id.0].ty;
                                (*ty == TypeRef::Str, *ty == TypeRef::Float)
                            }
                            _ => (false, false),
                        };
                        Some(if is_string {
                            "lpp_print_str"
                        } else if is_float {
                            "lpp_print_float"
                        } else {
                            "lpp_print_int"
                        }.to_string())
                    } else {
                        crate::builtins::get_builtins().iter()
                            .find(|b| b.name == name)
                            .map(|b| b.symbol.to_string())
                    };

                    if let Some(symbol) = builtin_symbol {
                        if !symbol.is_empty() {
                            builder.push_instr(MirInstr::Assign(
                                temp,
                                Rvalue::BuiltinCall(symbol, lowered_args),
                            ))?;
                            return Ok(if builder.function.locals[temp.0].ownership == Ownership::Borrowed {
                                Operand::Borrowed(temp)
                            } else {
                                Operand::Local(temp)
                            });
                        }
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
                    Operand::Local(local_id) | Operand::Borrowed(local_id) => {
                        builder.function.locals[local_id.0].ty.clone()
                    }
                    _ => TypeRef::Void,
                };
                let field_ty = self.get_field_type(&base_ty, field);
                let temp = builder.new_local(field_ty.clone(), false, None, None);
                // Reading a custom-struct field borrows the field's ARC edge;
                // it does not transfer ownership out of the containing object.
                if matches!(field_ty, TypeRef::Custom(_) | TypeRef::Generic(_, _)) {
                    builder.set_local_ownership(temp, Ownership::Borrowed);
                }
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::FieldAccess(base_op, field.clone()),
                ))?;
                if builder.function.locals[temp.0].ownership == Ownership::Borrowed {
                    Ok(Operand::Borrowed(temp))
                } else {
                    Ok(Operand::Local(temp))
                }
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
                    Rvalue::AllocateList(elem_ty.clone()),
                ))?;
                let push_symbol = if matches!(elem_ty, TypeRef::Custom(_)) {
                    "lpp_list_push_arc"
                } else {
                    "lpp_list_push"
                };
                for item in items {
                    let item_op = self.lower_expr(builder, item, binding_map)?;
                    let discard_local = builder.new_local(TypeRef::Void, false, None, None);
                    builder.push_instr(MirInstr::Assign(
                        discard_local,
                        Rvalue::BuiltinCall(
                            push_symbol.to_string(),
                            vec![Operand::Local(temp), item_op],
                        ),
                    ))?;
                }
                Ok(Operand::Local(temp))
            }
            Expr::Spawn { closure } => {
                let closure_op = self.lower_expr(builder, closure, binding_map)?;
                let temp = builder.new_local(TypeRef::Void, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::SpawnThread(closure_op),
                ))?;
                Ok(Operand::Local(temp))
            }
            Expr::Closure { params, return_type: opt_return_type, body } => {
                let closure_scope = {
                    let mut scope = None;
                    for i in self.closure_scope_idx..self.symbol_table.scopes.len() {
                        if let ScopeKind::Closure { .. } = self.symbol_table.scopes[i].kind {
                            scope = Some(self.symbol_table.scopes[i].id);
                            self.closure_scope_idx = i + 1;
                            break;
                        }
                    }
                    scope.ok_or_else(|| "Closure scope not found".to_string())?
                };

                let captures = match &self.symbol_table.scopes[closure_scope.0].kind {
                    ScopeKind::Closure { captures } => captures.clone(),
                    _ => Vec::new(),
                };

                // Mutable captures need a defined shared-cell / move ownership model.
                // Copying them into an environment makes `x = ...` inside the closure
                // silently diverge from the outer variable, which is not memory-safe or
                // unsurprising language semantics. Reject this case until that model is
                // implemented rather than compiling an incorrect program.
                if let Some(capture) = captures.iter().find(|id| self.symbol_table.bindings[id.0].is_mut) {
                    let binding = &self.symbol_table.bindings[capture.0];
                    return Err(format!(
                        "mutable capture '{}' is not supported safely by AOT closures yet",
                        binding.name
                    ));
                }

                // Register environment struct
                let env_struct_name = format!("__lpp_closure_env_{}", closure_scope.0);
                let env_struct_id = self.type_table.register_struct(env_struct_name);
                
                let mut fields = Vec::new();
                for (i, &cap_id) in captures.iter().enumerate() {
                    let binding = &self.symbol_table.bindings[cap_id.0];
                    let ty = binding.ty.as_ref().cloned().unwrap_or(TypeRef::Int);
                    fields.push((format!("cap_{}", i), ty));
                }
                self.type_table.definitions[env_struct_id.0].fields = fields;

                // Allocate environment struct at definition site
                let env_local = builder.new_local(
                    TypeRef::Custom(env_struct_id),
                    false,
                    Some("__env_alloc".to_string()),
                    None,
                );
                builder.push_instr(MirInstr::Assign(
                    env_local,
                    Rvalue::AllocateArcStruct(TypeRef::Custom(env_struct_id)),
                ))?;

                // Populate captures
                for (i, &cap_id) in captures.iter().enumerate() {
                    let binding = &self.symbol_table.bindings[cap_id.0];
                    let val_op = self.lower_expr(
                        builder,
                        &Expr::Identifier(binding.name.clone(), std::cell::Cell::new(Some(cap_id.0))),
                        binding_map,
                    )?;
                    builder.push_instr(MirInstr::AssignField {
                        base: env_local,
                        field: format!("cap_{}", i),
                        value: val_op,
                    })?;
                }

                // Allocate func ID for lifted closure function
                let closure_func_id = FuncId(self.next_func_id);
                self.next_func_id += 1;
                let closure_name = format!("__lpp_closure_fn_{}", closure_func_id.0);

                // Lower closure function
                let return_type = if let Some(t) = opt_return_type {
                    self.resolve_type(t)
                } else {
                    let mut inferred_rt = TypeRef::Void;
                    for stmt in body {
                        if let Stmt::Return(Some(expr)) = stmt {
                            // Best-effort typehint:
                            if let Ok(ty) = self.lower_expr(builder, expr, binding_map) {
                                if let Operand::Local(lid) | Operand::Borrowed(lid) = ty {
                                    inferred_rt = builder.function.locals[lid.0].ty.clone();
                                }
                            }
                            break;
                        }
                    }
                    inferred_rt
                };

                let mut closure_builder = MirBuilder::new(closure_func_id, closure_name.clone(), return_type);
                let mut closure_binding_map = HashMap::new();

                let env_ptr_local = closure_builder.new_local(
                    TypeRef::Custom(env_struct_id),
                    false,
                    Some("__env".to_string()),
                    None,
                );
                closure_builder.set_local_ownership(env_ptr_local, Ownership::Borrowed);
                closure_builder.function.params.push(env_ptr_local);

                for param in params {
                    let param_binding_id = self.symbol_table.scopes[closure_scope.0].bindings.get(&param.name).copied();
                    let ty = if let Some(ref t) = param.ty {
                        self.resolve_type(t)
                    } else if let Some(bid) = param_binding_id {
                        self.symbol_table.bindings[bid.0].ty.clone().unwrap_or(TypeRef::Int)
                    } else {
                        TypeRef::Int
                    };
                    let local = closure_builder.new_local(ty, false, Some(param.name.clone()), param_binding_id);
                    closure_builder.set_local_ownership(local, Ownership::Borrowed);
                    closure_builder.function.params.push(local);
                    if let Some(bid) = param_binding_id {
                        closure_binding_map.insert(bid, local);
                    }
                }

                // Set closure lowering context
                let saved_env_ptr = self.current_env_ptr;
                let saved_captures = std::mem::take(&mut self.current_captures);
                
                self.current_env_ptr = Some(env_ptr_local);
                self.current_captures = captures;

                for stmt in body {
                    self.lower_stmt(&mut closure_builder, stmt, &mut closure_binding_map)?;
                }

                if let Ok(current_block) = closure_builder.current_block() {
                    if current_block.0 < closure_builder.function.blocks.len() {
                        closure_builder.set_terminator(current_block, Terminator::Return(None))?;
                    }
                }

                let mir_fn = closure_builder.finish();
                self.lifted_functions.insert(closure_func_id, mir_fn);

                // Restore context
                self.current_env_ptr = saved_env_ptr;
                self.current_captures = saved_captures;

                // Return closure fat pointer
                let closure_local = builder.new_local(
                    TypeRef::Function,
                    false,
                    Some("__closure".to_string()),
                    None,
                );
                builder.push_instr(MirInstr::Assign(
                    closure_local,
                    Rvalue::MakeClosure(closure_func_id, vec![Operand::Local(env_local)]),
                ))?;

                Ok(Operand::Local(closure_local))
            }
        }
    }
}
