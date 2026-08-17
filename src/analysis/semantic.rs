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
    pub ty: Option<crate::types::TypeRef>,
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
    /// Short method names from impl blocks (e.g. "val" from "impl GetVal for Box")
    pub trait_method_names: std::collections::HashSet<String>,
    /// Variant name → declared payload type, so a match arm binds the payload
    /// at its real type. Binding everything as Int used to make
    /// `match m: F(v): float_to_str(v)` fail with "expected Float, got Int".
    variant_payload_types: std::collections::HashMap<String, Type>,
    /// Scopes from which currently-resolving spawned closures originate.
    spawn_origins: Vec<ScopeId>,
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
            trait_method_names: std::collections::HashSet::new(),
            variant_payload_types: std::collections::HashMap::new(),
            spawn_origins: Vec::new(),
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
                TopLevel::Enum(e) => {
                    // Register enum name and each variant as constructors
                    self.table.add_binding(
                        self.current_scope,
                        e.name.clone(),
                        false,
                        Some(Type::Custom(e.name.clone())),
                        BindingKind::FunctionName,
                    );
                    for variant in &e.variants {
                        // Remember the payload type so match arms can bind it
                        // at full width instead of assuming Int.
                        if let Some(p) = variant.fields.first() {
                            self.variant_payload_types
                                .insert(variant.name.clone(), p.ty.clone());
                        }
                        // Register EnumName.Variant as a callable
                        let variant_full = format!("{}.{}", e.name, variant.name);
                        self.table.add_binding(
                            self.current_scope,
                            variant_full,
                            false,
                            Some(Type::Custom(e.name.clone())),
                            BindingKind::FunctionName,
                        );
                    }
                }
                TopLevel::Const { name, .. } => {
                    self.table.add_binding(
                        self.current_scope,
                        name.clone(),
                        false,
                        Some(Type::Int), // constants are Int for now
                        BindingKind::Local,
                    );
                }
                TopLevel::TypeAlias { .. } => {
                    // Type aliases are resolved at parse/typecheck level
                }
                TopLevel::Trait(_) => {
                    // Trait definitions are metadata only; no bindings needed
                }
                TopLevel::Impl(impl_block) => {
                    // Register each impl method as a top-level function
                    for method in &impl_block.methods {
                        self.table.add_binding(
                            self.current_scope,
                            method.name.clone(),
                            false,
                            Some(Type::Custom("Function".into())),
                            BindingKind::FunctionName,
                        );
                        // Track the short method name (without StructName_ prefix)
                        // so semantic analysis can allow UFCS calls
                        if let Some(short) = method.name.split('_').last() {
                            self.trait_method_names.insert(short.to_string());
                        }
                    }
                }
                TopLevel::Extern(extern_block) => {
                    // Register each extern function as a builtin-like name
                    for ef in &extern_block.functions {
                        self.table.add_binding(
                            self.current_scope,
                            ef.name.clone(),
                            false,
                            Some(Type::Custom("Function".into())),
                            BindingKind::FunctionName,
                        );
                    }
                }
                TopLevel::Import(import_kind) => {
                    let module = match import_kind {
                        crate::ast::ImportKind::Module { path, .. } => path.last().cloned().unwrap_or_default(),
                        crate::ast::ImportKind::Selective { path, .. } => path.last().cloned().unwrap_or_default(),
                    };
                    self.imports.push(module.clone());
                    if module == "json" {
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
                    }
                }
            }
        }

        // Now walk bodies
        for decl in &mut program.declarations {
            match decl {
                TopLevel::Function(func) => {
                    self.resolve_function(func)?;
                }
                TopLevel::Impl(impl_block) => {
                    for method in &mut impl_block.methods {
                        self.resolve_function(method)?;
                    }
                }
                _ => {}
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
            // The public annotation on a typed rest parameter is its element
            // type, while the binding visible in the body is the owned rest
            // List[T] assembled at the call site.
            let binding_ty = if param.variadic {
                Type::Generic("List".to_string(), vec![param.ty.clone()])
            } else {
                param.ty.clone()
            };
            self.table.add_binding(
                self.current_scope,
                param.name.clone(),
                false,
                Some(binding_ty),
                BindingKind::Param,
            );
        }

        for stmt in &mut func.body {
            self.resolve_stmt(stmt)?;
        }

        self.current_scope = parent;
        Ok(())
    }

    /// Returns whether `candidate` is `scope` itself or one of its lexical
    /// ancestors.
    fn scope_is_at_or_above(&self, candidate: ScopeId, scope: ScopeId) -> bool {
        let mut current = Some(scope);
        while let Some(id) = current {
            if id == candidate {
                return true;
            }
            current = self.table.scopes[id.0].parent;
        }
        false
    }

    /// A spawned closure executes concurrently with the scope that spawned it.
    /// It may read captures, but must not write a binding owned by that scope
    /// (or an enclosing scope). Bindings declared in the closure remain local
    /// to that thread and are intentionally allowed to be mutable.
    fn check_spawn_capture_mutation(&self, binding: BindingId) -> Result<(), String> {
        let binding = &self.table.bindings[binding.0];
        if self.spawn_origins.iter().any(|&origin| {
            self.scope_is_at_or_above(binding.declared_in, origin)
        }) {
            return Err(format!(
                "Cannot mutate captured variable '{}' inside a spawned closure: this would cause a data race.",
                binding.name
            ));
        }
        Ok(())
    }

    fn resolve_stmt(&mut self, stmt: &mut Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Destructure {
                names,
                value,
                binding_ids,
            } => {
                self.resolve_expr(value)?;
                for (index, name) in names.iter().enumerate() {
                    let id = self.table.add_binding(
                        self.current_scope,
                        name.clone(),
                        false,
                        None,
                        BindingKind::Local,
                    );
                    if let Some(cell) = binding_ids.get(index) {
                        cell.set(Some(id.0));
                    }
                }
            }
            Stmt::LetInferred {
                name,
                is_mut,
                value,
                binding_id,
            } => {
                self.resolve_expr(value)?; // Resolve value before shadowing occurs!
                if self.loop_depth > 0 {
                    if let Some(outer_id) = self.table.resolve_name_immutable(self.current_scope, name) {
                        let outer_binding = &self.table.bindings[outer_id.0];
                        if outer_binding.declared_in != self.current_scope
                            && (outer_binding.kind == BindingKind::Local || outer_binding.kind == BindingKind::Param)
                        {
                            eprintln!(
                                "warning: variable '{}' in ':=' shadows an outer variable within a loop; did you mean '{} = ...' to assign to the outer variable?",
                                name, name
                            );
                        }
                    }
                }
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
                    self.check_spawn_capture_mutation(id)?;
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
                let mut curr: &Expr = base;
                let mut root_name = None;
                loop {
                    match curr {
                        Expr::Identifier(name, ..) => {
                            root_name = Some(name.as_str());
                            break;
                        }
                        Expr::FieldAccess { base: sub_base, .. } => {
                            curr = sub_base;
                        }
                        _ => break,
                    }
                }
                if let Some(name) = root_name {
                    if let Some(id) = self.table.resolve_name(self.current_scope, name) {
                        let binding = &self.table.bindings[id.0];
                        if !binding.is_mut {
                            return Err(format!(
                                "Cannot mutate field of immutable variable '{}'. Declare it with 'mut {} := ...' to allow field mutation.",
                                name, name
                            ));
                        }
                        self.check_spawn_capture_mutation(id)?;
                    }
                }
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                then_scope: then_scope_cell,
                else_scope: else_scope_cell,
            } => {
                self.resolve_expr(condition)?;

                let then_scope = self
                    .table
                    .new_scope(Some(self.current_scope), ScopeKind::Block);
                then_scope_cell.set(Some(then_scope.0));
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
                    else_scope_cell.set(Some(else_scope.0));
                    self.current_scope = else_scope;
                    for s in else_b {
                        self.resolve_stmt(s)?;
                    }
                    self.current_scope = old_scope;
                }
            }
            Stmt::While {
                condition,
                body,
                body_scope: body_scope_cell,
            } => {
                self.resolve_expr(condition)?;
                let body_scope = self
                    .table
                    .new_scope(Some(self.current_scope), ScopeKind::Block);
                body_scope_cell.set(Some(body_scope.0));
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
                step: _,
                body,
                binding_id,
                body_scope: body_scope_cell,
            } => {
                self.resolve_expr(start)?;
                self.resolve_expr(end)?;
                let body_scope = self
                    .table
                    .new_scope(Some(self.current_scope), ScopeKind::Block);
                body_scope_cell.set(Some(body_scope.0));
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
                body_scope: body_scope_cell,
            } => {
                self.resolve_expr(list)?;
                let body_scope = self
                    .table
                    .new_scope(Some(self.current_scope), ScopeKind::Block);
                body_scope_cell.set(Some(body_scope.0));
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
            Stmt::Match { subject, arms } => {
                self.resolve_expr(subject)?;
                for arm in arms {
                    // Register match arm bindings at the variant's declared
                    // payload type. Falling back to Int is only correct for
                    // integer payloads; a Float or Str binding needs its own
                    // type or the checker rejects legitimate uses.
                    let payload_ty = self
                        .variant_payload_types
                        .get(&arm.variant)
                        .cloned()
                        .unwrap_or(Type::Int);
                    for binding_name in &arm.bindings {
                        self.table.add_binding(
                            self.current_scope,
                            binding_name.clone(),
                            false,
                            Some(payload_ty.clone()),
                            BindingKind::Local,
                        );
                    }
                    for s in &mut arm.body {
                        self.resolve_stmt(s)?;
                    }
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
        // Check if it's a trait method (UFCS short name)
        if self.trait_method_names.contains(name) {
            return true;
        }
        if let Some(_builtin) = crate::builtins::get_builtins()
            .iter()
            .find(|b| b.name == name)
        {
            return true;
        }
        false
    }

    fn resolve_expr(&mut self, expr: &mut Expr) -> Result<(), String> {
        match expr {
            Expr::IntLiteral(_)
            | Expr::FloatLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::CharLiteral(_)
            | Expr::BoolLiteral(_) => {}
            Expr::Tuple(elements) => {
                for element in elements {
                    self.resolve_expr(element)?;
                }
            }
            Expr::Await(inner) => {
                self.resolve_expr(inner)?;
            }
            Expr::Identifier(name, binding_id_cell) => {
                // Ignore builtins and imported module namespaces for now
                if !self.is_builtin_resolved(name) && !self.imports.contains(name) {
                    if let Some(id) = self.table.resolve_name(self.current_scope, name) {
                        binding_id_cell.set(Some(id.0));
                    } else {
                        return Err(format!("Undeclared identifier '{}'", name));
                    }
                }
            }
            // Turbofish should already be gone: monomorphization runs before
            // name resolution and rewrites it to a call on the specialised
            // name. Surfacing it as an error beats resolving the template name
            // and silently calling the wrong (unspecialised) function.
            Expr::GenericCall { callee, .. } => {
                let name = match &**callee {
                    Expr::Identifier(n, _) => n.clone(),
                    _ => "<expr>".to_string(),
                };
                return Err(format!(
                    "'{}' does not take type arguments, or monomorphization could not resolve them",
                    name
                ));
            }
            Expr::UnaryOp { operand, .. } => {
                self.resolve_expr(operand)?;
            }
            Expr::BinaryOp { left, right, .. } => {
                self.resolve_expr(left)?;
                self.resolve_expr(right)?;
            }
            Expr::Call { callee, args } => {
                // Check if calling an imported module's function (e.g. gui.create) or UFCS method call (e.g. p.speak)
                let mut rewritten = None;
                let mut prepend_base = false;

                if let Expr::FieldAccess { base, field } = &**callee {
                    let mut is_module_call = false;
                    if let Expr::Identifier(module_name, _) = &**base {
                        if self.imports.contains(module_name) {
                            is_module_call = true;
                            let mangled = format!("{}_{}", module_name, field);
                            if self.table.resolve_name_immutable(self.current_scope, &mangled).is_some()
                                || self.is_builtin_resolved(&mangled)
                            {
                                rewritten = Some(Expr::Identifier(mangled, std::cell::Cell::new(None)));
                            } else if self.table.resolve_name_immutable(self.current_scope, field).is_some()
                                || self.is_builtin_resolved(field)
                            {
                                rewritten = Some(Expr::Identifier(field.clone(), std::cell::Cell::new(None)));
                            } else {
                                rewritten = Some(Expr::Identifier(mangled, std::cell::Cell::new(None)));
                            }
                        } else {
                            // Loophole Fix: Check if user wrote unimported module syntax like gui.key_down() or net.listen()
                            let mangled = format!("{}_{}", module_name, field);
                            if self.is_builtin_resolved(&mangled) || matches!(module_name.as_str(), "gui" | "math" | "net" | "json" | "http" | "io" | "physics" | "ecs" | "ui" | "lreact") {
                                return Err(format!(
                                    "Import Error: Module '{}' is used but not imported. Add 'import {}' at the top of your file.",
                                    module_name, module_name
                                ));
                            }
                        }
                    }

                    if !is_module_call {
                        rewritten = Some(Expr::Identifier(field.clone(), std::cell::Cell::new(None)));
                        prepend_base = true;
                    }
                }

                if let Some(new_callee) = rewritten {
                    if prepend_base {
                        if let Expr::FieldAccess { base, .. } = &**callee {
                            args.insert(0, *base.clone());
                        }
                    }
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
                self.spawn_origins.push(self.current_scope);
                let result = self.resolve_expr(closure);
                self.spawn_origins.pop();
                result?;
            }
            Expr::ListLiteral(elements) => {
                for element in elements {
                    self.resolve_expr(element)?;
                }
            }
            Expr::Match { subject, arms } => {
                self.resolve_expr(subject)?;
                for arm in arms {
                    for s in &mut arm.body {
                        self.resolve_stmt(s)?;
                    }
                }
            }
            Expr::EnumVariantConstruct { args, .. } => {
                for arg in args {
                    self.resolve_expr(arg)?;
                }
            }
            Expr::Try(inner) => {
                self.resolve_expr(inner)?;
            }
            Expr::Index { base, index } => {
                self.resolve_expr(base)?;
                self.resolve_expr(index)?;
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
                type_params: vec![], name: "main".to_string(),
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
                is_async: false,
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
                type_params: vec![], name: "main".to_string(),
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
                is_async: false,
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
                type_params: vec![], name: "main".to_string(),
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
                is_async: false,
            })],
        };

        let mut resolver = Resolver::new();
        let err = resolver
            .resolve_program(&mut program)
            .expect_err("should reject immutable field mutation");
        assert!(err.contains("Cannot mutate field of immutable variable 'box'"));
    }

    #[test]
    fn rejects_nested_field_mutation_on_immutable_variable() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                type_params: vec![], name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![
                    Stmt::LetInferred {
                        name: "player".to_string(),
                        is_mut: false,
                        value: Expr::IntLiteral(1),
                        binding_id: std::cell::Cell::new(None),
                    },
                    Stmt::AssignField {
                        base: Expr::FieldAccess {
                            base: Box::new(Expr::Identifier("player".to_string(), std::cell::Cell::new(None))),
                            field: "pos".to_string(),
                        },
                        field: "x".to_string(),
                        value: Expr::IntLiteral(10),
                    },
                ],
                is_async: false,
            })],
        };

        let mut resolver = Resolver::new();
        let err = resolver
            .resolve_program(&mut program)
            .expect_err("should reject nested field mutation on immutable root");
        assert!(err.contains("Cannot mutate field of immutable variable 'player'"));
    }

    #[test]
    fn rejects_break_outside_loop() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                type_params: vec![], name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![Stmt::Break],
                is_async: false,
            })],
        };

        let mut resolver = Resolver::new();
        let err = resolver
            .resolve_program(&mut program)
            .expect_err("should reject break outside loop");
        assert!(err.contains("outside of a loop"));
    }

    fn spawned_closure(body: Vec<Stmt>) -> Stmt {
        Stmt::Expr(Expr::Spawn {
            closure: Box::new(Expr::Closure {
                params: vec![],
                return_type: None,
                body,
            }),
        })
    }

    #[test]
    fn rejects_assignment_to_capture_in_spawned_closure() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                type_params: vec![],
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![
                    Stmt::LetInferred {
                        name: "x".to_string(),
                        is_mut: true,
                        value: Expr::IntLiteral(1),
                        binding_id: std::cell::Cell::new(None),
                    },
                    spawned_closure(vec![Stmt::Assign {
                        name: "x".to_string(),
                        value: Expr::IntLiteral(2),
                        binding_id: std::cell::Cell::new(None),
                    }]),
                ],
                is_async: false,
            })],
        };

        let err = Resolver::new().resolve_program(&mut program).unwrap_err();
        assert!(err.contains("Cannot mutate captured variable 'x'"));
        assert!(err.contains("data race"));
    }

    #[test]
    fn rejects_field_assignment_to_capture_in_spawned_closure() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                type_params: vec![],
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![
                    Stmt::LetInferred {
                        name: "box".to_string(),
                        is_mut: true,
                        value: Expr::IntLiteral(1),
                        binding_id: std::cell::Cell::new(None),
                    },
                    spawned_closure(vec![Stmt::AssignField {
                        base: Expr::Identifier("box".to_string(), std::cell::Cell::new(None)),
                        field: "value".to_string(),
                        value: Expr::IntLiteral(2),
                    }]),
                ],
                is_async: false,
            })],
        };

        let err = Resolver::new().resolve_program(&mut program).unwrap_err();
        assert!(err.contains("Cannot mutate captured variable 'box'"));
    }

    #[test]
    fn allows_reading_capture_in_spawned_closure() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                type_params: vec![],
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![
                    Stmt::LetInferred {
                        name: "x".to_string(),
                        is_mut: true,
                        value: Expr::IntLiteral(1),
                        binding_id: std::cell::Cell::new(None),
                    },
                    spawned_closure(vec![Stmt::Expr(Expr::Identifier(
                        "x".to_string(),
                        std::cell::Cell::new(None),
                    ))]),
                ],
                is_async: false,
            })],
        };

        Resolver::new()
            .resolve_program(&mut program)
            .expect("reads of a spawned capture are safe");
    }

    #[test]
    fn allows_mutating_local_in_spawned_closure() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                type_params: vec![],
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![spawned_closure(vec![
                    Stmt::LetInferred {
                        name: "local".to_string(),
                        is_mut: true,
                        value: Expr::IntLiteral(1),
                        binding_id: std::cell::Cell::new(None),
                    },
                    Stmt::Assign {
                        name: "local".to_string(),
                        value: Expr::IntLiteral(2),
                        binding_id: std::cell::Cell::new(None),
                    },
                ])],
                is_async: false,
            })],
        };

        Resolver::new()
            .resolve_program(&mut program)
            .expect("locals declared by a spawned closure may be mutable");
    }

    #[test]
    fn resolves_loop_shadowing_syntax() {
        let mut program = Program {
            declarations: vec![TopLevel::Function(Function {
                type_params: vec![],
                name: "main".to_string(),
                params: vec![],
                return_type: Type::Void,
                body: vec![
                    Stmt::LetInferred {
                        name: "i".to_string(),
                        is_mut: true,
                        value: Expr::IntLiteral(0),
                        binding_id: std::cell::Cell::new(None),
                    },
                    Stmt::While {
                        condition: Expr::BoolLiteral(true),
                        body: vec![
                            Stmt::LetInferred {
                                name: "i".to_string(),
                                is_mut: false,
                                value: Expr::IntLiteral(1),
                                binding_id: std::cell::Cell::new(None),
                            },
                        ],
                        body_scope: std::cell::Cell::new(None),
                    },
                ],
                is_async: false,
            })],
        };

        Resolver::new()
            .resolve_program(&mut program)
            .expect("loop shadowing resolves successfully");
    }
}
