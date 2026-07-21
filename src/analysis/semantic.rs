use crate::ast::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(pub usize);

#[derive(Debug, Clone)]
pub enum ScopeKind {
    Global,
    Function { name: String },
    Closure { captures: Vec<BindingId> },
    Block,
}

#[derive(Debug, Clone)]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub kind: ScopeKind,
    pub bindings: HashMap<String, BindingId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingKind {
    Local,
    Param,
    StructField, // Unused directly in Scope, but useful in TypeTable later
    FunctionName,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub id: BindingId,
    pub name: String,
    pub declared_in: ScopeId,
    pub ast_ty: Option<Type>,
    pub ty: Option<crate::typecheck::TypeRef>,
    pub is_mut: bool,
    pub kind: BindingKind,
}

#[derive(Debug)]
pub struct SymbolTable {
    pub scopes: Vec<Scope>,
    pub bindings: Vec<Binding>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            bindings: Vec::new(),
        }
    }

    fn new_scope(&mut self, parent: Option<ScopeId>, kind: ScopeKind) -> ScopeId {
        let id = ScopeId(self.scopes.len());
        self.scopes.push(Scope {
            id,
            parent,
            kind,
            bindings: HashMap::new(),
        });
        id
    }

    fn add_binding(
        &mut self,
        scope_id: ScopeId,
        name: String,
        is_mut: bool,
        ast_ty: Option<Type>,
        kind: BindingKind,
    ) -> BindingId {
        let binding_id = BindingId(self.bindings.len());
        self.bindings.push(Binding {
            id: binding_id,
            name: name.clone(),
            declared_in: scope_id,
            ast_ty,
            ty: None,
            is_mut,
            kind,
        });
        self.scopes[scope_id.0].bindings.insert(name, binding_id);
        binding_id
    }

    pub fn resolve_name(&mut self, start_scope: ScopeId, name: &str) -> Option<BindingId> {
        let mut current = Some(start_scope);
        let mut capture_chain: Vec<ScopeId> = Vec::new();

        while let Some(scope_id) = current {
            let scope = &self.scopes[scope_id.0];
            if let Some(&binding_id) = scope.bindings.get(name) {
                // Only capture runtime values (locals, params) — global
                // functions, struct constructors, and builtins are resolved
                // statically and never need an environment edge.
                let binding = &self.bindings[binding_id.0];
                if binding.kind == BindingKind::Local || binding.kind == BindingKind::Param {
                    for closure_scope_id in capture_chain {
                        if let ScopeKind::Closure { ref mut captures } =
                            self.scopes[closure_scope_id.0].kind
                        {
                            if !captures.contains(&binding_id) {
                                captures.push(binding_id);
                            }
                        }
                    }
                }
                return Some(binding_id);
            }

            if let ScopeKind::Closure { .. } = scope.kind {
                capture_chain.push(scope_id);
            }

            current = scope.parent;
        }
        None
    }

    pub fn resolve_name_immutable(&self, start_scope: ScopeId, name: &str) -> Option<BindingId> {
        let mut current = Some(start_scope);
        while let Some(scope_id) = current {
            let scope = &self.scopes[scope_id.0];
            if let Some(&binding_id) = scope.bindings.get(name) {
                return Some(binding_id);
            }
            current = scope.parent;
        }
        None
    }
}

pub struct Resolver {
    pub table: SymbolTable,
    current_scope: ScopeId,
    pub imports: Vec<String>,
    loop_depth: usize,
}

impl Resolver {
    pub fn new() -> Self {
        let mut table = SymbolTable::new();
        let global = table.new_scope(None, ScopeKind::Global);
        Self {
            table,
            current_scope: global,
            imports: Vec::new(),
            loop_depth: 0,
        }
    }

    pub fn resolve_program(&mut self, program: &mut Program) -> Result<(), String> {
        // Register top-level items first so they can be referenced anywhere
        for decl in &program.declarations {
            match decl {
                TopLevel::Function(func) => {
                    self.table.add_binding(
                        self.current_scope,
                        func.name.clone(),
                        false,
                        Some(Type::Custom("Function".into())),
                        BindingKind::FunctionName,
                    );
                }
                TopLevel::Struct(s) => {
                    // Register struct name as a constructor function
                    self.table.add_binding(
                        self.current_scope,
                        s.name.clone(),
                        false,
                        Some(Type::Custom("Function".into())),
                        BindingKind::FunctionName,
                    );
                }
                TopLevel::Import(module) => {
                    if module == "json" {
                        self.imports.push(module.clone());
                        self.table.add_binding(
                            self.current_scope,
                            "json_parse".to_string(),
                            false,
                            Some(Type::Custom("Function".into())),
                            BindingKind::FunctionName,
                        );
                        self.table.add_binding(
                            self.current_scope,
                            "json_get_int".to_string(),
                            false,
                            Some(Type::Custom("Function".into())),
                            BindingKind::FunctionName,
                        );
                        self.table.add_binding(
                            self.current_scope,
                            "json_get_str".to_string(),
                            false,
                            Some(Type::Custom("Function".into())),
                            BindingKind::FunctionName,
                        );
                        self.table.add_binding(
                            self.current_scope,
                            "json_get_obj".to_string(),
                            false,
                            Some(Type::Custom("Function".into())),
                            BindingKind::FunctionName,
                        );
                        self.table.add_binding(
                            self.current_scope,
                            "json_free".to_string(),
                            false,
                            Some(Type::Custom("Function".into())),
                            BindingKind::FunctionName,
                        );
                    } else {
                        // Custom local library module - parsed and merged at driver level
                    }
                }
            }
        }

        // Now walk bodies
        for decl in &mut program.declarations {
            if let TopLevel::Function(func) = decl {
                self.resolve_function(func)?;
            }
        }
        Ok(())
    }

    fn resolve_function(&mut self, func: &mut Function) -> Result<(), String> {
        let parent = self.current_scope;
        let func_scope = self.table.new_scope(
            Some(parent),
            ScopeKind::Function {
                name: func.name.clone(),
            },
        );
        self.current_scope = func_scope;

        for param in &func.params {
            self.table.add_binding(
                self.current_scope,
                param.name.clone(),
                false,
                Some(param.ty.clone()),
                BindingKind::Param,
            );
        }

        for stmt in &mut func.body {
            self.resolve_stmt(stmt)?;
        }

        self.current_scope = parent;
        Ok(())
    }

    fn resolve_stmt(&mut self, stmt: &mut Stmt) -> Result<(), String> {
        match stmt {
            Stmt::LetInferred {
                name,
                is_mut,
                value,
                binding_id,
            } => {
                self.resolve_expr(value)?; // Resolve value before shadowing occurs!
                let id = self.table.add_binding(
                    self.current_scope,
                    name.clone(),
                    *is_mut,
                    None, // Type inference comes next
                    BindingKind::Local,
                );
                binding_id.set(Some(id.0));
            }
            Stmt::Assign {
                name,
                value,
                binding_id,
            } => {
                self.resolve_expr(value)?;
                if let Some(id) = self.table.resolve_name(self.current_scope, name) {
                    binding_id.set(Some(id.0));
                    let binding = &self.table.bindings[id.0];
                    if !binding.is_mut {
                        return Err(format!(
                            "Cannot reassign immutable variable '{}'. Declare it with 'mut {} := ...' to allow mutation.",
                            name, name
                        ));
                    }
                } else {
                    return Err(format!("Assignment to undeclared variable '{}'", name));
                }
            }
            Stmt::AssignField {
                base,
                field: _,
                value,
            } => {
                self.resolve_expr(base)?;
                self.resolve_expr(value)?;
                if let Expr::Identifier(name, ..) = base {
                    if let Some(id) = self.table.resolve_name(self.current_scope, name) {
                        let binding = &self.table.bindings[id.0];
                        if !binding.is_mut {
                            return Err(format!(
                                "Cannot mutate field of immutable variable '{}'. Declare it with 'mut {} := ...' to allow field mutation.",
                                name, name
                            ));
                        }
                    }
                }
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.resolve_expr(condition)?;

                let then_scope = self
                    .table
                    .new_scope(Some(self.current_scope), ScopeKind::Block);
                let old_scope = self.current_scope;
                self.current_scope = then_scope;
                for s in then_block {
                    self.resolve_stmt(s)?;
                }
                self.current_scope = old_scope;

                if let Some(else_b) = else_block {
                    let else_scope = self
                        .table
                        .new_scope(Some(self.current_scope), ScopeKind::Block);
                    self.current_scope = else_scope;
                    for s in else_b {
                        self.resolve_stmt(s)?;
                    }
                    self.current_scope = old_scope;
                }
            }
            Stmt::While { condition, body } => {
                self.resolve_expr(condition)?;
                let body_scope = self
                    .table
                    .new_scope(Some(self.current_scope), ScopeKind::Block);
                let old_scope = self.current_scope;
                self.current_scope = body_scope;
                self.loop_depth += 1;
                for s in body {
                    self.resolve_stmt(s)?;
                }
                self.loop_depth -= 1;
                self.current_scope = old_scope;
            }
            Stmt::ForRange {
                var_name,
                start,
                end,
                body,
                binding_id,
            } => {
                self.resolve_expr(start)?;
                self.resolve_expr(end)?;
                let body_scope = self
                    .table
                    .new_scope(Some(self.current_scope), ScopeKind::Block);
                let old_scope = self.current_scope;
                self.current_scope = body_scope;
                let b_id = self.table.add_binding(
                    self.current_scope,
                    var_name.clone(),
                    true,
                    Some(Type::Int),
                    BindingKind::Local,
                );
                binding_id.set(Some(b_id.0));
                self.loop_depth += 1;
                for s in body {
                    self.resolve_stmt(s)?;
                }
                self.loop_depth -= 1;
                self.current_scope = old_scope;
            }
            Stmt::ForIn {
                var_name,
                list,
                body,
                binding_id,
            } => {
                self.resolve_expr(list)?;
                let body_scope = self
                    .table
                    .new_scope(Some(self.current_scope), ScopeKind::Block);
                let old_scope = self.current_scope;
                self.current_scope = body_scope;
                let b_id = self.table.add_binding(
                    self.current_scope,
                    var_name.clone(),
                    false,
                    None, // Inferred in typecheck from list element type
                    BindingKind::Local,
                );
                binding_id.set(Some(b_id.0));
                self.loop_depth += 1;
                for s in body {
                    self.resolve_stmt(s)?;
                }
                self.loop_depth -= 1;
                self.current_scope = old_scope;
            }
            Stmt::Break | Stmt::Continue => {
                if self.loop_depth == 0 {
                    return Err("Cannot use 'break' or 'continue' outside of a loop".to_string());
                }
            }
            Stmt::Block(stmts) => {
                for s in stmts {
                    self.resolve_stmt(s)?;
                }
            }
            Stmt::Expr(expr) => {
                self.resolve_expr(expr)?;
            }
            Stmt::Return(Some(expr)) => {
                self.resolve_expr(expr)?;
            }
            Stmt::Return(None) => {}
        }
        Ok(())
    }

    fn is_builtin_resolved(&self, name: &str) -> bool {
        if let Some(builtin) = crate::builtins::get_builtins()
            .iter()
            .find(|b| b.name == name)
        {
            if builtin.name.starts_with("json_") {
                return self.imports.iter().any(|imp| imp == "json");
            }
            return true;
        }
        false
    }

    fn resolve_expr(&mut self, expr: &mut Expr) -> Result<(), String> {
        match expr {
            Expr::IntLiteral(_)
            | Expr::FloatLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BoolLiteral(_) => {}
            Expr::Identifier(name, binding_id_cell) => {
                // Ignore builtins for now
                if !self.is_builtin_resolved(name) {
                    if let Some(id) = self.table.resolve_name(self.current_scope, name) {
                        binding_id_cell.set(Some(id.0));
                    } else {
                        return Err(format!("Undeclared identifier '{}'", name));
                    }
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.resolve_expr(left)?;
                self.resolve_expr(right)?;
            }
            Expr::Call { callee, args } => {
                // Check if calling an imported module's function (e.g., json.parse)
                let mut rewritten = None;
                if let Expr::FieldAccess { base, field } = &**callee {
                    if let Expr::Identifier(module_name, _) = &**base {
                        if self.imports.contains(module_name) {
                            rewritten = Some(Expr::Identifier(
                                format!("{}_{}", module_name, field),
                                std::cell::Cell::new(None),
                            ));
                        }
                    }
                }
                if let Some(new_callee) = rewritten {
                    *callee = Box::new(new_callee);
                }

                self.resolve_expr(callee)?;
                for arg in args {
                    self.resolve_expr(arg)?;
                }
            }
            Expr::Closure { params, body, .. } => {
                let parent = self.current_scope;
                let closure_scope = self.table.new_scope(
                    Some(parent),
                    ScopeKind::Closure {
                        captures: Vec::new(),
                    },
                );
                self.current_scope = closure_scope;

                for param in params {
                    self.table.add_binding(
                        self.current_scope,
                        param.name.clone(),
                        false,
                        param.ty.clone(),
                        BindingKind::Param,
                    );
                }

                for s in body {
                    self.resolve_stmt(s)?;
                }

                self.current_scope = parent;
            }
            Expr::FieldAccess { base, .. } => {
                self.resolve_expr(base)?;
            }
            Expr::Spawn { closure } => {
                self.resolve_expr(closure)?;
            }
            Expr::ListLiteral(elements) => {
                for element in elements {
                    self.resolve_expr(element)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Resolver;
    use crate::ast::{Expr, Function, Program, Stmt, TopLevel, Type};

    #[test]
    fn same_scope_shadowing_creates_distinct_bindings() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![
                    Stmt::LetInferred {
                        name: "x".to_string(),
                        is_mut: false,
                        value: Expr::IntLiteral(1),
                        binding_id: std::cell::Cell::new(None),
                    },
                    Stmt::LetInferred {
                        name: "x".to_string(),
                        is_mut: false,
                        value: Expr::IntLiteral(2),
                        binding_id: std::cell::Cell::new(None),
                    },
                ],
            })],
        };

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut program)
            .expect("program should resolve");

        let TopLevel::Function(func) = &program.declarations[0] else {
            panic!("expected function");
        };

        let first = match &func.body[0] {
            Stmt::LetInferred { binding_id, .. } => binding_id.get().expect("first binding id"),
            _ => panic!("expected let statement"),
        };
        let second = match &func.body[1] {
            Stmt::LetInferred { binding_id, .. } => binding_id.get().expect("second binding id"),
            _ => panic!("expected let statement"),
        };

        assert_ne!(first, second, "shadowing should mint a fresh binding");
    }

    #[test]
    fn rejects_reassigning_immutable_variable() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![
                    Stmt::LetInferred {
                        name: "x".to_string(),
                        is_mut: false,
                        value: Expr::IntLiteral(1),
                        binding_id: std::cell::Cell::new(None),
                    },
                    Stmt::Assign {
                        name: "x".to_string(),
                        value: Expr::IntLiteral(2),
                        binding_id: std::cell::Cell::new(None),
                    },
                ],
            })],
        };

        let mut resolver = Resolver::new();
        let err = resolver
            .resolve_program(&mut program)
            .expect_err("should reject immutable assignment");
        assert!(err.contains("Cannot reassign immutable variable 'x'"));
    }

    #[test]
    fn rejects_field_mutation_on_immutable_variable() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![
                    Stmt::LetInferred {
                        name: "box".to_string(),
                        is_mut: false,
                        value: Expr::IntLiteral(1),
                        binding_id: std::cell::Cell::new(None),
                    },
                    Stmt::AssignField {
                        base: Expr::Identifier("box".to_string(), std::cell::Cell::new(None)),
                        field: "val".to_string(),
                        value: Expr::IntLiteral(10),
                    },
                ],
            })],
        };

        let mut resolver = Resolver::new();
        let err = resolver
            .resolve_program(&mut program)
            .expect_err("should reject immutable field mutation");
        assert!(err.contains("Cannot mutate field of immutable variable 'box'"));
    }

    #[test]
    fn rejects_break_outside_loop() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![Stmt::Break],
            })],
        };

        let mut resolver = Resolver::new();
        let err = resolver
            .resolve_program(&mut program)
            .expect_err("should reject break outside loop");
        assert!(err.contains("outside of a loop"));
    }
}
