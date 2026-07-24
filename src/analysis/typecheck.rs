use crate::ast::*;
use crate::semantic::{ScopeId, ScopeKind, SymbolTable};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructTypeId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    Int,
    Float,
    Str,
    Void,
    Bool,
    Custom(StructTypeId),
    Generic(String, Vec<TypeRef>),
    Unresolved(String),
    Function, // Just a placeholder for function names
    TypeParam(String), // Generic type parameter (e.g. T, U) — erased to i64 at codegen
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

    pub fn lookup_struct(&self, name: &str) -> Option<StructTypeId> {
        self.structs_by_name.get(name).copied()
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
    pub block_scope_idx: usize, // BUG-11: tracks Block scopes for if/while bodies
    pub func_return_types: HashMap<String, TypeRef>,
    pub func_param_types: HashMap<String, Vec<TypeRef>>,
    pub trait_names: std::collections::HashSet<String>,
}

/// Check if two types are compatible, treating TypeParam as a wildcard.
fn types_compatible(expected: &TypeRef, actual: &TypeRef) -> bool {
    if expected == actual {
        return true;
    }
    // TypeParam is compatible with any concrete type (type erasure)
    matches!(expected, TypeRef::TypeParam(_)) || matches!(actual, TypeRef::TypeParam(_))
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
            trait_names: std::collections::HashSet::new(),
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
        Self::convert_ast_type_with_params(type_table, ast_ty, &[])
    }

    fn convert_ast_type_with_params(type_table: &TypeTable, ast_ty: &Type, type_params: &[String]) -> TypeRef {
        match ast_ty {
            Type::Int => TypeRef::Int,
            Type::Float => TypeRef::Float,
            Type::String => TypeRef::Str,
            Type::Bool => TypeRef::Bool,
            Type::Void => TypeRef::Void,
            Type::Custom(name) => {
                // Check if this is a type parameter first
                if type_params.iter().any(|tp| tp == name) {
                    return TypeRef::TypeParam(name.clone());
                }
                if let Some(&id) = type_table.structs_by_name.get(name) {
                    TypeRef::Custom(id)
                } else {
                    TypeRef::Unresolved(name.clone())
                }
            }
            Type::Generic(base_name, args) => {
                let mut ref_args = Vec::new();
                for arg in args {
                    ref_args.push(Self::convert_ast_type_with_params(type_table, arg, type_params));
                }
                TypeRef::Generic(base_name.clone(), ref_args)
            }
        }
    }

    fn verify_struct_cycles(type_table: &TypeTable) -> Result<(), String> {
        use std::collections::HashSet;
        fn reaches(
            type_table: &TypeTable,
            target: StructTypeId,
            current: StructTypeId,
            visited: &mut HashSet<StructTypeId>,
        ) -> bool {
            for (_, field_ty) in &type_table.definitions[current.0].fields {
                let next = match field_ty {
                    TypeRef::Custom(next) => Some(*next),
                    TypeRef::Generic(name, args) if name == "List" && args.len() == 1 => {
                        match args[0] {
                            TypeRef::Custom(next) => Some(next),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(next) = next {
                    if next == target {
                        return true;
                    }
                    if visited.insert(next) && reaches(type_table, target, next, visited) {
                        return true;
                    }
                }
            }
            false
        }

        for index in 0..type_table.definitions.len() {
            let id = StructTypeId(index);
            let mut visited = HashSet::new();
            if reaches(type_table, id, id, &mut visited) {
                let name = &type_table.definitions[index].name;
                return Err(format!(
                    "Type error: Cyclic owned struct '{}' detected. ARC cannot reclaim ownership cycles without explicit cycle collection.",
                    name
                ));
            }
        }
        Ok(())
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), String> {
        // Phase 0.5: Collect trait names
        for decl in &program.declarations {
            if let TopLevel::Trait(t) = decl {
                self.trait_names.insert(t.name.clone());
            }
        }

        // Phase 1: Register all struct and enum names (stubs) and map function return types
        for decl in &program.declarations {
            if let TopLevel::Struct(s) = decl {
                self.type_table.register_struct(s.name.clone());
            }
            if let TopLevel::Enum(e) = decl {
                // Register enum as a custom type (like a struct)
                self.type_table.register_struct(e.name.clone());
            }
        }
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                let tp = &f.type_params;
                let ret_ty = Self::convert_ast_type_with_params(&self.type_table, &f.return_type, tp);
                self.func_return_types.insert(f.name.clone(), ret_ty);
                let param_tys: Vec<TypeRef> = f
                    .params
                    .iter()
                    .map(|p| Self::convert_ast_type_with_params(&self.type_table, &p.ty, tp))
                    .collect();
                self.func_param_types.insert(f.name.clone(), param_tys);
            }
            // Register impl method types (they are mangled as TargetType_method)
            if let TopLevel::Impl(impl_block) = decl {
                for method in &impl_block.methods {
                    let tp = &method.type_params;
                    let ret_ty = Self::convert_ast_type_with_params(&self.type_table, &method.return_type, tp);
                    self.func_return_types.insert(method.name.clone(), ret_ty);
                    let param_tys: Vec<TypeRef> = method
                        .params
                        .iter()
                        .map(|p| Self::convert_ast_type_with_params(&self.type_table, &p.ty, tp))
                        .collect();
                    self.func_param_types.insert(method.name.clone(), param_tys);
                }
            }
        }

        // Phase 2: Resolve struct fields and check for self-reference
        for decl in &program.declarations {
            if let TopLevel::Struct(s) = decl {
                let id = *self
                    .type_table
                    .structs_by_name
                    .get(&s.name)
                    .ok_or_else(|| format!("Type error: Unknown struct definition '{}'", s.name))?;

                let mut resolved_fields = Vec::new();
                let mut is_self_referential = false;

                for field in &s.fields {
                    let field_ty = Self::convert_ast_type_with_params(&self.type_table, &field.ty, &s.type_params);

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

        // Check for cyclic ownership graphs in custom types
        Self::verify_struct_cycles(&self.type_table)?;

        // Collect all type parameter names from all generic functions/structs/enums
        let mut all_type_params: Vec<String> = Vec::new();
        for decl in &program.declarations {
            match decl {
                TopLevel::Function(f) => all_type_params.extend(f.type_params.clone()),
                TopLevel::Struct(s) => all_type_params.extend(s.type_params.clone()),
                TopLevel::Enum(e) => all_type_params.extend(e.type_params.clone()),
                _ => {}
            }
        }
        // Also treat trait names as type params so they resolve to TypeParam (→ i64)
        for tn in &self.trait_names {
            all_type_params.push(tn.clone());
        }
        all_type_params.sort();
        all_type_params.dedup();

        // Phase 3: Update all bindings in the symbol table with resolved TypeRefs
        for binding in &mut self.symbol_table.bindings {
            if let Some(ast_ty) = &binding.ast_ty {
                binding.ty = Some(Self::convert_ast_type_with_params(&self.type_table, ast_ty, &all_type_params));
            }
        }

        // Phase 4: Local Type Inference
        // Collect all functions: top-level + impl methods
        let mut all_funcs: Vec<&Function> = Vec::new();
        for decl in &program.declarations {
            if let TopLevel::Function(func) = decl {
                all_funcs.push(func);
            }
            if let TopLevel::Impl(impl_block) = decl {
                for method in &impl_block.methods {
                    all_funcs.push(method);
                }
            }
        }
        for func in &all_funcs {
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

        Ok(())
    }

    fn infer_stmt(&mut self, stmt: &Stmt, current_scope: ScopeId) -> Result<(), String> {
        match stmt {
            Stmt::LetInferred {
                name: _,
                is_mut: _,
                value,
                binding_id,
            } => {
                let inferred_type = self.infer_expr(value, current_scope, None)?;
                let b_id = binding_id
                    .get()
                    .ok_or_else(|| "Binding ID not set".to_string())?;
                let binding = &mut self.symbol_table.bindings[b_id];
                if binding.ty.is_none() {
                    binding.ty = Some(inferred_type);
                }
            }
            Stmt::Assign {
                name,
                value,
                binding_id,
            } => {
                let expected_ty = if let Some(b_id) = binding_id.get() {
                    self.symbol_table.bindings[b_id].ty.clone()
                } else if let Some(b_id) = self.symbol_table.resolve_name(current_scope, name) {
                    binding_id.set(Some(b_id.0));
                    self.symbol_table.bindings[b_id.0].ty.clone()
                } else {
                    None
                };
                let val_ty = self.infer_expr(value, current_scope, expected_ty.clone())?;
                if let Some(exp) = expected_ty {
                    if !types_compatible(&exp, &val_ty) {
                        return Err(format!(
                            "Type mismatch in assignment: cannot assign '{:?}' to variable '{}' of type '{:?}'",
                            val_ty, name, exp
                        ));
                    }
                }
            }
            Stmt::AssignField { base, field, value } => {
                let base_ty = self.infer_expr(base, current_scope, None)?;
                let mut expected_ty = None;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_entry) =
                        struct_def.fields.iter().find(|(name, _)| name == field)
                    {
                        expected_ty = Some(field_entry.1.clone());
                    }
                }
                let val_ty = self.infer_expr(value, current_scope, expected_ty)?;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_entry) =
                        struct_def.fields.iter().find(|(name, _)| name == field)
                    {
                        if !types_compatible(&field_entry.1, &val_ty) {
                            return Err(format!(
                                "Type mismatch in field assignment: expected {:?}, got {:?}",
                                field_entry.1, val_ty
                            ));
                        }
                    } else {
                        return Err(format!(
                            "Field '{}' not found on struct '{}'",
                            field, struct_def.name
                        ));
                    }
                } else {
                    return Err(format!(
                        "Cannot access field '{}' on non-struct type {:?}",
                        field, base_ty
                    ));
                }
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                let cond_ty = self.infer_expr(condition, current_scope, None)?;
                if cond_ty != TypeRef::Bool {
                    if cond_ty != TypeRef::Int {
                        return Err(format!(
                            "'if' condition must be Bool or Int, found {:?}",
                            cond_ty
                        ));
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
                    return Err(format!(
                        "'while' condition must be Bool or Int, found {:?}",
                        cond_ty
                    ));
                }

                // BUG-11: use the while body's own block scope
                let body_scope = self.next_block_scope(current_scope);
                for stmt in body {
                    self.infer_stmt(stmt, body_scope)?;
                }
            }
            Stmt::ForRange {
                var_name: _,
                start,
                end,
                step: _,
                body,
                binding_id,
            } => {
                let start_ty = self.infer_expr(start, current_scope, None)?;
                let end_ty = self.infer_expr(end, current_scope, None)?;
                if start_ty != TypeRef::Int || end_ty != TypeRef::Int {
                    return Err(format!(
                        "'for range' boundaries must be Int, found {:?} and {:?}",
                        start_ty, end_ty
                    ));
                }
                if let Some(ast_id) = binding_id.get() {
                    self.symbol_table.bindings[ast_id].ty = Some(TypeRef::Int);
                }
                let body_scope = self.next_block_scope(current_scope);
                for stmt in body {
                    self.infer_stmt(stmt, body_scope)?;
                }
            }
            Stmt::ForIn {
                var_name: _,
                list,
                body,
                binding_id,
            } => {
                let list_ty = self.infer_expr(list, current_scope, None)?;
                let elem_ty = match list_ty {
                    TypeRef::Generic(ref name, ref params) if name == "List" && !params.is_empty() => {
                        params[0].clone()
                    }
                    _ => TypeRef::Int,
                };
                if let Some(ast_id) = binding_id.get() {
                    self.symbol_table.bindings[ast_id].ty = Some(elem_ty);
                }
                let body_scope = self.next_block_scope(current_scope);
                for stmt in body {
                    self.infer_stmt(stmt, body_scope)?;
                }
            }
            Stmt::Block(stmts) => {
                for stmt in stmts {
                    self.infer_stmt(stmt, current_scope)?;
                }
            }
            Stmt::Expr(expr) => {
                self.infer_expr(expr, current_scope, None)?;
            }
            Stmt::Break | Stmt::Continue => {}
            Stmt::Match { subject, arms } => {
                self.infer_expr(subject, current_scope, None)?;
                for arm in arms {
                    for s in &arm.body {
                        self.infer_stmt(s, current_scope)?;
                    }
                }
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

    fn infer_expr(
        &mut self,
        expr: &Expr,
        current_scope: ScopeId,
        expected_ty: Option<TypeRef>,
    ) -> Result<TypeRef, String> {
        match expr {
            Expr::IntLiteral(_) => Ok(TypeRef::Int),
            Expr::FloatLiteral(_) => Ok(TypeRef::Float),
            Expr::StringLiteral(_) => Ok(TypeRef::Str),
            Expr::BoolLiteral(_) => Ok(TypeRef::Bool),
            Expr::Identifier(name, binding_id_cell) => {
                if let Some(id) = binding_id_cell.get() {
                    let binding = &self.symbol_table.bindings[id];
                    binding
                        .ty
                        .clone()
                        .ok_or_else(|| "Type of identifier not yet inferred".to_string())
                } else {
                    // BUG-05: Builtin identifiers have no binding_id (semantic resolver skips them).
                    // Return their known types instead of panicking with "Unresolved identifier".
                    match name.as_str() {
                        "input" | "read_file" | "json_get_str" | "net_recv" => Ok(TypeRef::Str),
                        "print" | "print_str" | "write_file" | "json_free" | "list_push"
                        | "list_free" | "net_close" | "map_put" | "map_remove" => Ok(TypeRef::Void),
                        "parse_int" | "json_parse" | "json_get_int" | "json_get_obj"
                        | "list_get" | "list_len" | "file_size" | "file_copy" | "file_move"
                        | "net_connect" | "net_listen" | "net_accept" | "net_send"
                        | "net_send_all" | "net_set_timeout" | "map_len" => Ok(TypeRef::Int),
                        "map_has" => Ok(TypeRef::Bool),
                        "list_new" => Ok(TypeRef::Generic("List".to_string(), vec![TypeRef::Int])),
                        "map_new" => Ok(TypeRef::Generic("Map".to_string(), vec![TypeRef::Int, TypeRef::Int])),
                        _ => Err(format!("Unresolved identifier '{}'", name)),
                    }
                }
            }
            Expr::UnaryOp { op, operand } => {
                let ty = self.infer_expr(operand, current_scope, None)?;
                match op {
                    UnaryOperator::Negate => Ok(ty), // -Int→Int, -Float→Float
                    UnaryOperator::Not => Ok(TypeRef::Bool), // !Bool→Bool
                }
            }
            Expr::BinaryOp { left, op, right } => {
                let left_ty = self.infer_expr(left, current_scope, None)?;
                let right_ty = self.infer_expr(right, current_scope, None)?;
                let is_ptr_null_check = match (&left_ty, &right_ty) {
                    (&TypeRef::Custom(_), &TypeRef::Int) | (&TypeRef::Int, &TypeRef::Custom(_)) => {
                        matches!(
                            op,
                            crate::ast::BinaryOperator::Eq | crate::ast::BinaryOperator::NotEq
                        )
                    }
                    _ => false,
                };
                if left_ty != right_ty && !is_ptr_null_check {
                    return Err(format!(
                        "Type mismatch in binary operation: {:?} and {:?}",
                        left_ty, right_ty
                    ));
                }
                match op {
                    crate::ast::BinaryOperator::Add
                    | crate::ast::BinaryOperator::Subtract
                    | crate::ast::BinaryOperator::Multiply
                    | crate::ast::BinaryOperator::Divide
                    | crate::ast::BinaryOperator::Modulo
                    | crate::ast::BinaryOperator::BitAnd
                    | crate::ast::BinaryOperator::BitOr
                    | crate::ast::BinaryOperator::BitXor
                    | crate::ast::BinaryOperator::Shl
                    | crate::ast::BinaryOperator::Shr => Ok(left_ty),
                    crate::ast::BinaryOperator::Eq
                    | crate::ast::BinaryOperator::NotEq
                    | crate::ast::BinaryOperator::Less
                    | crate::ast::BinaryOperator::LessEq
                    | crate::ast::BinaryOperator::Greater
                    | crate::ast::BinaryOperator::GreaterEq
                    | crate::ast::BinaryOperator::And
                    | crate::ast::BinaryOperator::Or => Ok(TypeRef::Bool),
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
                    } else if (name == "list_get" || name == "get") && args.len() >= 2 {
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
                    if let Some(builtin) = crate::builtins::get_builtins()
                        .iter()
                        .find(|b| b.name == name)
                    {
                        if builtin.params.len() != args.len()
                            && !builtin
                                .params
                                .iter()
                                .any(|p| matches!(p, crate::builtins::ParamType::Any))
                        {
                            return Err(format!(
                                "{} expects {} arguments, got {}",
                                name,
                                builtin.params.len(),
                                args.len()
                            ));
                        }

                        for (i, param) in builtin.params.iter().enumerate() {
                            match param {
                                crate::builtins::ParamType::Specific(expected_ty) => {
                                    let arg_ty = &arg_tys[i];
                                    if !types_compatible(expected_ty, arg_ty) {
                                        if let TypeRef::Generic(expected_name, _) = expected_ty {
                                            if let TypeRef::Generic(arg_name, _) = arg_ty {
                                                if expected_name == arg_name {
                                                    continue;
                                                }
                                            }
                                        }
                                        return Err(format!(
                                            "{} expects parameter {} to be {:?}, got {:?}",
                                            name,
                                            i + 1,
                                            expected_ty,
                                            arg_ty
                                        ));
                                    }
                                }
                                crate::builtins::ParamType::Any => {}
                            }
                        }

                        if name == "list_new" {
                            if let Some(TypeRef::Generic(list_name, params)) = expected_ty {
                                if list_name == "List" {
                                    if params.len() != 1 {
                                        return Err(
                                            "List requires exactly one element type".to_string()
                                        );
                                    }
                                    return Ok(TypeRef::Generic(
                                        "List".to_string(),
                                        params.clone(),
                                    ));
                                }
                            }
                            return Ok(TypeRef::Generic("List".to_string(), vec![TypeRef::Int]));
                        }

                        if name == "list_get" || name == "lpp_list_get" || name == "get" {
                            let list_ty = arg_tys[0].clone();
                            if let TypeRef::Generic(ref name, ref params) = list_ty {
                                if name == "List" && !params.is_empty() {
                                    return Ok(params[0].clone());
                                }
                            }
                            return Err(format!(
                                "list_get first argument must be a List, got {:?}",
                                list_ty
                            ));
                        }

                        if name == "map_new" || name == "lpp_map_new" {
                            if let Some(TypeRef::Generic(map_name, params)) = expected_ty {
                                if map_name == "Map" && params.len() == 2 {
                                    return Ok(TypeRef::Generic("Map".to_string(), params));
                                }
                            }
                            return Ok(TypeRef::Generic("Map".to_string(), vec![TypeRef::Int, TypeRef::Int]));
                        }

                        if name == "map_put" || name == "lpp_map_put" {
                            if args.len() >= 3 {
                                let key_ty = arg_tys[1].clone();
                                let val_ty = arg_tys[2].clone();
                                if let Expr::Identifier(_, ref cell) = args[0] {
                                    if let Some(id) = cell.get() {
                                        self.symbol_table.bindings[id].ty = Some(TypeRef::Generic(
                                            "Map".to_string(),
                                            vec![key_ty, val_ty],
                                        ));
                                    }
                                }
                            }
                            return Ok(TypeRef::Void);
                        }

                        if name == "map_get" || name == "lpp_map_get" {
                            let map_ty = arg_tys[0].clone();
                            if let TypeRef::Generic(ref name, ref params) = map_ty {
                                if name == "Map" && params.len() >= 2 {
                                    return Ok(params[1].clone());
                                }
                            }
                            return Ok(TypeRef::Int);
                        }

                        return Ok(builtin.return_type.clone());
                    }

                    if let Some(&id) = self.type_table.structs_by_name.get(name) {
                        let def = &self.type_table.definitions[id.0];
                        if !args.is_empty() {
                            if args.len() != def.fields.len() {
                                return Err(format!(
                                    "Struct '{}' constructor expects {} arguments or 0, got {}",
                                    def.name,
                                    def.fields.len(),
                                    args.len()
                                ));
                            }
                            for (i, (field_name, field_ty)) in def.fields.iter().enumerate() {
                                let arg_ty = &arg_tys[i];
                                if !types_compatible(field_ty, arg_ty) {
                                    return Err(format!(
                                        "Struct '{}' field '{}' expects {:?}, got {:?}",
                                        def.name, field_name, field_ty, arg_ty
                                    ));
                                }
                            }
                        }
                        return Ok(TypeRef::Custom(id));
                    }
                    if let Some(ty) = self.func_return_types.get(name) {
                        // Generic type inference: if return type is TypeParam,
                        // substitute it based on the actual argument types
                        if let TypeRef::TypeParam(tp_name) = ty {
                            if let Some(param_types) = self.func_param_types.get(name) {
                                for (i, pt) in param_types.iter().enumerate() {
                                    if let TypeRef::TypeParam(pn) = pt {
                                        if pn == tp_name && i < arg_tys.len() {
                                            return Ok(arg_tys[i].clone());
                                        }
                                    }
                                }
                            }
                        }
                        return Ok(ty.clone());
                    }
                    // Trait method dispatch: try StructName_method
                    if !arg_tys.is_empty() {
                        if let TypeRef::Custom(sid) = &arg_tys[0] {
                            let struct_name = &self.type_table.definitions[sid.0].name;
                            let mangled = format!("{}_{}", struct_name, name);
                            if let Some(ty) = self.func_return_types.get(&mangled) {
                                return Ok(ty.clone());
                            }
                        }
                    }
                }
                Ok(TypeRef::Int)
            }
            Expr::Closure {
                params,
                body,
                return_type,
            } => {
                let mut closure_scope = None;
                for i in self.closure_scope_idx..self.symbol_table.scopes.len() {
                    if let ScopeKind::Closure { .. } = self.symbol_table.scopes[i].kind {
                        closure_scope = Some(ScopeId(i));
                        self.closure_scope_idx = i + 1;
                        break;
                    }
                }

                let scope_id = closure_scope
                    .ok_or_else(|| "Type error: Closure scope resolution failed".to_string())?;

                for param in params {
                    if param.ty.is_none() {
                        let binding_id = self
                            .symbol_table
                            .resolve_name(scope_id, &param.name)
                            .ok_or_else(|| {
                                format!("Type error: Unresolved closure parameter '{}'", param.name)
                            })?;
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

                // Check an explicit/inferred result type for diagnostics, but
                // the expression itself is a callable ownership capsule, not
                // the value it will eventually return when invoked.
                if let Some(t) = return_type {
                    let _ = Self::convert_ast_type(&self.type_table, t);
                } else {
                    for stmt in body {
                        if let Stmt::Return(Some(expr)) = stmt {
                            let _ = self.infer_expr(expr, scope_id, None)?;
                            break;
                        }
                    }
                }
                Ok(TypeRef::Function)
            }
            Expr::FieldAccess { base, field } => {
                let base_ty = self.infer_expr(base, current_scope, None)?;
                if let TypeRef::Custom(struct_id) = &base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    // Check if it's a regular struct field
                    if let Some(field_entry) =
                        struct_def.fields.iter().find(|(name, _)| name == field)
                    {
                        return Ok(field_entry.1.clone());
                    }
                    // Check if it's an enum variant (enum has no fields in type table)
                    if struct_def.fields.is_empty() {
                        // Likely an enum — treat field access as variant constructor
                        return Ok(TypeRef::Custom(*struct_id));
                    }
                    Err(format!(
                        "Field '{}' not found on struct '{}'",
                        field, struct_def.name
                    ))
                } else {
                    // Could be an enum accessed via FunctionName binding
                    // Check if base is an identifier that matches a registered enum
                    if let Expr::Identifier(name, _) = base.as_ref() {
                        if let Some(id) = self.type_table.lookup_struct(name) {
                            return Ok(TypeRef::Custom(id));
                        }
                    }
                    Err(format!(
                        "Cannot access field '{}' on non-struct type {:?}",
                        field, base_ty
                    ))
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
                }
                for element in elements.iter().skip(1) {
                    let actual_ty = self.infer_expr(element, current_scope, None)?;
                    if actual_ty != elem_ty {
                        return Err(format!(
                            "list literal has mixed element types: expected {:?}, got {:?}",
                            elem_ty, actual_ty
                        ));
                    }
                }
                if !matches!(elem_ty, TypeRef::Int | TypeRef::Float | TypeRef::Custom(_) | TypeRef::Str | TypeRef::Bool) {
                    return Err(format!(
                        "List element type {:?} is not supported safely yet",
                        elem_ty
                    ));
                }
                Ok(TypeRef::Generic("List".to_string(), vec![elem_ty]))
            }
            Expr::Match { subject, arms } => {
                let _subject_ty = self.infer_expr(subject, current_scope, None)?;
                // For now, match returns Void (statement-level match)
                // Type-check each arm body
                for arm in arms {
                    for stmt in &arm.body {
                        self.infer_stmt(stmt, current_scope)?;
                    }
                }
                Ok(TypeRef::Void)
            }
            Expr::EnumVariantConstruct { enum_name, .. } => {
                // Returns the enum type
                if let Some(id) = self.type_table.lookup_struct(enum_name) {
                    Ok(TypeRef::Custom(id))
                } else {
                    Ok(TypeRef::Int)
                }
            }
            Expr::Try(inner) => {
                let _inner_ty = self.infer_expr(inner, current_scope, None)?;
                Ok(TypeRef::Int)
            }
            Expr::Index { base, index } => {
                let base_ty = self.infer_expr(base, current_scope, None)?;
                self.infer_expr(index, current_scope, None)?;
                match base_ty {
                    TypeRef::Str => Ok(TypeRef::Str), // str[i] → single char as Str
                    TypeRef::Generic(ref name, _) if name == "List" => Ok(TypeRef::Int),
                    _ => Ok(TypeRef::Int), // fallback
                }
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

    #[test]
    fn boolean_literals_typecheck() {
        let source = r#"
def main():
    mut b := true
    b = false
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("boolean program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("boolean program should typecheck");
    }

    #[test]
    fn map_operations_typecheck() {
        let source = r#"
def main():
    mut m := map_new()
    map_put(m, "apple", 100)
    map_put(m, "banana", 200)

    if map_has(m, "apple"):
        val := map_get(m, "apple")
        lpp_print_int(val)

    lpp_print_int(map_len(m))
    map_remove(m, "apple")
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("map operations should typecheck");
    }

    #[test]
    fn positional_struct_constructor_typechecks() {
        let source = r#"
struct Point:
    x: Int
    y: Int

def main():
    p := Point(10, 20)
    lpp_print_int(p.x)
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("positional struct constructor should typecheck");
    }

    #[test]
    fn rejects_cyclic_owned_structs() {
        let source = r#"
struct Node:
    next: Node

def main():
    print(0)
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        let err = type_checker
            .check_program(&ast)
            .expect_err("cyclic struct should fail typecheck");
        assert!(err.contains("Cyclic owned struct 'Node' detected"));
    }
}
