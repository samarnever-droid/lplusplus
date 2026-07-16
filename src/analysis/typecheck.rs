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
}

impl<'a> TypeChecker<'a> {
    pub fn new(symbol_table: &'a mut SymbolTable) -> Self {
        Self {
            type_table: TypeTable::new(),
            symbol_table,
            closure_scope_idx: 0,
        }
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
        // Phase 1: Register all struct names (stubs)
        for decl in &program.declarations {
            if let TopLevel::Struct(s) = decl {
                self.type_table.register_struct(s.name.clone());
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
                let inferred_type = self.infer_expr(value, current_scope)?;
                let b_id = binding_id.get().ok_or_else(|| "Binding ID not set".to_string())?;
                let binding = &mut self.symbol_table.bindings[b_id];
                if binding.ty.is_none() {
                    binding.ty = Some(inferred_type);
                }
            }
            Stmt::Assign { name: _, value, binding_id: _ } => {
                self.infer_expr(value, current_scope)?;
            }
            Stmt::If { condition, then_block, else_block } => {
                let cond_ty = self.infer_expr(condition, current_scope)?;
                if cond_ty != TypeRef::Bool {
                    if cond_ty != TypeRef::Int {
                        return Err(format!("'if' condition must be Bool or Int, found {:?}", cond_ty));
                    }
                }
                
                for stmt in then_block {
                    self.infer_stmt(stmt, current_scope)?;
                }
                
                if let Some(else_b) = else_block {
                    for stmt in else_b {
                        self.infer_stmt(stmt, current_scope)?;
                    }
                }
            }
            Stmt::While { condition, body } => {
                let cond_ty = self.infer_expr(condition, current_scope)?;
                if cond_ty != TypeRef::Bool && cond_ty != TypeRef::Int {
                    return Err(format!("'while' condition must be Bool or Int, found {:?}", cond_ty));
                }
                for stmt in body {
                    self.infer_stmt(stmt, current_scope)?;
                }
            }
            Stmt::Expr(expr) => {
                self.infer_expr(expr, current_scope)?;
            }
            Stmt::Return(Some(expr)) => {
                self.infer_expr(expr, current_scope)?;
            }
            Stmt::Return(None) => {}
        }
        Ok(())
    }

    fn infer_expr(&mut self, expr: &Expr, current_scope: ScopeId) -> Result<TypeRef, String> {
        match expr {
            Expr::IntLiteral(_) => Ok(TypeRef::Int),
            Expr::StringLiteral(_) => Ok(TypeRef::Str),
            Expr::Identifier(_, binding_id_cell) => {
                let id = binding_id_cell.get().ok_or_else(|| "Unresolved identifier".to_string())?;
                let binding = &self.symbol_table.bindings[id];
                binding.ty.clone().ok_or_else(|| "Type of identifier not yet inferred".to_string())
            }
            Expr::BinaryOp { left, op, right } => {
                let left_ty = self.infer_expr(left, current_scope)?;
                let right_ty = self.infer_expr(right, current_scope)?;
                if left_ty != right_ty {
                    return Err(format!("Type mismatch in binary operation: {:?} and {:?}", left_ty, right_ty));
                }
                match op {
                    crate::ast::BinaryOperator::Add | crate::ast::BinaryOperator::Subtract | 
                    crate::ast::BinaryOperator::Multiply | crate::ast::BinaryOperator::Divide => Ok(left_ty),
                    crate::ast::BinaryOperator::Eq | crate::ast::BinaryOperator::NotEq |
                    crate::ast::BinaryOperator::Less | crate::ast::BinaryOperator::LessEq |
                    crate::ast::BinaryOperator::Greater | crate::ast::BinaryOperator::GreaterEq => Ok(TypeRef::Bool),
                }
            }
            Expr::Call { callee, args } => {
                for arg in args {
                    self.infer_expr(arg, current_scope)?;
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
                        let arg_ty = self.infer_expr(&args[0], current_scope)?;
                        if arg_ty != TypeRef::Str {
                            return Err(format!("print_str expects a String, got {:?}", arg_ty));
                        }
                        return Ok(TypeRef::Void);
                    }
                    if name == "write_file" {
                        if args.len() != 2 {
                            return Err(format!("write_file expects 2 arguments, got {}", args.len()));
                        }
                        let arg1_ty = self.infer_expr(&args[0], current_scope)?;
                        let arg2_ty = self.infer_expr(&args[1], current_scope)?;
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
                        let arg_ty = self.infer_expr(&args[0], current_scope)?;
                        if arg_ty != TypeRef::Str {
                            return Err(format!("read_file expects a String, got {:?}", arg_ty));
                        }
                        return Ok(TypeRef::Str);
                    }
                    if let Some(&id) = self.type_table.structs_by_name.get(name) {
                        return Ok(TypeRef::Custom(id));
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
                            if let Ok(ty) = self.infer_expr(expr, scope_id) {
                                inferred_rt = ty;
                                break;
                            }
                        }
                    }
                    Ok(inferred_rt)
                }
            }
            Expr::FieldAccess { base, field } => {
                let base_ty = self.infer_expr(base, current_scope)?;
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
                self.infer_expr(closure, current_scope)?;
                // spawn could return a Task handle, but for now we return Void
                Ok(TypeRef::Void)
            }
            Expr::ListLiteral(elements) => {
                let mut elem_ty = TypeRef::Int; // Default if empty
                if !elements.is_empty() {
                    elem_ty = self.infer_expr(&elements[0], current_scope)?;
                    for element in elements.iter().skip(1) {
                        self.infer_expr(element, current_scope)?;
                    }
                }
                Ok(TypeRef::Generic("List".to_string(), vec![elem_ty]))
            }
        }
    }
}
