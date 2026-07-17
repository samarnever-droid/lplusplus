use std::collections::HashMap;
use crate::ast::*;
use crate::semantic::{SymbolTable, ScopeId, ScopeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructTypeId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    Int,
    Str,
    Void,
    Bool,
    Custom(StructTypeId),
    Generic(String, Vec<TypeRef>),
    Unresolved(String),
    Function, // Just a placeholder for function names
}

#[derive(Debug, Clone)]
pub struct StructTypeDef {
    pub name: String,
    pub fields: Vec<(String, TypeRef)>,
    pub is_self_referential: bool,
}

#[derive(Debug)]
pub struct TypeTable {
    pub structs_by_name: HashMap<String, StructTypeId>,
    pub definitions: Vec<StructTypeDef>,
}

impl TypeTable {
    pub fn new() -> Self {
        Self {
            structs_by_name: HashMap::new(),
            definitions: Vec::new(),
        }
    }

    pub fn register_struct(&mut self, name: String) -> StructTypeId {
        let id = StructTypeId(self.definitions.len());
        self.structs_by_name.insert(name.clone(), id);
        self.definitions.push(StructTypeDef {
            name,
            fields: Vec::new(),
            is_self_referential: false,
        });
        id
    }
}

pub struct TypeChecker<'a> {
    pub type_table: TypeTable,
    pub symbol_table: &'a mut SymbolTable,
    pub closure_scope_idx: usize,
    pub block_scope_idx: usize,    // BUG-11: tracks Block scopes for if/while bodies
    pub func_return_types: HashMap<String, TypeRef>,
    pub func_param_types: HashMap<String, Vec<TypeRef>>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(symbol_table: &'a mut SymbolTable) -> Self {
        Self {
            type_table: TypeTable::new(),
            symbol_table,
            closure_scope_idx: 0,
            block_scope_idx: 0,
            func_return_types: HashMap::new(),
            func_param_types: HashMap::new(),
        }
    }

    /// BUG-11: Find the next Block scope in document order and advance the index.
    /// Falls back to `parent` if no block scope is found (defensive; shouldn't happen).
    fn next_block_scope(&mut self, parent: ScopeId) -> ScopeId {
        for i in self.block_scope_idx..self.symbol_table.scopes.len() {
            if let ScopeKind::Block = self.symbol_table.scopes[i].kind {
                self.block_scope_idx = i + 1;
                return ScopeId(i);
            }
        }
        parent // defensive fallback
    }

    fn convert_ast_type(type_table: &TypeTable, ast_ty: &Type) -> TypeRef {
        match ast_ty {
            Type::Int => TypeRef::Int,
            Type::String => TypeRef::Str,
            Type::Void => TypeRef::Void,
            Type::Custom(name) => {
                if let Some(&id) = type_table.structs_by_name.get(name) {
                    TypeRef::Custom(id)
                } else {
                    TypeRef::Unresolved(name.clone())
                }
            }
            Type::Generic(base_name, args) => {
                let mut ref_args = Vec::new();
                for arg in args {
                    ref_args.push(Self::convert_ast_type(type_table, arg));
                }
                TypeRef::Generic(base_name.clone(), ref_args)
            }
        }
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), String> {
        // Phase 1: Register all struct names (stubs) and map function return types
        for decl in &program.declarations {
            if let TopLevel::Struct(s) = decl {
                self.type_table.register_struct(s.name.clone());
            }
        }
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                let ret_ty = Self::convert_ast_type(&self.type_table, &f.return_type);
                self.func_return_types.insert(f.name.clone(), ret_ty);
                let param_tys: Vec<TypeRef> = f.params.iter()
                    .map(|p| Self::convert_ast_type(&self.type_table, &p.ty))
                    .collect();
                self.func_param_types.insert(f.name.clone(), param_tys);
            }
        }

        // Phase 2: Resolve struct fields and check for self-reference
        for decl in &program.declarations {
            if let TopLevel::Struct(s) = decl {
                let id = *self.type_table.structs_by_name.get(&s.name).unwrap();
                
                let mut resolved_fields = Vec::new();
                let mut is_self_referential = false;

                for field in &s.fields {
                    let field_ty = Self::convert_ast_type(&self.type_table, &field.ty);
                    
                    if let TypeRef::Custom(ref_id) = field_ty {
                        if ref_id == id {
                            is_self_referential = true;
                        }
                    } else if let TypeRef::Unresolved(name) = &field_ty {
                        return Err(format!("Unknown type '{}' in struct '{}'", name, s.name));
                    }

                    resolved_fields.push((field.name.clone(), field_ty));
                }

                let def = &mut self.type_table.definitions[id.0];
                def.fields = resolved_fields;
                def.is_self_referential = is_self_referential;
            }
        }

        // Phase 3: Update all bindings in the symbol table with resolved TypeRefs
        for binding in &mut self.symbol_table.bindings {
            if let Some(ast_ty) = &binding.ast_ty {
                binding.ty = Some(Self::convert_ast_type(&self.type_table, ast_ty));
            }
        }

        // Phase 4: Local Type Inference
        for decl in &program.declarations {
            if let TopLevel::Function(func) = decl {
                // Find the function scope
                let mut func_scope_id = None;
                for scope in &self.symbol_table.scopes {
                    if let ScopeKind::Function { name } = &scope.kind {
                        if name == &func.name {
                            func_scope_id = Some(scope.id);
                            break;
                        }
                    }
                }

                if let Some(scope_id) = func_scope_id {
                    for stmt in &func.body {
                        self.infer_stmt(stmt, scope_id)?;
                    }
                }
            }
        }

        Ok(())
    }

    fn infer_stmt(&mut self, stmt: &Stmt, current_scope: ScopeId) -> Result<(), String> {
        match stmt {
            Stmt::LetInferred { name: _, is_mut: _, value, binding_id } => {
                let inferred_type = self.infer_expr(value, current_scope, None)?;
                let b_id = binding_id.get().ok_or_else(|| "Binding ID not set".to_string())?;
                let binding = &mut self.symbol_table.bindings[b_id];
                if binding.ty.is_none() {
                    binding.ty = Some(inferred_type);
                }
            }
            Stmt::Assign { name, value, binding_id } => {
                let expected_ty = if let Some(b_id) = binding_id.get() {
                    self.symbol_table.bindings[b_id].ty.clone()
                } else if let Some(b_id) = self.symbol_table.resolve_name(current_scope, name) {
                    binding_id.set(Some(b_id.0));
                    self.symbol_table.bindings[b_id.0].ty.clone()
                } else {
                    None
                };
                self.infer_expr(value, current_scope, expected_ty)?;
            }
            Stmt::AssignField { base, field, value } => {
                let base_ty = self.infer_expr(base, current_scope, None)?;
                let mut expected_ty = None;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_entry) = struct_def.fields.iter().find(|(name, _)| name == field) {
                        expected_ty = Some(field_entry.1.clone());
                    }
                }
                let val_ty = self.infer_expr(value, current_scope, expected_ty)?;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_entry) = struct_def.fields.iter().find(|(name, _)| name == field) {
                        if field_entry.1 != val_ty {
                            return Err(format!(
                                "Type mismatch in field assignment: expected {:?}, got {:?}",
                                field_entry.1, val_ty
                            ));
                        }
                    } else {
                        return Err(format!("Field '{}' not found on struct '{}'", field, struct_def.name));
                    }
                } else {
                    return Err(format!("Cannot access field '{}' on non-struct type {:?}", field, base_ty));
                }
            }
            Stmt::If { condition, then_block, else_block } => {
                let cond_ty = self.infer_expr(condition, current_scope, None)?;
                if cond_ty != TypeRef::Bool {
                    if cond_ty != TypeRef::Int {
                        return Err(format!("'if' condition must be Bool or Int, found {:?}", cond_ty));
                    }
                }
                
                // BUG-11: use the block's own scope, not the outer function scope
                let then_scope = self.next_block_scope(current_scope);
                for stmt in then_block {
                    self.infer_stmt(stmt, then_scope)?
                }
                
                if let Some(else_b) = else_block {
                    // BUG-11: use the block's own scope, not the outer function scope
                    let else_scope = self.next_block_scope(current_scope);
                    for stmt in else_b {
                        self.infer_stmt(stmt, else_scope)?
                    }
                }
            }
            Stmt::While { condition, body } => {
                let cond_ty = self.infer_expr(condition, current_scope, None)?;
                if cond_ty != TypeRef::Bool && cond_ty != TypeRef::Int {
                    return Err(format!("'while' condition must be Bool or Int, found {:?}", cond_ty));
                }
                
                // BUG-11: use the while body's own block scope
                let body_scope = self.next_block_scope(current_scope);
                for stmt in body {
                    self.infer_stmt(stmt, body_scope)?;
                }
            }
            Stmt::Expr(expr) => {
                self.infer_expr(expr, current_scope, None)?;
            }
            Stmt::Return(Some(expr)) => {
                let mut expected_ret_ty = None;
                let mut curr = Some(current_scope);
                while let Some(sid) = curr {
                    if let ScopeKind::Function { name } = &self.symbol_table.scopes[sid.0].kind {
                        expected_ret_ty = self.func_return_types.get(name).cloned();
                        break;
                    }
                    curr = self.symbol_table.scopes[sid.0].parent;
                }
                self.infer_expr(expr, current_scope, expected_ret_ty)?;
            }
            Stmt::Return(None) => {}
        }
        Ok(())
    }

    fn infer_expr(&mut self, expr: &Expr, current_scope: ScopeId, expected_ty: Option<TypeRef>) -> Result<TypeRef, String> {
        match expr {
            Expr::IntLiteral(_) => Ok(TypeRef::Int),
            Expr::StringLiteral(_) => Ok(TypeRef::Str),
            Expr::Identifier(name, binding_id_cell) => {
                if let Some(id) = binding_id_cell.get() {
                    let binding = &self.symbol_table.bindings[id];
                    binding.ty.clone().ok_or_else(|| "Type of identifier not yet inferred".to_string())
                } else {
                    // BUG-05: Builtin identifiers have no binding_id (semantic resolver skips them).
                    // Return their known types instead of panicking with "Unresolved identifier".
                    match name.as_str() {
                        "input" | "read_file" | "json_get_str" | "net_recv" => Ok(TypeRef::Str),
                        "print" | "print_str" | "write_file" | "json_free"
                        | "list_push" | "list_free" | "net_close" => Ok(TypeRef::Void),
                        "parse_int" | "json_parse" | "json_get_int"
                        | "json_get_obj" | "list_get" | "list_len"
                        | "net_connect" | "net_listen" | "net_accept" | "net_send" => Ok(TypeRef::Int),
                        "list_new" => Ok(TypeRef::Generic("List".to_string(), vec![TypeRef::Int])),
                        _ => Err(format!("Unresolved identifier '{}'", name)),
                    }
                }
            }
            Expr::BinaryOp { left, op, right } => {
                let left_ty = self.infer_expr(left, current_scope, None)?;
                let right_ty = self.infer_expr(right, current_scope, None)?;
                let is_ptr_null_check = match (&left_ty, &right_ty) {
                    (&TypeRef::Custom(_), &TypeRef::Int) | (&TypeRef::Int, &TypeRef::Custom(_)) => {
                        matches!(op, crate::ast::BinaryOperator::Eq | crate::ast::BinaryOperator::NotEq)
                    }
                    _ => false,
                };
                if left_ty != right_ty && !is_ptr_null_check {
                    return Err(format!("Type mismatch in binary operation: {:?} and {:?}", left_ty, right_ty));
                }
                match op {
                    crate::ast::BinaryOperator::Add | crate::ast::BinaryOperator::Subtract | 
                    crate::ast::BinaryOperator::Multiply | crate::ast::BinaryOperator::Divide |
                    crate::ast::BinaryOperator::Modulo => Ok(left_ty),
                    crate::ast::BinaryOperator::Eq | crate::ast::BinaryOperator::NotEq |
                    crate::ast::BinaryOperator::Less | crate::ast::BinaryOperator::LessEq |
                    crate::ast::BinaryOperator::Greater | crate::ast::BinaryOperator::GreaterEq => Ok(TypeRef::Bool),
                }
            }
            Expr::Call { callee, args } => {
                let mut param_tys = Vec::new();
                if let Expr::Identifier(name, _) = &**callee {
                    if let Some(tys) = self.func_param_types.get(name) {
                        param_tys = tys.clone();
                    } else if name == "list_push" && args.len() >= 2 {
                        let list_ty = self.infer_expr(&args[0], current_scope, None)?;
                        if let TypeRef::Generic(ref list_name, ref params) = list_ty {
                            if list_name == "List" && !params.is_empty() {
                                param_tys = vec![list_ty.clone(), params[0].clone()];
                            }
                        }
                    } else if name == "list_get" && args.len() >= 2 {
                        let list_ty = self.infer_expr(&args[0], current_scope, None)?;
                        if let TypeRef::Generic(ref list_name, ref params) = list_ty {
                            if list_name == "List" && !params.is_empty() {
                                param_tys = vec![list_ty.clone(), TypeRef::Int];
                            }
                        }
                    }
                }

                let mut arg_tys = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let expected_arg_ty = param_tys.get(i);
                    arg_tys.push(self.infer_expr(arg, current_scope, expected_arg_ty.cloned())?);
                }
                
                if let Expr::Identifier(name, _) = &**callee {
                    if name == "print" {
                        if args.is_empty() {
                            return Err("print requires at least 1 argument".to_string());
                        }
                        return Ok(TypeRef::Void);
                    }
                    if name == "print_str" {
                        if args.len() != 1 {
                            return Err(format!("print_str expects 1 argument, got {}", args.len()));
                        }
                        let arg_ty = arg_tys[0].clone();
                        if arg_ty != TypeRef::Str {
                            return Err(format!("print_str expects a String, got {:?}", arg_ty));
                        }
                        return Ok(TypeRef::Void);
                    }
                    if name == "write_file" {
                        if args.len() != 2 {
                            return Err(format!("write_file expects 2 arguments, got {}", args.len()));
                        }
                        let arg1_ty = arg_tys[0].clone();
                        let arg2_ty = arg_tys[1].clone();
                        if arg1_ty != TypeRef::Str || arg2_ty != TypeRef::Str {
                            return Err(format!("write_file expects (String, String), got ({:?}, {:?})", arg1_ty, arg2_ty));
                        }
                        return Ok(TypeRef::Void);
                    }
                    if name == "input" {
                        if !args.is_empty() {
                            return Err(format!("input expects 0 arguments, got {}", args.len()));
                        }
                        return Ok(TypeRef::Str);
                    }
                    if name == "read_file" {
                        if args.len() != 1 {
                            return Err(format!("read_file expects 1 argument, got {}", args.len()));
                        }
                        let arg_ty = arg_tys[0].clone();
                        if arg_ty != TypeRef::Str {
                            return Err(format!("read_file expects a String, got {:?}", arg_ty));
                        }
                        return Ok(TypeRef::Str);
                    }
                    if name == "parse_int" {
                        if args.len() != 1 {
                            return Err(format!("parse_int expects 1 argument, got {}", args.len()));
                        }
                        let arg_ty = arg_tys[0].clone();
                        if arg_ty != TypeRef::Str {
                            return Err(format!("parse_int expects a String, got {:?}", arg_ty));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "json_parse" {
                        if args.len() != 1 {
                            return Err(format!("json_parse expects 1 argument, got {}", args.len()));
                        }
                        let arg_ty = arg_tys[0].clone();
                        if arg_ty != TypeRef::Str {
                            return Err(format!("json_parse expects a String, got {:?}", arg_ty));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "json_get_int" {
                        if args.len() != 2 {
                            return Err(format!("json_get_int expects 2 arguments, got {}", args.len()));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "json_get_str" {
                        if args.len() != 2 {
                            return Err(format!("json_get_str expects 2 arguments, got {}", args.len()));
                        }
                        return Ok(TypeRef::Str);
                    }
                    if name == "json_get_obj" {
                        if args.len() != 2 {
                            return Err(format!("json_get_obj expects 2 arguments, got {}", args.len()));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "json_free" {
                        if args.len() != 1 {
                            return Err(format!("json_free expects 1 argument, got {}", args.len()));
                        }
                        return Ok(TypeRef::Void);
                    }
                    if name == "list_new" {
                        if args.len() != 0 {
                            return Err(format!("list_new expects 0 arguments, got {}", args.len()));
                        }
                        if let Some(TypeRef::Generic(list_name, params)) = expected_ty {
                            if list_name == "List" {
                                return Ok(TypeRef::Generic("List".to_string(), params.clone()));
                            }
                        }
                        return Ok(TypeRef::Generic("List".to_string(), vec![TypeRef::Int]));
                    }
                    if name == "list_push" {
                        if args.len() != 2 {
                            return Err(format!("list_push expects 2 arguments, got {}", args.len()));
                        }
                        let list_ty = arg_tys[0].clone();
                        if !matches!(list_ty, TypeRef::Generic(ref name, _) if name == "List") {
                            return Err(format!("list_push first argument must be a List, got {:?}", list_ty));
                        }
                        return Ok(TypeRef::Void);
                    }
                    if name == "list_get" {
                        if args.len() != 2 {
                            return Err(format!("list_get expects 2 arguments, got {}", args.len()));
                        }
                        let list_ty = arg_tys[0].clone();
                        if let TypeRef::Generic(ref name, ref params) = list_ty {
                            if name == "List" && !params.is_empty() {
                                return Ok(params[0].clone());
                            }
                        }
                        return Err(format!("list_get first argument must be a List, got {:?}", list_ty));
                    }
                    if name == "list_len" {
                        if args.len() != 1 {
                            return Err(format!("list_len expects 1 argument, got {}", args.len()));
                        }
                        let list_ty = arg_tys[0].clone();
                        if !matches!(list_ty, TypeRef::Generic(ref name, _) if name == "List") {
                            return Err(format!("list_len argument must be a List, got {:?}", list_ty));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "list_free" {
                        if args.len() != 1 {
                            return Err(format!("list_free expects 1 argument, got {}", args.len()));
                        }
                        let list_ty = arg_tys[0].clone();
                        if !matches!(list_ty, TypeRef::Generic(ref name, _) if name == "List") {
                            return Err(format!("list_free argument must be a List, got {:?}", list_ty));
                        }
                        return Ok(TypeRef::Void);
                    }
                    if name == "net_connect" {
                        if args.len() != 2 {
                            return Err(format!("net_connect expects 2 arguments, got {}", args.len()));
                        }
                        if arg_tys[0] != TypeRef::Str || arg_tys[1] != TypeRef::Int {
                            return Err(format!("net_connect expects (String, Int), got ({:?}, {:?})", arg_tys[0], arg_tys[1]));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "net_listen" {
                        if args.len() != 1 {
                            return Err(format!("net_listen expects 1 argument, got {}", args.len()));
                        }
                        if arg_tys[0] != TypeRef::Int {
                            return Err(format!("net_listen expects (Int), got ({:?})", arg_tys[0]));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "net_accept" {
                        if args.len() != 1 {
                            return Err(format!("net_accept expects 1 argument, got {}", args.len()));
                        }
                        if arg_tys[0] != TypeRef::Int {
                            return Err(format!("net_accept expects (Int), got ({:?})", arg_tys[0]));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "net_send" {
                        if args.len() != 2 {
                            return Err(format!("net_send expects 2 arguments, got {}", args.len()));
                        }
                        if arg_tys[0] != TypeRef::Int || arg_tys[1] != TypeRef::Str {
                            return Err(format!("net_send expects (Int, String), got ({:?}, {:?})", arg_tys[0], arg_tys[1]));
                        }
                        return Ok(TypeRef::Int);
                    }
                    if name == "net_recv" {
                        if args.len() != 2 {
                            return Err(format!("net_recv expects 2 arguments, got {}", args.len()));
                        }
                        if arg_tys[0] != TypeRef::Int || arg_tys[1] != TypeRef::Int {
                            return Err(format!("net_recv expects (Int, Int), got ({:?}, {:?})", arg_tys[0], arg_tys[1]));
                        }
                        return Ok(TypeRef::Str);
                    }
                    if name == "net_close" {
                        if args.len() != 1 {
                            return Err(format!("net_close expects 1 argument, got {}", args.len()));
                        }
                        if arg_tys[0] != TypeRef::Int {
                            return Err(format!("net_close expects (Int), got ({:?})", arg_tys[0]));
                        }
                        return Ok(TypeRef::Void);
                    }
                    if let Some(&id) = self.type_table.structs_by_name.get(name) {
                        return Ok(TypeRef::Custom(id));
                    }
                    if let Some(ty) = self.func_return_types.get(name) {
                        return Ok(ty.clone());
                    }
                }
                Ok(TypeRef::Int) 
            }
            Expr::Closure { params, body, return_type } => {
                let mut closure_scope = None;
                for i in self.closure_scope_idx..self.symbol_table.scopes.len() {
                    if let ScopeKind::Closure { .. } = self.symbol_table.scopes[i].kind {
                        closure_scope = Some(ScopeId(i));
                        self.closure_scope_idx = i + 1;
                        break;
                    }
                }
                
                let scope_id = closure_scope.expect("Closure scope not found");
                
                for param in params {
                    if param.ty.is_none() {
                        let binding_id = self.symbol_table.resolve_name(scope_id, &param.name).unwrap();
                        let binding = &mut self.symbol_table.bindings[binding_id.0];
                        if binding.ty.is_none() {
                            binding.ty = Some(TypeRef::Int);
                        }
                    }
                }
                
                // Traverse body
                for stmt in body {
                    self.infer_stmt(stmt, scope_id)?;
                }
                
                if let Some(t) = return_type {
                    Ok(Self::convert_ast_type(&self.type_table, t))
                } else {
                    let mut inferred_rt = TypeRef::Void;
                    for stmt in body {
                        if let Stmt::Return(Some(expr)) = stmt {
                            if let Ok(ty) = self.infer_expr(expr, scope_id, None) {
                                inferred_rt = ty;
                                break;
                            }
                        }
                    }
                    Ok(inferred_rt)
                }
            }
            Expr::FieldAccess { base, field } => {
                let base_ty = self.infer_expr(base, current_scope, None)?;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_entry) = struct_def.fields.iter().find(|(name, _)| name == field) {
                        return Ok(field_entry.1.clone());
                    }
                    Err(format!("Field '{}' not found on struct '{}'", field, struct_def.name))
                } else {
                    Err(format!("Cannot access field '{}' on non-struct type {:?}", field, base_ty))
                }
            }
            Expr::Spawn { closure } => {
                self.infer_expr(closure, current_scope, None)?;
                Ok(TypeRef::Void)
            }
            Expr::ListLiteral(elements) => {
                let mut elem_ty = TypeRef::Int; // Default if empty
                if !elements.is_empty() {
                    elem_ty = self.infer_expr(&elements[0], current_scope, None)?;
                    for element in elements.iter().skip(1) {
                        self.infer_expr(element, current_scope, None)?;
                    }
                }
                Ok(TypeRef::Generic("List".to_string(), vec![elem_ty]))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TypeChecker;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::semantic::Resolver;

    #[test]
    fn networking_builtins_typecheck_in_lpp_programs() {
        let source = r#"
def main():
    listener := net_listen(9000)
    client := net_accept(listener)
    sent := net_send(client, "hello from lpp")
    payload := net_recv(client, 128)
    print(sent)
    print_str(payload)
    net_close(client)
    net_close(listener)
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("networking program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("networking builtins should typecheck");
    }
}
