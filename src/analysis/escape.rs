use std::collections::HashMap;
use crate::ast::*;
use crate::semantic::{SymbolTable, ScopeId, ScopeKind, BindingId};
use crate::typecheck::{TypeTable, TypeRef, StructTypeId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageClass {
    Value,
    Arc,
    Arena { region: StructTypeId },
}

pub struct EscapeAnalyzer;

impl EscapeAnalyzer {
    fn promote(storage: &mut HashMap<BindingId, StorageClass>, binding_id: BindingId, new_class: StorageClass) {
        let current = storage.entry(binding_id).or_insert(StorageClass::Value);
        
        match (&current, &new_class) {
            (StorageClass::Value, _) => {
                *current = new_class;
            }
            (StorageClass::Arc, StorageClass::Arena { .. }) => {
                *current = new_class;
            }
            _ => {}
        }
    }

    pub fn analyze(
        program: &Program,
        symbol_table: &SymbolTable,
        type_table: &TypeTable,
    ) -> Result<HashMap<BindingId, StorageClass>, String> {
        let mut storage = HashMap::new();

        for binding in &symbol_table.bindings {
            storage.insert(binding.id, StorageClass::Value);
        }

        for scope in &symbol_table.scopes {
            if let ScopeKind::Closure { captures } = &scope.kind {
                for &captured_id in captures {
                    let binding = &symbol_table.bindings[captured_id.0];
                    let is_struct = matches!(binding.ty, Some(TypeRef::Custom(_)));
                    let is_mut = binding.is_mut;

                    // Rule 2 & 4: If a reference crosses a concurrency boundary or escaping closure,
                    // it is promoted to Arc if it's mutable (so edits are shared) or if it's a struct (references).
                    // Pure immutable scalars are safely cloned by value and remain on the stack.
                    if is_struct || is_mut {
                        Self::promote(&mut storage, captured_id, StorageClass::Arc);
                    }
                }
            }
        }

        for binding in &symbol_table.bindings {
            if let Some(TypeRef::Custom(struct_id)) = &binding.ty {
                if type_table.definitions[struct_id.0].is_self_referential {
                    Self::promote(&mut storage, binding.id, StorageClass::Arena { region: *struct_id });
                }
            }
        }

        let closure_scopes: Vec<ScopeId> = symbol_table.scopes.iter()
            .filter(|s| matches!(s.kind, ScopeKind::Closure { .. }))
            .map(|s| s.id)
            .collect();
        
        let mut closure_idx = 0;

        let mut func_scopes = HashMap::new();
        for scope in &symbol_table.scopes {
            if let ScopeKind::Function { name } = &scope.kind {
                func_scopes.insert(name.clone(), scope.id);
            }
        }

        for decl in &program.declarations {
            if let TopLevel::Function(func) = decl {
                if let Some(&scope_id) = func_scopes.get(&func.name) {
                    for stmt in &func.body {
                        Self::walk_stmt_rule1(stmt, scope_id, symbol_table, type_table, &closure_scopes, &mut closure_idx, &mut storage)?;
                    }
                }
            }
        }

        Ok(storage)
    }

    fn get_root_binding(mut expr: &Expr, _current_scope: ScopeId, _symbol_table: &SymbolTable) -> Option<BindingId> {
        loop {
            match expr {
                Expr::Identifier(_, binding_id_cell) => {
                    return binding_id_cell.get().map(|id| BindingId(id));
                }
                Expr::FieldAccess { base, .. } => {
                    expr = base;
                }
                _ => return None,
            }
        }
    }

    fn get_expr_type(
        expr: &Expr,
        current_scope: ScopeId,
        symbol_table: &SymbolTable,
        type_table: &TypeTable
    ) -> Option<TypeRef> {
        match expr {
            Expr::Identifier(_, _) => {
                let root_id = Self::get_root_binding(expr, current_scope, symbol_table)?;
                symbol_table.bindings[root_id.0].ty.clone()
            }
            Expr::FieldAccess { base, field } => {
                let base_ty = Self::get_expr_type(base, current_scope, symbol_table, type_table)?;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &type_table.definitions[struct_id.0];
                    if let Some(param) = struct_def.fields.iter().find(|(n, _)| n == field) {
                        return Some(param.1.clone());
                    }
                }
                None
            }
            _ => None
        }
    }

    fn walk_stmt_rule1(
        stmt: &Stmt, 
        current_scope: ScopeId, 
        symbol_table: &SymbolTable,
        type_table: &TypeTable,
        closure_scopes: &[ScopeId], 
        closure_idx: &mut usize,
        storage: &mut HashMap<BindingId, StorageClass>
    ) -> Result<(), String> {
        match stmt {
            Stmt::LetInferred { value, .. } => {
                Self::walk_expr_rule1(value, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
            }
            Stmt::Assign { value, .. } => {
                Self::walk_expr_rule1(value, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
            }
            Stmt::AssignField { base, field: _, value } => {
                Self::walk_expr_rule1(base, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                Self::walk_expr_rule1(value, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
            }
            Stmt::If { condition, then_block, else_block } => {
                Self::walk_expr_rule1(condition, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                for stmt in then_block {
                    Self::walk_stmt_rule1(stmt, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                }
                if let Some(else_b) = else_block {
                    for stmt in else_b {
                        Self::walk_stmt_rule1(stmt, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                    }
                }
            }
            Stmt::While { condition, body } => {
                Self::walk_expr_rule1(condition, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                for stmt in body {
                    Self::walk_stmt_rule1(stmt, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                }
            }
            Stmt::Expr(expr) => {
                Self::walk_expr_rule1(expr, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
            }
            Stmt::Return(Some(expr)) => {
                let mut should_promote = false;
                if let Some(ty) = Self::get_expr_type(expr, current_scope, symbol_table, type_table) {
                    if matches!(ty, TypeRef::Custom(_) | TypeRef::Generic(_, _) | TypeRef::Str) {
                        should_promote = true;
                    }
                }

                if should_promote {
                    if let Some(binding_id) = Self::get_root_binding(expr, current_scope, symbol_table) {
                        let binding = &symbol_table.bindings[binding_id.0];
                        if !matches!(symbol_table.scopes[binding.declared_in.0].kind, ScopeKind::Global) {
                            Self::promote(storage, binding_id, StorageClass::Arc);
                        }
                    }
                }
                
                Self::walk_expr_rule1(expr, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
            }
            Stmt::Return(None) => {}
        }
        Ok(())
    }

    fn walk_expr_rule1(
        expr: &Expr, 
        current_scope: ScopeId, 
        symbol_table: &SymbolTable,
        type_table: &TypeTable,
        closure_scopes: &[ScopeId], 
        closure_idx: &mut usize,
        storage: &mut HashMap<BindingId, StorageClass>
    ) -> Result<(), String> {
        match expr {
            Expr::BinaryOp { left, right, .. } => {
                Self::walk_expr_rule1(left, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                Self::walk_expr_rule1(right, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
            }
            Expr::Call { callee, args } => {
                Self::walk_expr_rule1(callee, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                for arg in args {
                    Self::walk_expr_rule1(arg, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                }
            }
            Expr::Closure { body, .. } => {
                let scope_id = closure_scopes[*closure_idx];
                *closure_idx += 1;
                
                for stmt in body {
                    Self::walk_stmt_rule1(stmt, scope_id, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                }
            }
            Expr::FieldAccess { base, .. } => {
                Self::walk_expr_rule1(base, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
            }
            Expr::Spawn { closure } => {
                // Rule 4: Crossing a concurrency boundary.
                Self::walk_expr_rule1(closure, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
            }
            Expr::ListLiteral(elements) => {
                for element in elements {
                    let mut should_promote = false;
                    if let Some(ty) = Self::get_expr_type(element, current_scope, symbol_table, type_table) {
                        if matches!(ty, TypeRef::Custom(_) | TypeRef::Generic(_, _) | TypeRef::Str) {
                            should_promote = true;
                        }
                    }

                    if should_promote {
                        // Rule 3: Unbounded lifetime container.
                        if let Some(binding_id) = Self::get_root_binding(element, current_scope, symbol_table) {
                            let binding = &symbol_table.bindings[binding_id.0];
                            if !matches!(symbol_table.scopes[binding.declared_in.0].kind, ScopeKind::Global) {
                                Self::promote(storage, binding_id, StorageClass::Arc);
                            }
                        }
                    }
                    Self::walk_expr_rule1(element, current_scope, symbol_table, type_table, closure_scopes, closure_idx, storage)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
